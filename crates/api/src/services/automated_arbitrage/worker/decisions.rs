use super::*;

mod runtime;

pub(super) use runtime::{record_if_changed, sync_runtime_if_changed};

const MIN_FAILED_SUBMISSION_COOLDOWN_SECS: u64 = 60;
const MIN_PREVIEW_RETRY_COOLDOWN_SECS: u64 = 5;

#[derive(Clone, Copy)]
pub(super) struct FailureRuntime {
    pub active_run_count: usize,
    pub occurred_at_ms: i64,
    pub cooldown_secs: u64,
}

pub(super) fn record_qualified(
    state: &AppState,
    target: (&str, &str),
    artifact: shared_types::DeterministicExecutionArtifact,
    active_run_count: usize,
    now_ms: i64,
) {
    let mut qualified = decision(
        AutomationDecisionKind::OpportunityQualified,
        Some(target),
        if artifact.environment == shared_types::ExecutionEnvironment::Live {
            "deterministic opportunity artifact is ready for automatic live execution".to_owned()
        } else {
            "deterministic opportunity artifact is ready for paper execution".to_owned()
        },
        (None, None),
        now_ms,
    );
    qualified.execution_artifact = Some(artifact);
    record(
        state,
        qualified,
        AutomationRuntimeState::Submitting,
        active_run_count,
        None,
    );
}

pub(super) fn record_candidate_preview_blocked(
    state: &AppState,
    target: (&str, &str),
    failure: PreviewFailure,
    active_run_count: usize,
    now_ms: i64,
) {
    record(
        state,
        decision(
            AutomationDecisionKind::PreviewBlocked,
            Some(target),
            failure.reason,
            (None, failure.problem),
            now_ms,
        ),
        AutomationRuntimeState::Previewing,
        active_run_count,
        None,
    );
}

pub(super) fn record_submission(
    state: &AppState,
    candidate: &automation::CandidateSelection,
    response: &HedgeConfirmResponse,
    now_ms: i64,
    cooldown_secs: u64,
) {
    let action_status = crate::services::hedge_confirm::confirm_action_status(response);
    let (kind, runtime_state) = submission_decision(
        action_status,
        response.status,
        response.execution_run.as_ref().map(|run| run.state),
    );
    let problem = crate::services::hedge_confirm::confirm_action_problem(response);
    let execution_run_id = response
        .execution_run
        .as_ref()
        .map(|run| run.run_id.clone());
    let successful = action_status == ActionRunStatus::Succeeded;
    let active_run_count = active_runs(state, common::time::now_ms()).len();
    let cooldown_until_ms = Some(cooldown_deadline(
        now_ms,
        if successful {
            cooldown_secs
        } else {
            failed_submission_cooldown_secs(cooldown_secs)
        },
    ));

    record(
        state,
        decision(
            kind,
            Some((&candidate.opportunity.id, &candidate.opportunity.symbol)),
            format!("hedge confirm {}", response.status),
            (execution_run_id, problem),
            now_ms,
        ),
        runtime_state,
        active_run_count,
        cooldown_until_ms,
    );
}

pub(super) fn record_preview_exhausted(
    state: &AppState,
    attempted_count: usize,
    rejections: &[String],
    active_run_count: usize,
    now_ms: i64,
    cooldown_secs: u64,
) {
    let details = rejections
        .iter()
        .map(|reason| reason.chars().take(180).collect::<String>())
        .collect::<Vec<_>>()
        .join(" | ");
    let details = if details.is_empty() {
        "no per-candidate blocker detail was recorded".to_owned()
    } else {
        details
    };
    let reason =
        format!("all {attempted_count} bounded candidate previews were blocked: {details}");
    record(
        state,
        decision(
            AutomationDecisionKind::PreviewBlocked,
            None,
            reason,
            (None, None),
            now_ms,
        ),
        AutomationRuntimeState::CoolingDown,
        active_run_count,
        Some(cooldown_deadline(
            now_ms,
            preview_retry_cooldown_secs(cooldown_secs),
        )),
    );
}

pub(super) fn submission_decision(
    action_status: ActionRunStatus,
    confirm_status: HedgeConfirmStatus,
    execution_state: Option<shared_types::ExecutionRunState>,
) -> (AutomationDecisionKind, AutomationRuntimeState) {
    if action_status != ActionRunStatus::Succeeded {
        return (
            AutomationDecisionKind::Failed,
            AutomationRuntimeState::Error,
        );
    }
    let kind = if confirm_status == HedgeConfirmStatus::Replayed {
        AutomationDecisionKind::Replayed
    } else {
        AutomationDecisionKind::Submitted
    };
    let runtime_state = match execution_state {
        Some(shared_types::ExecutionRunState::Hedged) => AutomationRuntimeState::Hedged,
        Some(
            shared_types::ExecutionRunState::Previewed
            | shared_types::ExecutionRunState::RiskChecked
            | shared_types::ExecutionRunState::SubmittingFirstLeg
            | shared_types::ExecutionRunState::FirstLegPartial
            | shared_types::ExecutionRunState::SubmittingSecondLeg
            | shared_types::ExecutionRunState::SecondLegSubmitted,
        ) => AutomationRuntimeState::Submitting,
        Some(
            shared_types::ExecutionRunState::UnwindRequired
            | shared_types::ExecutionRunState::Unwinding
            | shared_types::ExecutionRunState::FailedSafe,
        ) => AutomationRuntimeState::Error,
        Some(shared_types::ExecutionRunState::Closed) | None => AutomationRuntimeState::CoolingDown,
    };
    (kind, runtime_state)
}

pub(super) fn cooldown_deadline(now_ms: i64, cooldown_secs: u64) -> i64 {
    now_ms.saturating_add(
        i64::try_from(cooldown_secs)
            .unwrap_or(i64::MAX)
            .saturating_mul(1_000),
    )
}

pub(super) const fn failed_submission_cooldown_secs(configured_secs: u64) -> u64 {
    if configured_secs < MIN_FAILED_SUBMISSION_COOLDOWN_SECS {
        MIN_FAILED_SUBMISSION_COOLDOWN_SECS
    } else {
        configured_secs
    }
}

pub(super) const fn preview_retry_cooldown_secs(configured_secs: u64) -> u64 {
    if configured_secs < MIN_PREVIEW_RETRY_COOLDOWN_SECS {
        MIN_PREVIEW_RETRY_COOLDOWN_SECS
    } else {
        configured_secs
    }
}

pub(super) fn record_error(
    state: &AppState,
    target: (&str, &str),
    reason: &str,
    error: &AppError,
    runtime: FailureRuntime,
) {
    record(
        state,
        decision(
            AutomationDecisionKind::Failed,
            Some(target),
            reason.to_owned(),
            (
                None,
                Some(error.to_api_problem().with_source(DECISION_SOURCE)),
            ),
            runtime.occurred_at_ms,
        ),
        AutomationRuntimeState::Error,
        runtime.active_run_count,
        Some(cooldown_deadline(
            runtime.occurred_at_ms,
            runtime.cooldown_secs,
        )),
    );
}

pub(super) fn record(
    state: &AppState,
    decision: AutomationDecision,
    runtime_state: AutomationRuntimeState,
    active_run_count: usize,
    cooldown_until_ms: Option<i64>,
) {
    let status = state.automation().record_decision(
        decision,
        runtime_state,
        active_run_count,
        cooldown_until_ms,
    );
    if let Err(error) = crate::services::ws_publish::publish_automation_status(state, &status) {
        tracing::warn!(%error, "automation status websocket publish failed");
    }
}

pub(super) fn decision(
    kind: AutomationDecisionKind,
    target: Option<(&str, &str)>,
    reason: String,
    outcome: (Option<String>, Option<shared_types::ApiProblem>),
    now_ms: i64,
) -> AutomationDecision {
    AutomationDecision {
        id: format!("automation-{}", Uuid::new_v4()),
        kind,
        opportunity_id: target.map(|(id, _)| id.to_owned()),
        symbol: target.map(|(_, symbol)| symbol.to_owned()),
        reason,
        execution_run_id: outcome.0,
        problem: outcome.1,
        execution_artifact: None,
        occurred_at_ms: now_ms,
    }
}

#[cfg(test)]
#[path = "decisions/tests.rs"]
mod tests;
