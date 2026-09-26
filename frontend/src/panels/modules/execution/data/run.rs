//! 执行 run feed 装配：WS 订阅 + REST seed + 兜底轮询，收敛成 workstation-owned
//! [`ExecutionRunFeed`]。上下文解析见 `run/context.rs`，seed/stream 应用见 `run/seed.rs`。

#[path = "run/context.rs"]
mod context;
#[path = "run/seed.rs"]
mod seed;

use crate::api::ws::{start_execution_stream_with_state, WsChannelState};
use crate::state::polling::{use_ws_channel_context_snapshot_fallback, SnapshotFallbackTiming};
use crate::state::read_scope::bounded_read;
use futures::future::{AbortHandle, Abortable};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ApiProblem, ExecutionRun, HedgeTicketView};
use std::time::Duration;

use super::super::selection::ExecutionSelection;
use super::submission::SubmissionRecovery;
use super::workflow::{apply_run_candidate, WorkflowViewFeed, REST_RUN_SOURCE, WS_RUN_SOURCE};
use context::ExecutionRunContext;
pub(super) use seed::apply_run_update;
use seed::apply_seed_result;

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
    recovery: SubmissionRecovery,
    confirm_state: RwSignal<shared_types::ActionState>,
) -> ExecutionRunFeed {
    let ExecutionRunFeed {
        run,
        seed_problem,
        stream_problem,
        channel_state,
    } = feed;
    let request_version = RwSignal::new(0_u64);
    let reading = RwSignal::new(false);
    let read_again = RwSignal::new(false);
    let retry_at = RwSignal::new(0_u64);
    let active_read = StoredValue::new(None::<AbortHandle>);
    on_cleanup(move || active_read.update_value(|active| {
        if let Some(abort) = active.take() { abort.abort(); }
    }));
    let connection = recovery.connection;
    let current_client = connection.client();
    Effect::new(move |previous: Option<gloo_timers::callback::Interval>| {
        if let Some(previous) = previous {
            return previous;
        }
        let current_client = current_client.clone();
        gloo_timers::callback::Interval::new(5_000, move || {
            if reading.try_get_untracked() == Some(false)
                && recovery.pending.get_untracked().is_some()
                && !recovery.sending.get_untracked()
                && crate::state::polling::now_ms() >= retry_at.get_untracked()
                && recovery.matches_backend(&current_client.base_url())
            {
                refresh_nonce.update(|value| *value = value.wrapping_add(1));
            }
        })
    });
    let client = connection.client();
    Effect::new(move |_| {
        refresh_nonce.get();
        if !connection.available() { return; }
        if recovery.sending.get() { return; }
        let context =
            ExecutionRunContext::with_pending(connection, &selection.get(), recovery.pending.get().as_ref());
        let pending = recovery.pending.get_untracked();
        let token = next_request_token(request_version);
        if reading.get_untracked() {
            read_again.set(true);
            return;
        }
        if !context.has_filter() {
            seed_problem.set(None);
            run.set(None);
            workflow.clear();
            return;
        }
        if !recovery.matches_backend(&client.base_url()) {
            return;
        }
        reading.set(true);
        let client = client.clone();
        let (abort, registration) = AbortHandle::new_pair();
        active_read.set_value(Some(abort));
        spawn_local(async move {
            let Ok(result) = Abortable::new(
                read_execution_snapshot(client.clone(), &context, pending.as_ref()),
                registration,
            ).await else { return; };
            if !connection.current() { return; }
            let (result, rejection) = match result {
                Ok((envelope, rejection)) => (Ok(envelope), rejection),
                Err(problem) => (Err(problem), None),
            };
            if reading.try_get_untracked().is_none() {
                return;
            }
            active_read.update_value(|active| { active.take(); });
            reading.set(false);
            retry_at.set(
                crate::state::polling::now_ms()
                    + result
                        .as_ref()
                        .err()
                        .map(|problem| problem.retry_after_ms.unwrap_or(5_000).max(5_000))
                        .unwrap_or(0),
            );
            if read_again.get_untracked() {
                read_again.set(false);
                refresh_nonce.update(|value| *value = value.wrapping_add(1));
                return;
            }
            if !request_token_is_latest(request_version.try_get_untracked(), token) {
                return;
            }
            let Some(current_selection) = selection.try_get_untracked() else {
                return;
            };
            let current_context = ExecutionRunContext::with_pending(connection,
                &current_selection,
                recovery.pending.get_untracked().as_ref(),
            );
            if context != current_context || !recovery.matches_backend(&client.base_url()) {
                return;
            }
            if let (Some(problem), Some(pending)) = (rejection, pending.as_ref()) {
                clear_execution_run_context(connection);
                recovery.resolve(&pending.idempotency_key);
                confirm_state.set(shared_types::ActionState::failed(
                    "已核实：下单前被拒绝",
                    problem,
                ));
                seed_problem.set(None);
                return;
            }
            // A lost/in-flight write can precede its run entry. Keep the recovery
            // lock without reporting an ordinary journal wait as a read failure.
            if pending.is_some() && result.as_ref().is_ok_and(|envelope| {
                envelope.status == shared_types::ListStatus::Fresh
                    && envelope.problems.is_empty()
                    && !envelope.rows.iter().any(|run| context.matches(run))
            }) {
                seed_problem.set(None);
                return;
            }
            if let Some(candidate) = apply_seed_result(run, seed_problem, &context, result) {
                apply_run_candidate(workflow, candidate, REST_RUN_SOURCE);
            }
        });
    });
    let stream_client = connection.client();
    let handle = start_execution_stream_with_state(
        channel_state,
        move |event| {
            if !recovery.matches_backend(&stream_client.base_url()) {
                return;
            }
            if is_execution_run_update_event(&event.event) {
                if let Some(next) = event.execution_run.filter(|next| {
                    explicit_stream_context(ExecutionRunContext::with_pending(connection,
                        &selection.get_untracked(),
                        recovery.pending.get_untracked().as_ref(),
                    ))
                    .is_some_and(|context| context.matches(next))
                }) {
                    let candidate = HedgeTicketView::from_execution_run(&next);
                    stream_problem.set(None);
                    if apply_run_update(run, next, true) {
                        apply_run_candidate(workflow, candidate, WS_RUN_SOURCE);
                    }
                }
            }
        },
        move |problem| { if connection.current() { stream_problem.set(Some(problem)); } },
    );
    on_cleanup(move || handle.cancel());
    let fallback_client = connection.client();
    let fallback_guard_client = fallback_client.clone();
    let fallback_result_client = fallback_client.clone();
    use_ws_channel_context_snapshot_fallback(
        channel_state,
        EXECUTION_FALLBACK_TIMING,
        move || {
            !reading.get_untracked()
                && recovery.pending.get_untracked().is_none()
                && recovery.matches_backend(&fallback_guard_client.base_url())
                && ExecutionRunContext::with_pending(connection, &selection.get_untracked(), None).has_filter()
        },
        move || {
            let client = fallback_client.clone();
            let context = ExecutionRunContext::with_pending(connection,
                &selection.get_untracked(),
                recovery.pending.get_untracked().as_ref(),
            );
            let token = request_version.get_untracked();
            reading.set(true);
            async move {
                let result = read_execution_snapshot(client, &context, None)
                    .await.map(|(envelope, _)| envelope);
                ((context, token), result)
            }
        },
        move |(requested_context, token), result| {
            reading.set(false);
            if !recovery.matches_backend(&fallback_result_client.base_url()) {
                return;
            }
            if read_again.get_untracked() {
                read_again.set(false);
                refresh_nonce.update(|value| *value = value.wrapping_add(1));
                return;
            }
            if !request_token_is_latest(request_version.try_get_untracked(), token) {
                return;
            }
            let current_context = ExecutionRunContext::with_pending(connection,
                &selection.get_untracked(),
                recovery.pending.get_untracked().as_ref(),
            );
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

async fn read_execution_snapshot(
    client: crate::api::rest::ApiClient,
    context: &ExecutionRunContext,
    pending: Option<&shared_types::HedgeConfirmContext>,
) -> Result<(shared_types::ListEnvelope<ExecutionRun>, Option<ApiProblem>), ApiProblem> {
    let client = client.cancelable_reads();
    // The run lookup and optional rejection lookup share one deadline, not two.
    bounded_read(async {
        let envelope = client.execution_runs_for_context(
            context.opportunity_id.as_deref(),
            context.ticket_id.as_deref(),
            context.run_id.as_deref(),
        ).await?;
        let rejection = if let Some(pending) = pending.filter(|_| {
            envelope.status == shared_types::ListStatus::Fresh
                && envelope.problems.is_empty()
                && !envelope.rows.iter().any(|run| context.matches(run))
        }) {
            let rows = client.action_runs().await?;
            confirmed_pre_order_rejection(&rows, pending)
        } else { None };
        Ok((envelope, rejection))
    }).await.map_err(|problem| {
        if problem.code == "SHARED_READ_TIMEOUT" {
            ApiProblem::new("EXECUTION_RUN_READ_TIMEOUT",
                "核对原执行超过 15 秒，已停止本次查询；原提交仍待核对，不会重复下单")
                .with_source("execution.run_recovery")
                .with_retry_after_ms(Some(5_000))
        } else { problem }
    })
}

fn confirmed_pre_order_rejection(
    rows: &[shared_types::ActionRun],
    pending: &shared_types::HedgeConfirmContext,
) -> Option<ApiProblem> {
    rows.iter()
        .filter(|row| {
            row.kind == shared_types::ActionRunKind::HedgeConfirm
                && row.idempotency_key.as_deref() == Some(pending.idempotency_key.as_str())
                && row.target.as_deref() == Some(pending.opportunity_id.as_str())
        })
        .max_by_key(|row| row.updated_at_ms)
        .filter(|row| row.status == shared_types::ActionRunStatus::Failed)
        .and_then(|row| row.problem.clone())
        .filter(super::actions::outcome::confirm_rejected_before_order)
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
