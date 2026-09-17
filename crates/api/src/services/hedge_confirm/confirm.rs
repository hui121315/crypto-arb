use super::confirm_replay::{
    confirm_action_in_flight, confirm_replay_failed_or_problem, confirm_replay_unavailable,
};
use super::confirm_validate::{
    pre_trade_check, validate_confirm, validate_confirm_scoped_preflight, validate_confirm_ticket,
};
use super::*;

#[path = "confirm/context.rs"]
mod context;

use context::{confirm_preview_context, confirm_request_context, with_confirm_context};

pub(crate) async fn confirm(
    state: &AppState,
    id: String,
    req: HedgeConfirmRequest,
    start: ActionRunStart,
) -> Result<HedgeConfirmResponse, AppError> {
    let request_context = confirm_request_context(&id, &req);
    let begin = action_runs::begin_idempotent(state, start)?;
    if begin.is_replayed() {
        return replay_confirm_response(state, begin.run(), &req.idempotency_key);
    }
    let preview = state
        .hedge_previews()
        .get(&req.idempotency_key)
        .map(|entry| entry.clone())
        .ok_or_else(|| {
            AppError::domain(
                StatusCode::NOT_FOUND,
                codes::HEDGE_PREVIEW_NOT_FOUND,
                format!("hedge preview: {}", req.idempotency_key),
            )
            .with_details(serde_json::json!({ "confirmContext": request_context }))
        });
    let mut preview = match preview {
        Ok(preview) => preview,
        Err(error) => return action_runs::fail_response(state, &begin.run().id, error),
    };
    let confirm_context = confirm_preview_context(&preview, &req.idempotency_key);
    if let Err(error) =
        validate_confirm(&id, &preview).and_then(|()| validate_confirm_ticket(&req, &preview))
    {
        return action_runs::fail_response(
            state,
            &begin.run().id,
            with_confirm_context(error, &confirm_context),
        );
    }
    if let Err(error) = validate_confirm_scoped_preflight(state, &preview).await {
        return action_runs::fail_response(
            state,
            &begin.run().id,
            with_confirm_context(error, &confirm_context),
        );
    }
    if let Err(error) =
        crate::services::hedge_margin::ensure_final_margin(state, &mut preview).await
    {
        return action_runs::fail_response(
            state,
            &begin.run().id,
            with_confirm_context(error, &confirm_context),
        );
    }
    if let Err(error) = pre_trade_check(state, &mut preview).await {
        return action_runs::fail_response(
            state,
            &begin.run().id,
            with_confirm_context(error, &confirm_context),
        );
    }
    if let Err(error) = validate_confirm_ticket(&req, &preview) {
        return action_runs::fail_response(
            state,
            &begin.run().id,
            with_confirm_context(error, &confirm_context),
        );
    }
    crate::services::hedge_preview::persist_preview(state, &preview);
    let response = crate::services::execution_orchestrator::confirm_preview(
        state,
        preview,
        req.idempotency_key,
    )
    .await;
    state.hedge_previews().remove(&response.idempotency_key);
    finish_confirm_action(state, begin.run(), response)
}

pub(crate) fn replay_confirm_response(
    state: &AppState,
    action_run: &ActionRun,
    idempotency_key: &str,
) -> Result<HedgeConfirmResponse, AppError> {
    let run_id = run_id_for_key(idempotency_key);
    if action_run.result.is_some() {
        let mut response = action_runs::replay_payload::<HedgeConfirmResponse>(action_run)?;
        if let Some(run) = state
            .execution_runs()
            .get(&run_id)
            .map(|entry| entry.clone())
        {
            let problem = confirm_run_problem(&run);
            let error = problem.as_ref().map(|problem| problem.message.clone());
            response.status = shared_types::HedgeConfirmStatus::Replayed;
            response.execution_run = Some(run);
            response.problem = response.problem.or(problem);
            response.error = response.error.or(error);
        }
        return Ok(response);
    }
    if let Some(run) = state
        .execution_runs()
        .get(&run_id)
        .map(|entry| entry.clone())
    {
        let problem = confirm_run_problem(&run);
        let error = problem.as_ref().map(|problem| problem.message.clone());
        return Ok(HedgeConfirmResponse {
            idempotency_key: idempotency_key.to_owned(),
            status: shared_types::HedgeConfirmStatus::Replayed,
            context: shared_types::HedgeConfirmContext {
                idempotency_key: idempotency_key.to_owned(),
                ticket_id: Some(run.ticket_id.clone()),
                run_id: Some(run.run_id.clone()),
                opportunity_id: run.opportunity_id.clone(),
                long_venue: Some(run.long_leg.exchange.clone()),
                short_venue: Some(run.short_leg.exchange.clone()),
                ..shared_types::HedgeConfirmContext::default()
            },
            execution_run: Some(run),
            long_record: None,
            short_record: None,
            unwind_record: None,
            problem,
            partial_outcome: None,
            error,
        });
    }
    match action_run.status {
        ActionRunStatus::Accepted => Err(confirm_action_in_flight(action_run, idempotency_key)),
        ActionRunStatus::Failed => Err(confirm_replay_failed_or_problem(
            action_run,
            idempotency_key,
        )),
        ActionRunStatus::Succeeded => Err(confirm_replay_unavailable(action_run, idempotency_key)),
    }
}

pub(super) fn run_id_for_key(idempotency_key: &str) -> String {
    format!("run-{idempotency_key}")
}

pub(crate) fn finish_confirm_action(
    state: &AppState,
    action_run: &ActionRun,
    response: HedgeConfirmResponse,
) -> Result<HedgeConfirmResponse, AppError> {
    let status = confirm_action_status(&response);
    let problem = confirm_action_problem(&response);
    action_runs::finish_status_with_payload(
        state,
        &action_run.id,
        status,
        format!("hedge confirm {}", response.status),
        problem,
        &response,
    )?;
    Ok(response)
}

pub(crate) fn confirm_action_status(response: &HedgeConfirmResponse) -> ActionRunStatus {
    if response.problem.is_some() {
        return ActionRunStatus::Failed;
    }
    if response.error.is_some() {
        return ActionRunStatus::Failed;
    }
    match response.execution_run.as_ref().map(|run| run.state) {
        Some(ExecutionRunState::Hedged | ExecutionRunState::SecondLegSubmitted) => {
            ActionRunStatus::Succeeded
        }
        _ => ActionRunStatus::Failed,
    }
}

pub(crate) fn confirm_action_problem(
    response: &HedgeConfirmResponse,
) -> Option<shared_types::ApiProblem> {
    if confirm_action_status(response) != ActionRunStatus::Failed {
        return None;
    }
    if let Some(problem) = response
        .problem
        .clone()
        .or_else(|| {
            response
                .execution_run
                .as_ref()
                .and_then(confirm_run_problem)
        })
        .or_else(|| {
            response
                .partial_outcome
                .as_ref()
                .and_then(confirm_partial_outcome_problem)
        })
    {
        return Some(problem_with_partial_outcome(problem, response));
    }
    Some(problem_with_partial_outcome(
        shared_types::ApiProblem::new(
            codes::HEDGE_CONFIRM_NOT_HEDGED,
            response
                .error
                .clone()
                .unwrap_or_else(|| format!("hedge confirm ended as {}", response.status)),
        )
        .with_status(StatusCode::CONFLICT.as_u16()),
        response,
    ))
}

fn confirm_partial_outcome_problem(
    outcome: &shared_types::HedgeConfirmPartialOutcome,
) -> Option<shared_types::ApiProblem> {
    outcome
        .unwind_problem
        .clone()
        .or_else(|| outcome.primary_problem.clone())
}

fn confirm_run_problem(run: &shared_types::ExecutionRun) -> Option<shared_types::ApiProblem> {
    run.unwind_problem
        .clone()
        .or_else(|| run.valuation_problem.clone())
}

fn problem_with_partial_outcome(
    mut problem: shared_types::ApiProblem,
    response: &HedgeConfirmResponse,
) -> shared_types::ApiProblem {
    let Some(outcome) = response.partial_outcome.as_ref() else {
        return problem;
    };
    let outcome = encoded_partial_outcome(outcome);
    let mut object = match problem.details.take() {
        Some(serde_json::Value::Object(object)) => object,
        Some(value) => serde_json::Map::from_iter([("originalDetails".to_owned(), value)]),
        None => serde_json::Map::new(),
    };
    object.insert("confirmContext".into(), serde_json::json!(response.context));
    object.insert("partialOutcome".into(), outcome);
    object.insert("confirmStatus".into(), serde_json::json!(response.status));
    problem.details = Some(serde_json::Value::Object(object));
    problem
}

fn encoded_partial_outcome(
    outcome: &shared_types::HedgeConfirmPartialOutcome,
) -> serde_json::Value {
    match serde_json::to_value(outcome) {
        Ok(value) => value,
        Err(error) => serde_json::json!({
            "encodingProblem": {
                "code": codes::HEDGE_CONFIRM_PARTIAL_OUTCOME_ENCODE_FAILED,
                "message": error.to_string(),
            }
        }),
    }
}
