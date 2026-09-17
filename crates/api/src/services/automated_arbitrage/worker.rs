mod candidate_preview;
mod decisions;

use super::guards::{active_runs, cooldown_blocker, entry_blocker, entry_safety_blocker};
use crate::services::action_runs::ActionRunStart;
use crate::state::AppState;
use automation::select_candidates;
use candidate_preview::{
    build_candidate_preview, candidate_allowed, PreviewFailure, MAX_PREVIEW_CANDIDATES,
};
use common::AppError;
use decisions::*;
use shared_types::{
    ActionRunKind, ActionRunStatus, AutomationDecision, AutomationDecisionKind,
    AutomationRuntimeState, ExecutionArtifactBuildRequest, HedgeConfirmRequest,
    HedgeConfirmResponse, HedgeConfirmStatus,
};
use std::collections::BTreeSet;
use uuid::Uuid;

const DECISION_SOURCE: &str = "automated_arbitrage";

#[allow(clippy::too_many_lines)]
pub(crate) async fn evaluate_once(state: &AppState, now_ms: i64) {
    let snapshot = state.automation().snapshot();
    let active = active_runs(state, now_ms);
    if let Some((runtime_state, reason)) = entry_safety_blocker(state, &snapshot) {
        record_if_changed(state, runtime_state, reason, active.len(), now_ms);
        return;
    }
    if let Some((runtime_state, reason)) =
        active_capacity_state(&active, snapshot.config.max_concurrent_runs)
    {
        if runtime_state == AutomationRuntimeState::Blocked {
            record_if_changed(state, runtime_state, reason, active.len(), now_ms);
        } else {
            sync_runtime_if_changed(state, runtime_state, active.len(), now_ms);
        }
        return;
    }
    if let Some((runtime_state, _)) = cooldown_blocker(&snapshot, now_ms) {
        sync_runtime_if_changed(state, runtime_state, active.len(), now_ms);
        return;
    }

    let view = state.opportunity_index().read();
    let active_ids = active
        .iter()
        .map(|run| run.opportunity_id.clone())
        .collect::<BTreeSet<_>>();
    let risk = state.trading_service().risk_config();
    let scoped_rows = view
        .iter()
        .flat_map(|view| view.rows().iter())
        .filter(|row| {
            candidate_allowed(&risk, &row.long_exchange, &row.short_exchange, &row.symbol)
        });
    let candidates = select_candidates(
        scoped_rows,
        &snapshot.config,
        &active_ids,
        now_ms,
        MAX_PREVIEW_CANDIDATES,
    );
    if candidates.is_empty() {
        record_if_changed(
            state,
            AutomationRuntimeState::Watching,
            "no preview-ready opportunity passed strategy, risk scope, verified positive net edge and freshness gates",
            active.len(),
            now_ms,
        );
        return;
    }

    let preview_attempt_count = candidates.len();
    let mut preview_rejections = Vec::new();
    for candidate in candidates {
        record(
            state,
            decision(
                AutomationDecisionKind::CandidateSelected,
                Some((&candidate.opportunity.id, &candidate.opportunity.symbol)),
                candidate.reason.clone(),
                (None, None),
                now_ms,
            ),
            AutomationRuntimeState::Previewing,
            active.len(),
            None,
        );
        let preview = match build_candidate_preview(
            state,
            &candidate,
            view.as_ref().map_or("", |view| view.snapshot_id()),
            &snapshot.config,
        )
        .await
        {
            Ok(preview) => preview,
            Err(failure) => {
                preview_rejections
                    .push(format!("{}: {}", candidate.opportunity.id, failure.reason));
                record_candidate_preview_blocked(
                    state,
                    (&candidate.opportunity.id, &candidate.opportunity.symbol),
                    failure,
                    active.len(),
                    now_ms,
                );
                continue;
            }
        };

        let artifact = match crate::services::execution_artifact::build(
            state,
            &ExecutionArtifactBuildRequest {
                idempotency_key: preview.idempotency_key.clone(),
                ticket_id: preview.ticket.ticket_id.clone(),
                opportunity_snapshot_id: preview.opportunity_snapshot_id.clone(),
            },
        ) {
            Ok(artifact) if artifact.status.is_ready() => artifact,
            Ok(artifact) => {
                let failure = PreviewFailure {
                    reason: format!(
                        "execution artifact is not ready: {}",
                        artifact.blockers.join("; ")
                    ),
                    problem: None,
                };
                preview_rejections
                    .push(format!("{}: {}", candidate.opportunity.id, failure.reason));
                record_candidate_preview_blocked(
                    state,
                    (&candidate.opportunity.id, &candidate.opportunity.symbol),
                    failure,
                    active.len(),
                    now_ms,
                );
                continue;
            }
            Err(error) => {
                record_error(
                    state,
                    (&candidate.opportunity.id, &candidate.opportunity.symbol),
                    "execution artifact generation failed",
                    &error,
                    FailureRuntime {
                        active_run_count: active.len(),
                        occurred_at_ms: now_ms,
                        cooldown_secs: snapshot.config.cooldown_secs,
                    },
                );
                return;
            }
        };
        record_qualified(
            state,
            (&candidate.opportunity.id, &candidate.opportunity.symbol),
            artifact,
            active.len(),
            now_ms,
        );
        let refreshed = state.automation().snapshot();
        let pre_submit_now_ms = common::time::now_ms();
        let refreshed_active = active_runs(state, pre_submit_now_ms);
        if entry_blocker(state, &refreshed, refreshed_active.len(), pre_submit_now_ms).is_some() {
            record_if_changed(
                state,
                AutomationRuntimeState::Blocked,
                "automation state changed after preview; automatic submission suppressed",
                refreshed_active.len(),
                pre_submit_now_ms,
            );
            return;
        }
        // `hedge_confirm` is the existing atomic submit service name. Automation calls it
        // directly under standing authorization; no user confirmation is requested here.
        let response = match crate::services::hedge_confirm::confirm(
            state,
            candidate.opportunity.id.clone(),
            HedgeConfirmRequest {
                idempotency_key: preview.idempotency_key.clone(),
                ticket_id: Some(preview.ticket.ticket_id.clone()),
            },
            ActionRunStart {
                kind: ActionRunKind::HedgeConfirm,
                actor: "system:automated-arbitrage".to_owned(),
                target: Some(candidate.opportunity.id.clone()),
                idempotency_key: Some(preview.idempotency_key.clone()),
                message: "automated hedge submission accepted".to_owned(),
            },
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                record_error(
                    state,
                    (&candidate.opportunity.id, &candidate.opportunity.symbol),
                    "automatic hedge submission failed",
                    &error,
                    FailureRuntime {
                        active_run_count: active.len(),
                        occurred_at_ms: now_ms,
                        cooldown_secs: failed_submission_cooldown_secs(
                            snapshot.config.cooldown_secs,
                        ),
                    },
                );
                return;
            }
        };
        record_submission(
            state,
            &candidate,
            &response,
            now_ms,
            snapshot.config.cooldown_secs,
        );
        return;
    }
    record_preview_exhausted(
        state,
        preview_attempt_count,
        &preview_rejections,
        active.len(),
        now_ms,
        snapshot.config.cooldown_secs,
    );
}

fn active_capacity_state(
    active: &[shared_types::ExecutionRun],
    max_concurrent_runs: usize,
) -> Option<(AutomationRuntimeState, &'static str)> {
    if active.len() < max_concurrent_runs {
        return None;
    }
    if active.iter().any(|run| {
        matches!(
            run.state,
            shared_types::ExecutionRunState::UnwindRequired
                | shared_types::ExecutionRunState::Unwinding
        )
    }) {
        return Some((
            AutomationRuntimeState::Blocked,
            "an active automated execution requires recovery",
        ));
    }
    if active
        .iter()
        .all(|run| run.state == shared_types::ExecutionRunState::Hedged)
    {
        return Some((
            AutomationRuntimeState::Hedged,
            "maximum concurrent automated positions are hedged",
        ));
    }
    Some((
        AutomationRuntimeState::Submitting,
        "maximum concurrent automated entries are awaiting finality",
    ))
}
