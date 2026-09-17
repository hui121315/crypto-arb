//! 执行 run feed 装配：WS 订阅 + REST seed + 兜底轮询，收敛成 workstation-owned
//! [`ExecutionRunFeed`]。上下文解析见 `run/context.rs`，seed/stream 应用见 `run/seed.rs`。

#[path = "run/context.rs"]
mod context;
#[path = "run/seed.rs"]
mod seed;

use crate::api::ws::{start_execution_stream_with_state, WsChannelState};
use crate::state::context::use_global;
use crate::state::polling::{use_ws_channel_context_snapshot_fallback, SnapshotFallbackTiming};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ApiProblem, ExecutionRun, HedgeTicketView, ListStatus};
use std::time::Duration;

use super::super::selection::ExecutionSelection;
use super::workflow::{apply_run_candidate, WorkflowViewFeed, REST_RUN_SOURCE, WS_RUN_SOURCE};
use context::ExecutionRunContext;
use seed::{apply_run_update, apply_seed_result};

pub(super) use context::{
    clear_execution_run_context, restored_execution_run_evidence, restored_execution_run_matches,
    store_confirm_request_context, store_execution_run_context, store_workspace_route_context,
};

pub(super) const EXECUTION_CHANNEL: &str = "execution";
const EXECUTION_FALLBACK_TIMING: SnapshotFallbackTiming = SnapshotFallbackTiming {
    period: Duration::from_secs(5),
    grace: Duration::from_secs(8),
    stale_after: Duration::from_secs(10),
};
const EXECUTION_RUN_EVENT: &str = "execution_run_updated";
const EXECUTION_RUN_CLOSED_EVENT: &str = "execution_run_closed";
const PRIVATE_WS_FILL_EVENT: &str = "private_ws_fill_event";

#[derive(Clone, Copy)]
pub(crate) struct ExecutionRunFeed {
    pub run: RwSignal<Option<ExecutionRun>>,
    pub seed_problem: RwSignal<Option<ApiProblem>>,
    pub stream_problem: RwSignal<Option<ApiProblem>>,
    pub channel_state: RwSignal<WsChannelState>,
}

pub(crate) fn use_execution_run_updates(
    selection: Memo<ExecutionSelection>,
    feed: ExecutionRunFeed,
    workflow: WorkflowViewFeed,
    refresh_nonce: RwSignal<u64>,
) -> ExecutionRunFeed {
    let ExecutionRunFeed {
        run,
        seed_problem,
        stream_problem,
        channel_state,
    } = feed;
    let request_version = RwSignal::new(0_u64);
    let client = use_global().client;
    Effect::new(move |_| {
        refresh_nonce.get();
        let context = ExecutionRunContext::from_selection(&selection.get());
        let token = next_request_token(request_version);
        if !context.has_filter() {
            seed_problem.set(None);
            run.set(None);
            workflow.clear();
            return;
        }
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .execution_runs_for_context(
                    context.opportunity_id.as_deref(),
                    context.ticket_id.as_deref(),
                    context.run_id.as_deref(),
                )
                .await
                .map_err(|error| error.problem);
            if !request_token_is_latest(request_version.try_get_untracked(), token) {
                return;
            }
            if clear_missing_local_restore(run, seed_problem, workflow, &context, &result) {
                return;
            }
            if let Some(candidate) = apply_seed_result(run, seed_problem, &context, result) {
                apply_run_candidate(workflow, candidate, REST_RUN_SOURCE);
            }
        });
    });
    let handle = start_execution_stream_with_state(
        channel_state,
        move |event| {
            if is_execution_run_update_event(&event.event) {
                if let Some(next) = event.execution_run.filter(|next| {
                    explicit_stream_context(ExecutionRunContext::from_selection(
                        &selection.get_untracked(),
                    ))
                    .is_some_and(|context| context.matches(next))
                }) {
                    let candidate = HedgeTicketView::from_execution_run(&next);
                    stream_problem.set(None);
                    if apply_run_update(run, next) {
                        apply_run_candidate(workflow, candidate, WS_RUN_SOURCE);
                    }
                }
            }
        },
        move |problem| stream_problem.set(Some(problem)),
    );
    on_cleanup(move || handle.cancel());
    let fallback_client = use_global().client;
    use_ws_channel_context_snapshot_fallback(
        channel_state,
        EXECUTION_FALLBACK_TIMING,
        move || !selection.get_untracked().opportunity_id.trim().is_empty(),
        move || {
            let client = fallback_client.clone();
            let context = ExecutionRunContext::from_selection(&selection.get_untracked());
            async move {
                let result = client
                    .execution_runs_for_context(
                        context.opportunity_id.as_deref(),
                        context.ticket_id.as_deref(),
                        context.run_id.as_deref(),
                    )
                    .await
                    .map_err(|error| error.problem);
                (context, result)
            }
        },
        move |requested_context, result| {
            let current_context = ExecutionRunContext::from_selection(&selection.get_untracked());
            if requested_context == current_context
                && clear_missing_local_restore(
                    run,
                    seed_problem,
                    workflow,
                    &current_context,
                    &result,
                )
            {
                return;
            }
            if let Some(candidate) = apply_fallback_result(
                run,
                seed_problem,
                &requested_context,
                &current_context,
                result,
            ) {
                apply_run_candidate(workflow, candidate, REST_RUN_SOURCE);
            }
        },
    );
    ExecutionRunFeed {
        run,
        seed_problem,
        stream_problem,
        channel_state,
    }
}

fn is_execution_run_update_event(event: &str) -> bool {
    matches!(
        event,
        EXECUTION_RUN_EVENT | EXECUTION_RUN_CLOSED_EVENT | PRIVATE_WS_FILL_EVENT
    )
}

fn explicit_stream_context(context: ExecutionRunContext) -> Option<ExecutionRunContext> {
    context.has_filter().then_some(context)
}

fn next_request_token(version: RwSignal<u64>) -> u64 {
    let token = version.get_untracked().wrapping_add(1);
    version.set(token);
    token
}

fn request_token_is_latest(version: Option<u64>, token: u64) -> bool {
    version == Some(token)
}

fn clear_missing_local_restore(
    run: RwSignal<Option<ExecutionRun>>,
    seed_problem: RwSignal<Option<ApiProblem>>,
    workflow: WorkflowViewFeed,
    context: &ExecutionRunContext,
    result: &Result<shared_types::ListEnvelope<ExecutionRun>, ApiProblem>,
) -> bool {
    if !missing_local_restore_confirmed(context, result) {
        return false;
    }
    clear_execution_run_context();
    workflow.clear();
    run.set(None);
    seed_problem.set(None);
    true
}

fn missing_local_restore_confirmed(
    context: &ExecutionRunContext,
    result: &Result<shared_types::ListEnvelope<ExecutionRun>, ApiProblem>,
) -> bool {
    if !context.is_local_persisted_restore() {
        return false;
    }
    let Ok(envelope) = result else {
        return false;
    };
    envelope.status == ListStatus::Fresh
        && envelope.problems.is_empty()
        && !envelope.rows.iter().any(|run| context.matches(run))
}

fn apply_fallback_result(
    run: RwSignal<Option<ExecutionRun>>,
    seed_problem: RwSignal<Option<ApiProblem>>,
    requested_context: &ExecutionRunContext,
    current_context: &ExecutionRunContext,
    result: Result<shared_types::ListEnvelope<ExecutionRun>, ApiProblem>,
) -> Option<HedgeTicketView> {
    if requested_context != current_context {
        return None;
    }
    apply_seed_result(run, seed_problem, current_context, result)
}

#[cfg(test)]
#[path = "run/tests.rs"]
mod tests;
