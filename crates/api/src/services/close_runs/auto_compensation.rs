use super::*;

mod retry_policy;

use crate::services::action_runs::ActionRunStart;
use retry_policy::auto_compensation_decision;

const AUTO_COMPENSATION_ACTOR: &str = "system:close-run-auto-compensation";
const AUTO_COMPENSATION_MESSAGE: &str = "close-run auto compensation accepted";
const AUTO_COMPENSATION_REASON: &str =
    "background worker submitted single-candidate close-run compensation";
const AUTO_COMPENSATION_RETRY_REASON: &str =
    "background worker retried failed single-candidate close-run compensation";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AutoCompensationOutcome {
    pub(crate) scanned_run_count: usize,
    pub(crate) eligible_run_count: usize,
    pub(crate) submitted_count: usize,
    pub(crate) skipped_multi_candidate_count: usize,
    pub(crate) skipped_live_count: usize,
    pub(crate) submission_failure_count: usize,
    pub(crate) publish_failure_count: usize,
}

impl AutoCompensationOutcome {
    pub(crate) fn has_activity(&self) -> bool {
        self.eligible_run_count > 0
            || self.submitted_count > 0
            || self.skipped_multi_candidate_count > 0
            || self.skipped_live_count > 0
            || self.submission_failure_count > 0
            || self.publish_failure_count > 0
    }

    pub(crate) fn has_failures(&self) -> bool {
        self.submission_failure_count > 0 || self.publish_failure_count > 0
    }
}

#[derive(Debug, Clone)]
struct AutoCompensationSubmit {
    close_run_id: String,
    request: CloseRunCompensationRequest,
    attempt_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AutoCompensationSkip {
    NotEligible,
    MultipleCandidates,
    LiveMode,
    InvalidCandidate,
}

#[derive(Debug, Clone)]
enum AutoCompensationDecision {
    Submit(AutoCompensationSubmit),
    Skip(AutoCompensationSkip),
}

pub(crate) async fn auto_submit_compensation_once(state: &AppState) -> AutoCompensationOutcome {
    let runs = close_run_snapshots(state);
    let mode = execution_mode(state.trading_service().adapter_name());
    let mut outcome = AutoCompensationOutcome {
        scanned_run_count: runs.len(),
        ..AutoCompensationOutcome::default()
    };
    for run in runs {
        handle_auto_compensation_decision(state, &run, mode, &mut outcome).await;
    }
    outcome
}

async fn handle_auto_compensation_decision(
    state: &AppState,
    run: &CloseRun,
    mode: ExecutionMode,
    outcome: &mut AutoCompensationOutcome,
) {
    match auto_compensation_decision(run, mode) {
        AutoCompensationDecision::Submit(submit) => {
            outcome.eligible_run_count += 1;
            submit_auto_compensation(state, submit, outcome).await;
        }
        AutoCompensationDecision::Skip(skip) => record_auto_compensation_skip(run, skip, outcome),
    }
}

fn record_auto_compensation_skip(
    run: &CloseRun,
    skip: AutoCompensationSkip,
    outcome: &mut AutoCompensationOutcome,
) {
    match skip {
        AutoCompensationSkip::MultipleCandidates => record_multi_candidate_skip(run, outcome),
        AutoCompensationSkip::LiveMode => record_live_skip(run, outcome),
        AutoCompensationSkip::InvalidCandidate => record_invalid_candidate_skip(run, outcome),
        AutoCompensationSkip::NotEligible => {}
    }
}

fn record_multi_candidate_skip(run: &CloseRun, outcome: &mut AutoCompensationOutcome) {
    outcome.eligible_run_count += 1;
    outcome.skipped_multi_candidate_count += 1;
    tracing::warn!(
        close_run_id = %run.id,
        "close-run auto compensation skipped because multiple candidates require operator selection"
    );
}

fn record_live_skip(run: &CloseRun, outcome: &mut AutoCompensationOutcome) {
    outcome.eligible_run_count += 1;
    outcome.skipped_live_count += 1;
    tracing::warn!(
        close_run_id = %run.id,
        "close-run auto compensation skipped in live mode; use manual compensation action"
    );
}

fn record_invalid_candidate_skip(run: &CloseRun, outcome: &mut AutoCompensationOutcome) {
    outcome.eligible_run_count += 1;
    outcome.submission_failure_count += 1;
    tracing::warn!(
        close_run_id = %run.id,
        "close-run auto compensation skipped because candidate is not submittable"
    );
}

fn close_run_snapshots(state: &AppState) -> Vec<CloseRun> {
    let mut runs: Vec<_> = state
        .close_runs()
        .iter()
        .map(|entry| entry.value().clone())
        .collect();
    runs.sort_by(|left, right| {
        left.updated_at_ms
            .cmp(&right.updated_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    runs
}

async fn submit_auto_compensation(
    state: &AppState,
    submit: AutoCompensationSubmit,
    outcome: &mut AutoCompensationOutcome,
) {
    let action_run = match begin_auto_compensation_action_run(state, &submit) {
        Ok(run) => run,
        Err(error) => {
            tracing::warn!(
                close_run_id = %submit.close_run_id,
                %error,
                "close-run auto compensation blocked because durable audit acceptance failed"
            );
            outcome.submission_failure_count += 1;
            return;
        }
    };
    let submission =
        submit_compensation_order(state, &submit.close_run_id, &submit.request, &action_run).await;
    finish_auto_compensation_submission(state, &submit, &action_run, submission, outcome);
}

fn begin_auto_compensation_action_run(
    state: &AppState,
    submit: &AutoCompensationSubmit,
) -> Result<ActionRun, AppError> {
    action_runs::begin(state, action_run_start(submit))
}

fn finish_auto_compensation_submission(
    state: &AppState,
    submit: &AutoCompensationSubmit,
    action_run: &ActionRun,
    submission: Result<CloseRun, AppError>,
    outcome: &mut AutoCompensationOutcome,
) {
    let close_run = match submission {
        Ok(close_run) => close_run,
        Err(error) => {
            tracing::warn!(
                close_run_id = %submit.close_run_id,
                %error,
                "close-run auto compensation submit blocked"
            );
            let _ = action_runs::fail_response::<()>(state, &action_run.id, error);
            outcome.submission_failure_count += 1;
            return;
        }
    };
    if finish_auto_action_run(state, action_run, &close_run).is_err() {
        outcome.submission_failure_count += 1;
        return;
    }
    if publish_auto_compensation_orders(state, action_run, &close_run).is_err() {
        outcome.publish_failure_count += 1;
        return;
    }
    outcome.submitted_count += 1;
}

fn action_run_start(submit: &AutoCompensationSubmit) -> ActionRunStart {
    ActionRunStart {
        kind: ActionRunKind::PortfolioCloseCompensation,
        actor: AUTO_COMPENSATION_ACTOR.to_owned(),
        target: Some(submit.close_run_id.clone()),
        idempotency_key: Some(auto_compensation_key(submit)),
        message: AUTO_COMPENSATION_MESSAGE.to_owned(),
    }
}

fn auto_compensation_key(submit: &AutoCompensationSubmit) -> String {
    let base = format!(
        "portfolio-close:auto-compensate:{}:{}:{}",
        submit.close_run_id,
        submit
            .request
            .snapshot_version
            .as_deref()
            .unwrap_or_default(),
        submit.request.candidate_index.unwrap_or(0),
    );
    format!("{base}:attempt:{}", submit.attempt_index)
}

fn finish_auto_action_run(
    state: &AppState,
    action_run: &ActionRun,
    close_run: &CloseRun,
) -> Result<(), AppError> {
    action_runs::finish_status_with_payload(
        state,
        &action_run.id,
        action_status(close_run.status),
        close_run.message.clone(),
        close_run.problem.clone(),
        close_run,
    )
    .map(|_| ())
}

fn publish_auto_compensation_orders(
    state: &AppState,
    action_run: &ActionRun,
    close_run: &CloseRun,
) -> Result<(), AppError> {
    let Some(plan) = close_run.unwind_plan.as_ref() else {
        return Ok(());
    };
    for order in plan
        .compensation_attempts
        .iter()
        .filter(|attempt| {
            attempt
                .action_run_id
                .as_deref()
                .is_some_and(|id| id == action_run.id)
        })
        .filter_map(|attempt| attempt.order.as_ref())
    {
        crate::services::ws_publish::publish_order_event(
            state,
            "close_run_auto_compensation_submitted",
            order,
        )?;
    }
    Ok(())
}
