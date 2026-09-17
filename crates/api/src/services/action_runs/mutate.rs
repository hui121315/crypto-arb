use super::audit_log::{audit_outcome, record_audit, to_json_value};
use super::*;

struct ActionRunUpdate {
    status: ActionRunStatus,
    message: String,
    problem: Option<ApiProblem>,
    result: Option<serde_json::Value>,
    mutation: Option<ActionMutationDiff>,
}

pub(crate) fn finish_result_with_payload<T>(
    state: &AppState,
    run_id: &str,
    result: Result<T, AppError>,
    success_message: impl Into<String>,
) -> Result<T, AppError>
where
    T: Serialize,
{
    finish_result_with_payload_and_mutation(state, run_id, result, success_message, None)
}

pub(crate) fn finish_result_with_payload_and_mutation<T>(
    state: &AppState,
    run_id: &str,
    result: Result<T, AppError>,
    success_message: impl Into<String>,
    mutation: Option<ActionMutationDiff>,
) -> Result<T, AppError>
where
    T: Serialize,
{
    match result {
        Ok(value) => {
            let payload = match encode_payload(&value) {
                Ok(payload) => payload,
                Err(error) => return fail_response(state, run_id, error),
            };
            succeed_with_payload(state, run_id, success_message, payload, mutation)?;
            Ok(value)
        }
        Err(error) => fail_response(state, run_id, error),
    }
}

pub(crate) fn replay_payload<T>(run: &ActionRun) -> Result<T, AppError>
where
    T: DeserializeOwned,
{
    let value = run
        .result
        .as_ref()
        .ok_or_else(|| replay_payload_error::<T>(run, "missing_result", None))?;
    serde_json::from_value(value.clone()).map_err(|error| {
        let error = error.to_string();
        replay_payload_error::<T>(run, "decode_failed", Some(error.as_str()))
    })
}

pub(crate) fn fail_response<T>(
    state: &AppState,
    run_id: &str,
    error: AppError,
) -> Result<T, AppError> {
    fail(state, run_id, &error)?;
    Err(error)
}

#[cfg(test)]
pub(crate) fn finish_status(
    state: &AppState,
    run_id: &str,
    status: ActionRunStatus,
    message: impl Into<String>,
    problem: Option<ApiProblem>,
) -> Result<Option<ActionRun>, AppError> {
    update_run(state, run_id, status, message.into(), problem)
}

pub(crate) fn finish_status_with_payload<T>(
    state: &AppState,
    run_id: &str,
    status: ActionRunStatus,
    message: impl Into<String>,
    problem: Option<ApiProblem>,
    result: &T,
) -> Result<ActionRun, AppError>
where
    T: Serialize,
{
    let result = match encode_payload(result) {
        Ok(value) => Some(value),
        Err(error) => {
            fail(state, run_id, &error)?;
            return Err(error);
        }
    };
    update_run_fields(
        state,
        run_id,
        ActionRunUpdate {
            status,
            message: message.into(),
            problem,
            result,
            mutation: None,
        },
    )?
    .ok_or_else(|| action_run_update_missing_error::<T>(run_id))
}

fn succeed_with_payload(
    state: &AppState,
    run_id: &str,
    message: impl Into<String>,
    result: serde_json::Value,
    mutation: Option<ActionMutationDiff>,
) -> Result<Option<ActionRun>, AppError> {
    update_run_fields(
        state,
        run_id,
        ActionRunUpdate {
            status: ActionRunStatus::Succeeded,
            message: message.into(),
            problem: None,
            result: Some(result),
            mutation,
        },
    )
}

fn encode_payload<T>(result: &T) -> Result<serde_json::Value, AppError>
where
    T: Serialize,
{
    serde_json::to_value(result).map_err(|error| {
        let error = error.to_string();
        replay_contract_error::<T>(
            "action run payload encode failed",
            "encode_failed",
            Some(error.as_str()),
            None,
        )
    })
}

fn replay_payload_error<T>(run: &ActionRun, reason: &str, error: Option<&str>) -> AppError {
    replay_contract_error::<T>(
        "action run replay payload is unavailable",
        reason,
        error,
        Some(run),
    )
}

fn action_run_update_missing_error<T>(run_id: &str) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_UNAVAILABLE,
        "action run payload update target is unavailable",
    )
    .with_details(json!({
        "actionRunId": run_id,
        "payloadType": type_name::<T>(),
        "reason": "missing_action_run",
    }))
}

fn replay_contract_error<T>(
    message: &'static str,
    reason: &str,
    error: Option<&str>,
    run: Option<&ActionRun>,
) -> AppError {
    let details = json!({
        "actionRunId": run.map(|run| run.id.as_str()),
        "requestId": run.and_then(|run| run.request_id.as_deref()),
        "idempotencyKey": run.and_then(|run| run.idempotency_key.as_deref()),
        "status": run.map(|run| to_json_value(&run.status)),
        "payloadType": type_name::<T>(),
        "reason": reason,
        "error": error,
    });
    AppError::domain(
        StatusCode::CONFLICT,
        codes::ACTION_RUN_REPLAY_UNAVAILABLE,
        message,
    )
    .with_details(details)
}

fn fail(state: &AppState, run_id: &str, error: &AppError) -> Result<Option<ActionRun>, AppError> {
    update_run_fields(
        state,
        run_id,
        ActionRunUpdate {
            status: ActionRunStatus::Failed,
            message: error.to_string(),
            problem: Some(problem_from_error(error)),
            result: None,
            mutation: None,
        },
    )
}

#[cfg(test)]
fn update_run(
    state: &AppState,
    run_id: &str,
    status: ActionRunStatus,
    message: String,
    problem: Option<ApiProblem>,
) -> Result<Option<ActionRun>, AppError> {
    update_run_fields(
        state,
        run_id,
        ActionRunUpdate {
            status,
            message,
            problem,
            result: None,
            mutation: None,
        },
    )
}

fn update_run_fields(
    state: &AppState,
    run_id: &str,
    update: ActionRunUpdate,
) -> Result<Option<ActionRun>, AppError> {
    let Some(current) = state
        .action_runs()
        .get(run_id)
        .map(|entry| entry.value().clone())
    else {
        return Ok(None);
    };
    let mutation = update.mutation.or(current.mutation.clone());
    let updated = ActionRun {
        status: update.status,
        message: update.message,
        problem: update.problem,
        result: update.result,
        mutation,
        updated_at_ms: common::time::now_ms(),
        ..current
    };
    record_audit(&updated, audit_outcome(&updated))?;
    state
        .action_runs()
        .insert(run_id.to_owned(), updated.clone());
    state
        .ws_hub()
        .notify_activity(realtime::channels::ACTION_RUN_ACTIVITY);
    Ok(Some(updated))
}

fn problem_from_error(error: &AppError) -> ApiProblem {
    error.to_api_problem()
}

pub(super) fn prune(state: &AppState) {
    let overflow = state.action_runs().len().saturating_sub(MAX_ACTION_RUNS);
    if overflow == 0 {
        return;
    }
    let mut runs: Vec<(String, i64)> = state
        .action_runs()
        .iter()
        .map(|entry| (entry.key().clone(), entry.value().updated_at_ms))
        .collect();
    runs.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
    for (id, _) in runs.into_iter().take(overflow) {
        state.action_runs().remove(&id);
    }
}
