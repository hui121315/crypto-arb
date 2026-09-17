use super::confirm::run_id_for_key;
use super::*;

pub(super) fn confirm_action_in_flight(action_run: &ActionRun, idempotency_key: &str) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_IN_FLIGHT,
        "hedge confirm idempotency key is already in flight",
    )
    .with_details(serde_json::json!({
        "actionRunId": action_run.id,
        "requestId": action_run.request_id,
        "idempotencyKey": idempotency_key,
        "status": action_run.status,
    }))
}

pub(super) fn confirm_replay_failed_or_problem(
    action_run: &ActionRun,
    idempotency_key: &str,
) -> AppError {
    if let Some(problem) = action_run.problem.as_ref() {
        return confirm_replay_problem(action_run, idempotency_key, problem);
    }
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_FAILED,
        "hedge confirm idempotency key already failed",
    )
    .with_details(serde_json::json!({
        "actionRunId": action_run.id,
        "requestId": action_run.request_id,
        "idempotencyKey": idempotency_key,
        "status": action_run.status,
    }))
}

fn confirm_replay_problem(
    action_run: &ActionRun,
    idempotency_key: &str,
    problem: &ApiProblem,
) -> AppError {
    AppError::domain(
        confirm_problem_status(problem),
        confirm_replay_problem_code(&problem.code),
        problem.message.clone(),
    )
    .with_details(serde_json::json!({
        "actionRunId": action_run.id,
        "requestId": action_run.request_id,
        "idempotencyKey": idempotency_key,
        "status": action_run.status,
        "replayed": true,
        "originalProblem": problem,
    }))
}

pub(super) fn confirm_replay_unavailable(
    action_run: &ActionRun,
    idempotency_key: &str,
) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_UNAVAILABLE,
        "hedge confirm replay is missing execution run",
    )
    .with_details(serde_json::json!({
        "actionRunId": action_run.id,
        "requestId": action_run.request_id,
        "idempotencyKey": idempotency_key,
        "status": action_run.status,
        "expectedRunId": run_id_for_key(idempotency_key),
    }))
}

fn confirm_problem_status(problem: &ApiProblem) -> StatusCode {
    problem
        .status
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::CONFLICT)
}

fn confirm_replay_problem_code(code: &str) -> &'static str {
    match code {
        codes::HEDGE_PREVIEW_NOT_FOUND => codes::HEDGE_PREVIEW_NOT_FOUND,
        codes::HEDGE_PREVIEW_OPPORTUNITY_MISMATCH => codes::HEDGE_PREVIEW_OPPORTUNITY_MISMATCH,
        codes::HEDGE_PREVIEW_RISK_BLOCKED => codes::HEDGE_PREVIEW_RISK_BLOCKED,
        codes::HEDGE_TICKET_BLOCKED => codes::HEDGE_TICKET_BLOCKED,
        codes::HEDGE_TICKET_EXPIRED => codes::HEDGE_TICKET_EXPIRED,
        codes::HEDGE_TICKET_REQUIRED => codes::HEDGE_TICKET_REQUIRED,
        codes::HEDGE_TICKET_MISMATCH => codes::HEDGE_TICKET_MISMATCH,
        codes::HEDGE_PRE_TRADE_REJECTED => codes::HEDGE_PRE_TRADE_REJECTED,
        codes::HEDGE_CONFIRM_NOT_HEDGED => codes::HEDGE_CONFIRM_NOT_HEDGED,
        codes::ACTION_RUN_IN_FLIGHT => codes::ACTION_RUN_IN_FLIGHT,
        codes::ACTION_RUN_REPLAY_UNAVAILABLE => codes::ACTION_RUN_REPLAY_UNAVAILABLE,
        "NOT_FOUND" => "NOT_FOUND",
        "BAD_REQUEST" => "BAD_REQUEST",
        _ => codes::ACTION_RUN_REPLAY_FAILED,
    }
}
