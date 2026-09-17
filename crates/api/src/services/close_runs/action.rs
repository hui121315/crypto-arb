use super::*;

struct ActionRunPayloadRefresh<'a> {
    action_run_id: Option<&'a str>,
    status: ActionRunStatus,
    message: String,
    problem: Option<ApiProblem>,
}

pub(super) fn unwind_plan_status_label(run: &CloseRun) -> &'static str {
    match run
        .unwind_plan
        .as_ref()
        .map(|plan| plan.status)
        .unwrap_or(CloseRunUnwindPlanStatus::BlockedPendingManualRecheck)
    {
        CloseRunUnwindPlanStatus::BlockedPendingManualRecheck => "blocked_pending_manual_recheck",
        CloseRunUnwindPlanStatus::CompensationSubmitted => "compensation_submitted",
        CloseRunUnwindPlanStatus::Compensated => "compensated",
        CloseRunUnwindPlanStatus::CompensationFailed => "compensation_failed",
        CloseRunUnwindPlanStatus::ManualTerminalRecorded => "manual_terminal_recorded",
    }
}

pub(super) fn unwind_plan_filled_legs(run: &CloseRun) -> Vec<CloseRunUnwindLegEvidence> {
    run.unwind_plan
        .as_ref()
        .map(|plan| plan.filled_legs.clone())
        .unwrap_or_default()
}

pub(super) fn unwind_plan_failed_legs(run: &CloseRun) -> Vec<CloseRunUnwindLegEvidence> {
    run.unwind_plan
        .as_ref()
        .map(|plan| plan.failed_legs.clone())
        .unwrap_or_default()
}

pub(super) fn unwind_plan_candidates(run: &CloseRun) -> Vec<CloseRunUnwindLegEvidence> {
    run.unwind_plan
        .as_ref()
        .map(|plan| plan.compensation_candidates.clone())
        .unwrap_or_default()
}

pub(super) fn unwind_plan_attempts(run: &CloseRun) -> Vec<CloseRunCompensationAttempt> {
    run.unwind_plan
        .as_ref()
        .map(|plan| plan.compensation_attempts.clone())
        .unwrap_or_default()
}

pub(super) fn unwind_plan_remaining_positions(run: &CloseRun) -> Vec<CloseRunUnwindLegEvidence> {
    run.unwind_plan
        .as_ref()
        .map(|plan| plan.remaining_positions.clone())
        .unwrap_or_default()
}

pub(super) fn unwind_plan_next_actions(run: &CloseRun) -> Vec<CloseRunNextAction> {
    run.unwind_plan
        .as_ref()
        .map(|plan| plan.next_actions.clone())
        .unwrap_or_default()
}

pub(crate) fn action_status(status: CloseRunStatus) -> ActionRunStatus {
    match status {
        CloseRunStatus::Submitted | CloseRunStatus::CompensationSubmitted => {
            ActionRunStatus::Accepted
        }
        CloseRunStatus::Succeeded | CloseRunStatus::Compensated => ActionRunStatus::Succeeded,
        CloseRunStatus::ManuallyResolved => ActionRunStatus::Succeeded,
        CloseRunStatus::PartiallySubmitted
        | CloseRunStatus::UnwindRequired
        | CloseRunStatus::CompensationFailed
        | CloseRunStatus::Failed => ActionRunStatus::Failed,
    }
}

pub(super) fn refresh_action_run_payload(state: &AppState, run: &CloseRun) {
    let mut refreshed = Vec::new();
    refresh_action_run_id(
        state,
        run,
        &mut refreshed,
        ActionRunPayloadRefresh {
            action_run_id: run.action_run_id.as_deref(),
            status: action_status(run.status),
            message: run.message.clone(),
            problem: run.problem.clone(),
        },
    );
    if let Some(plan) = run.unwind_plan.as_ref() {
        for attempt in &plan.compensation_attempts {
            refresh_action_run_id(
                state,
                run,
                &mut refreshed,
                ActionRunPayloadRefresh {
                    action_run_id: attempt.action_run_id.as_deref(),
                    status: compensation_attempt_action_status(attempt),
                    message: compensation_attempt_action_message(attempt),
                    problem: attempt.problem.clone(),
                },
            );
        }
        refresh_action_run_id(
            state,
            run,
            &mut refreshed,
            ActionRunPayloadRefresh {
                action_run_id: plan
                    .manual_terminal_evidence
                    .as_ref()
                    .and_then(|evidence| evidence.action_run_id.as_deref()),
                status: ActionRunStatus::Succeeded,
                message: "close-run manual terminal evidence recorded".to_owned(),
                problem: None,
            },
        );
    }
}

fn refresh_action_run_id(
    state: &AppState,
    run: &CloseRun,
    refreshed: &mut Vec<String>,
    refresh: ActionRunPayloadRefresh<'_>,
) {
    let Some(action_run_id) = clean_action_run_id(refresh.action_run_id) else {
        return;
    };
    if refreshed.iter().any(|id| id == &action_run_id) {
        return;
    }
    if let Err(error) = action_runs::finish_status_with_payload(
        state,
        &action_run_id,
        refresh.status,
        refresh.message,
        refresh.problem,
        run,
    ) {
        tracing::warn!(
            close_run_id = %run.id,
            action_run_id = %action_run_id,
            error = %error,
            "failed to refresh close run action payload"
        );
    }
    refreshed.push(action_run_id);
}

fn compensation_attempt_action_status(attempt: &CloseRunCompensationAttempt) -> ActionRunStatus {
    if compensation_attempt_filled(attempt) {
        ActionRunStatus::Succeeded
    } else if compensation_attempt_failed(attempt) {
        ActionRunStatus::Failed
    } else {
        ActionRunStatus::Accepted
    }
}

fn compensation_attempt_action_message(attempt: &CloseRunCompensationAttempt) -> String {
    match compensation_attempt_action_status(attempt) {
        ActionRunStatus::Succeeded => "close-run compensation order filled".to_owned(),
        ActionRunStatus::Failed => "close-run compensation order failed".to_owned(),
        ActionRunStatus::Accepted => "close-run compensation order submitted".to_owned(),
    }
}

fn clean_action_run_id(id: Option<&str>) -> Option<String> {
    id.map(str::trim)
        .filter(|id| !id.is_empty())
        .map(ToOwned::to_owned)
}
