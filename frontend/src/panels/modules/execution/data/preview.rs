use crate::state::{context::use_global, load_state::LoadState, polling::use_debounced_value};
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::time::Duration;

#[path = "preview/build.rs"]
mod build;
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
use model::PreviewQuery;
use runtime::{
    active_backoff_problem, active_preview_query, load_preview, now_ms, preview_backoff_until_ms,
    preview_request_is_current, preview_request_is_in_flight, preview_request_state,
    refresh_stale_selection_snapshot, restore_last_ready, set_preview_problem_state,
    PreviewBackoff, ReadyPreview,
};

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

pub(crate) fn preview_memo(
    state: RwSignal<LoadState<ExecutionPreview>>,
    signals: PreviewSignals,
) -> Memo<ExecutionPreview> {
    Memo::new(move |_| match state.get() {
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
) -> RwSignal<LoadState<ExecutionPreview>> {
    let client = use_global().client;
    let last_ready = RwSignal::new(None::<ReadyPreview>);
    let request_version = RwSignal::new(0_u64);
    let in_flight = RwSignal::new(None::<PreviewQuery>);
    let backoff = RwSignal::new(None::<PreviewBackoff>);
    let debounced_query = use_debounced_value(move || preview_query(signals), PREVIEW_DEBOUNCE);

    Effect::new(move |_| {
        refresh_nonce.get();
        let selected_opportunity_id = signals
            .selection
            .with_untracked(|selection| selection.opportunity_id.clone());
        let query = active_preview_query(debounced_query.get(), &selected_opportunity_id);
        let Some(query) = query else {
            request_version.update(|value| *value = value.wrapping_add(1));
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
        let version = request_version.get_untracked();
        in_flight.set(Some(query.clone()));
        state.set(preview_request_state(&last_ready.get_untracked(), &query));
        let client = client.clone();
        spawn_local(async move {
            let result = load_preview(client, &query).await;
            let Some(current_version) = request_version.try_get_untracked() else {
                return;
            };
            if !preview_request_is_current(current_version, version) {
                return;
            }
            in_flight.set(None);
            match result {
                Ok(loaded) => {
                    backoff.set(None);
                    apply_preview(workflow, loaded.workflow_view);
                    let preview = loaded.preview;
                    last_ready.set(Some(ReadyPreview::new(&query, preview.clone())));
                    state.set(LoadState::Ready(preview));
                }
                Err(problem) => {
                    if refresh_stale_selection_snapshot(signals.selection_state, &query, &problem) {
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

    state
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
