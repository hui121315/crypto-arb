use crate::state::{load_state::LoadState, polling::use_debounced_value};
use super::connection::ExecutionConnection;
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::time::Duration;

#[path = "preview/build.rs"]
mod build;
#[path = "preview/clock.rs"]
mod clock;
pub(in crate::panels::modules::execution) use clock::{TicketClock, TicketClockHistory};
#[path = "preview/context.rs"]
mod context;
#[path = "preview/format.rs"]
mod format;
#[path = "preview/model.rs"]
mod model;
#[path = "preview/response.rs"]
mod response;
#[path = "preview/runtime.rs"]
mod runtime;

use super::workflow::{apply_preview, WorkflowViewFeed};
use build::{failed_for_inputs, pending_for_inputs, preview_query, stale_preview};
use context::{use_preview_context, PreviewContext};
use model::PreviewQuery;
use runtime::{
    active_backoff_problem, active_preview_query, load_preview, now_ms, preview_backoff_until_ms,
    preview_request_is_current, preview_request_is_in_flight, preview_request_state,
    restore_last_ready, set_preview_problem_state, PreviewBackoff, ReadyPreview, SnapshotRefreshBudget,
};

#[cfg(test)]
use runtime::refresh_stale_selection_snapshot;

#[cfg(test)]
use super::super::selection::ExecutionSelection;
#[cfg(test)]
use model::PreviewInput;
#[cfg(test)]
use response::{from_api_preview, preview_request};
#[cfg(test)]
use runtime::preview_problem_state;
#[cfg(test)]
use shared_types::{problem::codes, ApiProblem};

pub(crate) use build::{
    default_capital_text, default_leverage_text, default_limit_offset_text,
    quantity_from_notional_text,
};
pub(in crate::panels::modules::execution) use model::{
    ExecutionPreview, PreviewDepth, PreviewFundingWindowEvidence, PreviewOneCycleCost,
    PreviewReadiness, PreviewSignals,
};
#[cfg(test)]
pub(in crate::panels::modules::execution) use model::{
    PreviewLiquidation, PreviewProfitEvidence, PreviewRisk, PreviewSeed,
};

#[cfg(test)]
use build::failed_preview;
#[cfg(test)]
use build::pending_preview;
#[cfg(test)]
use format::{fee_evidence_lines, parse_number};
#[cfg(test)]
use shared_types::{
    ExecutionMode, FeeProduct, HedgeExecutionParams, MarginMode, MarketDataHealth, OrderType,
    TimeInForce, TradeFeeSnapshot, TradeFeeSource,
};

const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(250);

fn preview_memo(
    state: RwSignal<LoadState<ExecutionPreview>>,
    signals: PreviewSignals,
    inputs_match: Memo<bool>,
) -> Memo<ExecutionPreview> {
    Memo::new(move |_| match state.get() {
        LoadState::Ready(_) if !inputs_match.get() => pending_for_inputs(signals),
        LoadState::Ready(preview) => preview,
        LoadState::Stale { value, problem } => stale_preview(value, &problem),
        LoadState::Error(problem) => failed_for_inputs(signals, &problem),
        LoadState::Loading => pending_for_inputs(signals),
    })
}

pub(crate) fn use_preview(
    signals: PreviewSignals,
    workflow: WorkflowViewFeed,
    refresh_nonce: RwSignal<u64>,
    state: RwSignal<LoadState<ExecutionPreview>>,
    clocks: StoredValue<TicketClockHistory>,
) -> (
    RwSignal<LoadState<ExecutionPreview>>,
    Memo<ExecutionPreview>,
) {
    let connection = expect_context::<ExecutionConnection>();
    let client = connection.client();
    let last_ready = RwSignal::new(None::<ReadyPreview>);
    let request_version = RwSignal::new(0_u64);
    let in_flight = RwSignal::new(None::<PreviewQuery>);
    let backoff = RwSignal::new(None::<PreviewBackoff>);
    let snapshot_retries = RwSignal::new(SnapshotRefreshBudget::default());
    let context = use_preview_context();
    let observed_context = StoredValue::new(None::<PreviewContext>);
    let current_query = Memo::new(move |_| preview_query(signals));
    let input_problem = Memo::new(move |_| build::input_problem(signals));
    let ready_query = RwSignal::new(None::<(PreviewQuery, u64, PreviewContext)>);
    let inputs_match = Memo::new(move |_| {
        connection.available()
            && context.get().problem().is_none()
            && input_problem.get().is_none()
            && ready_query.get().as_ref()
                == Some(&(current_query.get(), refresh_nonce.get(), context.get()))
    });
    let preview = preview_memo(state, signals, inputs_match);
    let debounced_query = use_debounced_value(move || current_query.get(), PREVIEW_DEBOUNCE);

    Effect::new(move |_| {
        refresh_nonce.get();
        if !connection.available() { return; }
        let request_context = context.get();
        if observed_context.get_value().as_ref() != Some(&request_context) {
            observed_context.set_value(Some(request_context.clone()));
            request_version.update(|value| *value = value.wrapping_add(1));
            ready_query.set(None);
            in_flight.set(None);
            last_ready.set(None);
            backoff.set(None);
            snapshot_retries.set(SnapshotRefreshBudget::default());
            state.set(LoadState::Loading);
        }
        if let Some(problem) = request_context.problem() {
            state.set(LoadState::Error(problem));
            return;
        }
        let current = current_query.get();
        if let Some(problem) = input_problem.get() {
            request_version.update(|value| *value = value.wrapping_add(1));
            ready_query.set(None);
            in_flight.set(None);
            state.set(LoadState::Error(problem));
            return;
        }
        if ready_query.get_untracked().as_ref()
            == Some(&(current.clone(), refresh_nonce.get_untracked(), request_context.clone()))
        {
            return;
        }
        let selected_opportunity_id = signals
            .selection
            .with_untracked(|selection| selection.opportunity_id.clone());
        let query = active_preview_query(debounced_query.get(), &selected_opportunity_id)
            .filter(|query| query == &current);
        let Some(query) = query else {
            request_version.update(|value| *value = value.wrapping_add(1));
            ready_query.set(None);
            in_flight.set(None);
            state.set(LoadState::Ready(pending_for_inputs(signals)));
            return;
        };
        restore_last_ready(last_ready, state, &query);
        if let Some(problem) = active_backoff_problem(&backoff.get_untracked(), &query, now_ms()) {
            set_preview_problem_state(state, last_ready, &query, problem);
            return;
        }
        if preview_request_is_in_flight(&in_flight.get_untracked(), &query) {
            return;
        }
        request_version.update(|value| *value = value.wrapping_add(1));
        ready_query.set(None);
        let version = request_version.get_untracked();
        in_flight.set(Some(query.clone()));
        state.set(preview_request_state(&last_ready.get_untracked(), &query));
        let client = client.clone();
        spawn_local(async move {
            let result = load_preview(client, &query).await;
            let Some(current_version) = request_version.try_get_untracked() else {
                return;
            };
            if !connection.current() || !preview_request_is_current(current_version, version)
                || context.get_untracked() != request_context
                || current_query.get_untracked() != query
                || input_problem.get_untracked().is_some()
            {
                return;
            }
            in_flight.set(None);
            // Repeated refreshes of these same inputs share the pending response.
            let refresh = refresh_nonce.get_untracked();
            let result = result.and_then(|mut loaded| {
                request_context.validate(&loaded.preview)?;
                if let (Some(ticket_id), Some(clock)) = (
                    loaded.preview.ticket_id.as_deref(), loaded.preview.clock.take(),
                ) {
                    let mut retained = Ok(clock.clone());
                    clocks.update_value(|history| retained = history.retain(ticket_id, clock));
                    loaded.preview.clock = Some(retained?);
                }
                Ok(loaded)
            });
            match result {
                Ok(loaded) => {
                    backoff.set(None);
                    apply_preview(workflow, loaded.workflow_view);
                    let preview = loaded.preview;
                    let mut query = query;
                    if query.seed.opportunity_snapshot_id != preview.opportunity_snapshot_id {
                        query.seed.opportunity_snapshot_id
                            .clone_from(&preview.opportunity_snapshot_id);
                        signals.selection_state.update(|selection| {
                            selection.opportunity_snapshot_id
                                .clone_from(&preview.opportunity_snapshot_id);
                        });
                    }
                    last_ready.set(Some(ReadyPreview::new(&query, preview.clone())));
                    ready_query.set(Some((query, refresh, request_context)));
                    state.set(LoadState::Ready(preview));
                }
                Err(mut problem) => {
                    let mut retry = false;
                    snapshot_retries.update(|budget| {
                        retry = budget.refresh(
                            signals.selection_state,
                            &query,
                            refresh,
                            &mut problem,
                        );
                    });
                    if retry {
                        state.set(preview_request_state(&last_ready.get_untracked(), &query));
                        return;
                    }
                    if let Some(until_ms) = preview_backoff_until_ms(&problem, now_ms()) {
                        backoff.set(Some(PreviewBackoff::new(&query, problem.clone(), until_ms)));
                    }
                    set_preview_problem_state(state, last_ready, &query, problem);
                }
            }
        });
    });

    (state, preview)
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
