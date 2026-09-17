use super::*;

pub(super) fn replay_submit_order(
    state: &AppState,
    run: &ActionRun,
    client_order_id: &str,
) -> Result<Json<OrderRecord>, AppError> {
    match run.status {
        ActionRunStatus::Succeeded => replay_succeeded_submit(state, run, client_order_id),
        ActionRunStatus::Failed => Err(submit_replay_failed(run, client_order_id)),
        ActionRunStatus::Accepted => Err(action_run_in_flight(run, client_order_id)),
    }
}

pub(super) fn replay_succeeded_submit(
    state: &AppState,
    run: &ActionRun,
    client_order_id: &str,
) -> Result<Json<OrderRecord>, AppError> {
    if let Some(record) = state
        .trading_service()
        .get_order_by_client_order_id(client_order_id)
    {
        return Ok(Json(record));
    }
    action_runs::replay_payload::<OrderRecord>(run).map(Json)
}

pub(super) async fn replay_cancel_order(
    state: &AppState,
    run: &ActionRun,
    order_id: &str,
    idempotency_key: &str,
) -> Result<Json<OrderRecord>, AppError> {
    match run.status {
        ActionRunStatus::Succeeded => replay_succeeded_cancel(state, run, order_id).await,
        ActionRunStatus::Failed => Err(cancel_replay_failed(run, idempotency_key)),
        ActionRunStatus::Accepted => Err(cancel_action_in_flight(run, idempotency_key)),
    }
}

pub(super) async fn replay_succeeded_cancel(
    state: &AppState,
    run: &ActionRun,
    order_id: &str,
) -> Result<Json<OrderRecord>, AppError> {
    if let Some(record) = state.trading_service().get_order(order_id) {
        let previous_state = record.state;
        let record = state
            .trading_service()
            .refresh_pending_cancel_finality(record)
            .await;
        let record = publish_refreshed_cancel_event(state, previous_state, record)?;
        let message = cancel_action_message(&record);
        let record = action_runs::finish_result_with_payload(state, &run.id, Ok(record), message)?;
        return Ok(Json(record));
    }
    action_runs::replay_payload::<OrderRecord>(run).map(Json)
}

pub(super) fn publish_refreshed_cancel_event(
    state: &AppState,
    previous_state: LiveOrderState,
    record: OrderRecord,
) -> Result<OrderRecord, AppError> {
    if previous_state == record.state {
        return Ok(record);
    }
    cancel_order_event_record(state, record)
}

pub(super) fn cancel_replay_failed(run: &ActionRun, idempotency_key: &str) -> AppError {
    if let Some(error) = restart_replay_unavailable(run, idempotency_key) {
        return error;
    }
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_FAILED,
        "order cancel idempotency key already failed",
    )
    .with_details(serde_json::json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": idempotency_key,
        "problem": run.problem,
    }))
}

pub(super) fn cancel_action_in_flight(run: &ActionRun, idempotency_key: &str) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_IN_FLIGHT,
        "order cancel idempotency key is already in flight",
    )
    .with_details(serde_json::json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": idempotency_key,
        "status": run.status,
    }))
}

pub(super) fn action_run_in_flight(run: &ActionRun, client_order_id: &str) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_IN_FLIGHT,
        "order submit idempotency key is already in flight",
    )
    .with_details(serde_json::json!({
        "actionRunId": run.id,
        "idempotencyKey": client_order_id,
        "status": run.status,
    }))
}

pub(super) fn submit_replay_failed(run: &ActionRun, client_order_id: &str) -> AppError {
    if let Some(error) = restart_replay_unavailable(run, client_order_id) {
        return error;
    }
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_FAILED,
        "order submit idempotency key already failed",
    )
    .with_details(serde_json::json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": client_order_id,
        "problem": run.problem,
    }))
}

fn restart_replay_unavailable(run: &ActionRun, idempotency_key: &str) -> Option<AppError> {
    let problem = run.problem.as_ref()?;
    if problem.code != codes::ACTION_RUN_REPLAY_UNAVAILABLE {
        return None;
    }
    let status = problem
        .status
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::CONFLICT);
    Some(
        AppError::domain(
            status,
            codes::ACTION_RUN_REPLAY_UNAVAILABLE,
            problem.message.clone(),
        )
        .with_details(serde_json::json!({
            "actionRunId": run.id,
            "requestId": run.request_id,
            "idempotencyKey": idempotency_key,
            "replayed": true,
            "originalProblem": problem,
        })),
    )
}

pub(super) fn kill_switch_action_in_flight(run: &ActionRun) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_IN_FLIGHT,
        "kill switch idempotency key is already in flight",
    )
    .with_details(serde_json::json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": run.idempotency_key,
        "status": run.status,
    }))
}

pub(super) fn kill_switch_replay_failed(run: &ActionRun) -> AppError {
    if let Some(problem) = run.problem.as_ref() {
        return kill_switch_replay_problem(run, problem);
    }
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_FAILED,
        "kill switch idempotency key already failed",
    )
    .with_details(serde_json::json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": run.idempotency_key,
        "problem": run.problem,
    }))
}

fn kill_switch_replay_problem(run: &ActionRun, problem: &ApiProblem) -> AppError {
    AppError::domain(
        kill_switch_problem_status(problem),
        kill_switch_replay_problem_code(&problem.code),
        problem.message.clone(),
    )
    .with_details(serde_json::json!({
        "actionRunId": run.id,
        "requestId": run.request_id,
        "idempotencyKey": run.idempotency_key,
        "replayed": true,
        "originalProblem": problem,
    }))
}

fn kill_switch_problem_status(problem: &ApiProblem) -> StatusCode {
    problem
        .status
        .and_then(|status| StatusCode::from_u16(status).ok())
        .unwrap_or(StatusCode::CONFLICT)
}

fn kill_switch_replay_problem_code(code: &str) -> &'static str {
    match code {
        codes::KILL_SWITCH_REQUEST_INVALID => codes::KILL_SWITCH_REQUEST_INVALID,
        codes::KILL_SWITCH_STALE_CONFIRMATION => codes::KILL_SWITCH_STALE_CONFIRMATION,
        codes::ACTION_RUN_IN_FLIGHT => codes::ACTION_RUN_IN_FLIGHT,
        codes::ACTION_RUN_REPLAY_UNAVAILABLE => codes::ACTION_RUN_REPLAY_UNAVAILABLE,
        "BAD_REQUEST" => "BAD_REQUEST",
        _ => codes::ACTION_RUN_REPLAY_FAILED,
    }
}

pub(super) fn cancel_idempotency_key(order_id: &str) -> String {
    format!("cancel:{order_id}")
}

pub(super) fn explicit_idempotency_key(headers: &HeaderMap) -> Option<String> {
    header_text(headers, HEADER_IDEMPOTENCY_KEY)
        .or_else(|| header_text(headers, HEADER_X_IDEMPOTENCY_KEY))
}

pub(super) fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_interrupted_action_run_is_never_replayed_as_a_new_submit() {
        let run = ActionRun {
            id: "act-restart".to_owned(),
            kind: ActionRunKind::TradingOrderSubmit,
            status: ActionRunStatus::Failed,
            actor: "api-token:operator:0123456789abcdef".to_owned(),
            target: Some("client-restart".to_owned()),
            request_id: Some("req-restart".to_owned()),
            idempotency_key: Some("client-restart".to_owned()),
            message: "interrupted".to_owned(),
            problem: Some(
                ApiProblem::new(
                    codes::ACTION_RUN_REPLAY_UNAVAILABLE,
                    "action run terminal outcome is unknown after restart",
                )
                .with_status(StatusCode::CONFLICT.as_u16()),
            ),
            result: None,
            mutation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        };

        let error = submit_replay_failed(&run, "client-restart");

        assert_eq!(error.status(), StatusCode::CONFLICT);
        assert_eq!(error.code(), codes::ACTION_RUN_REPLAY_UNAVAILABLE);
        assert!(error.to_string().contains("terminal outcome is unknown"));
    }
}
