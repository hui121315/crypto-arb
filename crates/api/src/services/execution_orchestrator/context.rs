use super::*;

pub(super) fn from_preview(
    preview: &HedgePreviewResponse,
    idempotency_key: &str,
) -> shared_types::HedgeConfirmContext {
    let environment = if preview.long_leg.mode == shared_types::ExecutionMode::Live
        || preview.short_leg.mode == shared_types::ExecutionMode::Live
    {
        shared_types::ExecutionEnvironment::Live
    } else {
        shared_types::ExecutionEnvironment::Paper
    };
    shared_types::HedgeConfirmContext {
        opportunity_id: preview.opportunity_id.clone(),
        idempotency_key: idempotency_key.to_owned(),
        ticket_id: Some(preview.ticket.ticket_id.clone()),
        run_id: None,
        environment: Some(environment),
        long_venue: Some(preview.long_leg.exchange.clone()),
        short_venue: Some(preview.short_leg.exchange.clone()),
        long_problem: None,
        short_problem: None,
    }
}

pub(super) fn attach_response(
    mut response: HedgeConfirmResponse,
    mut context: shared_types::HedgeConfirmContext,
    first_role: HedgeLegRole,
) -> HedgeConfirmResponse {
    if let Some(run) = response.execution_run.as_ref() {
        context.run_id = Some(run.run_id.clone());
        context.ticket_id = Some(run.ticket_id.clone());
        context.long_venue = Some(run.long_leg.exchange.clone());
        context.short_venue = Some(run.short_leg.exchange.clone());
    }
    assign_leg_problems(&response, &mut context, first_role);
    if let Some(outcome) = response.partial_outcome.as_mut() {
        ensure_recheck_problem(outcome);
        outcome.context = context.clone();
    }
    response.context = context.clone();
    response.problem = response
        .problem
        .take()
        .map(|problem| attach_problem(problem, &context));
    response
}

pub(crate) fn attach_problem(
    mut problem: ApiProblem,
    context: &shared_types::HedgeConfirmContext,
) -> ApiProblem {
    let mut object = details_object(problem.details.take());
    object.insert("confirmContext".into(), serde_json::json!(context));
    problem.details = Some(serde_json::Value::Object(object));
    problem
}

fn details_object(
    details: Option<serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    match details {
        Some(serde_json::Value::Object(object)) => object,
        Some(value) => serde_json::Map::from_iter([("originalDetails".to_owned(), value)]),
        None => serde_json::Map::new(),
    }
}

fn assign_leg_problems(
    response: &HedgeConfirmResponse,
    context: &mut shared_types::HedgeConfirmContext,
    first_role: HedgeLegRole,
) {
    let primary = response
        .partial_outcome
        .as_ref()
        .and_then(|outcome| outcome.primary_problem.clone());
    let unwind = response
        .partial_outcome
        .as_ref()
        .and_then(|outcome| outcome.unwind_problem.clone());
    match response.status {
        HedgeConfirmStatus::LongLegFailed => {
            assign_problem(context, first_role, response.problem.clone());
        }
        HedgeConfirmStatus::ValuationMissing => {
            assign_problem(
                context,
                valuation_problem_role(response, first_role),
                response.problem.clone(),
            );
        }
        HedgeConfirmStatus::FirstLegPartialUnwindAttempted
        | HedgeConfirmStatus::FirstLegPartialUnwindFailed
        | HedgeConfirmStatus::FirstLegPartialWaitingFillQty => {
            assign_problem(
                context,
                first_role,
                unwind.or(primary).or_else(|| response.problem.clone()),
            );
        }
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindAttempted
        | HedgeConfirmStatus::HedgeRecheckBlockedUnwindFailed => {
            assign_problem(
                context,
                crate::services::hedge_ticket::opposite_role(first_role),
                primary.or_else(|| recheck_problem(response)),
            );
            assign_problem(context, first_role, unwind);
        }
        HedgeConfirmStatus::HedgeBrokenUnwindAttempted
        | HedgeConfirmStatus::HedgeBrokenUnwindFailed => {
            assign_problem(
                context,
                crate::services::hedge_ticket::opposite_role(first_role),
                primary.or_else(|| response.problem.clone()),
            );
            assign_problem(context, first_role, unwind);
        }
        HedgeConfirmStatus::Submitted
        | HedgeConfirmStatus::Replayed
        | HedgeConfirmStatus::Unknown => {}
    }
}

fn valuation_problem_role(
    response: &HedgeConfirmResponse,
    first_role: HedgeLegRole,
) -> HedgeLegRole {
    match (
        response.long_record.is_some(),
        response.short_record.is_some(),
    ) {
        (true, false) => HedgeLegRole::Long,
        (false, true) => HedgeLegRole::Short,
        (true, true) => crate::services::hedge_ticket::opposite_role(first_role),
        (false, false) => first_role,
    }
}

fn assign_problem(
    context: &mut shared_types::HedgeConfirmContext,
    role: HedgeLegRole,
    problem: Option<ApiProblem>,
) {
    match role {
        HedgeLegRole::Long => context.long_problem = problem,
        HedgeLegRole::Short => context.short_problem = problem,
    }
}

fn ensure_recheck_problem(outcome: &mut HedgeConfirmPartialOutcome) {
    if outcome.cause != HedgeConfirmPartialCause::HedgeRecheckBlocked
        || outcome.primary_problem.is_some()
    {
        return;
    }
    outcome.primary_problem = outcome.primary_message.as_deref().map(|message| {
        ApiProblem::new(codes::HEDGE_PRE_TRADE_REJECTED, message)
            .with_status(axum::http::StatusCode::BAD_REQUEST.as_u16())
            .with_source("hedge_recheck")
            .with_request_id(common::request_id::current())
    });
}

fn recheck_problem(response: &HedgeConfirmResponse) -> Option<ApiProblem> {
    response
        .partial_outcome
        .as_ref()
        .and_then(|outcome| outcome.primary_message.as_deref())
        .map(|message| {
            ApiProblem::new(codes::HEDGE_PRE_TRADE_REJECTED, message)
                .with_status(axum::http::StatusCode::BAD_REQUEST.as_u16())
                .with_source("hedge_recheck")
                .with_request_id(common::request_id::current())
        })
}

#[cfg(test)]
#[path = "context/tests.rs"]
mod tests;
