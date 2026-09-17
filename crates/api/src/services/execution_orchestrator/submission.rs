use shared_types::{ApiProblem, HedgeLegRole, HedgePreviewResponse, OrderSubmissionContext};

pub(super) fn ticket_submission_context(
    preview: &HedgePreviewResponse,
    role: HedgeLegRole,
) -> Result<OrderSubmissionContext, Box<ApiProblem>> {
    let plans = preview.ticket_order_plans.as_ref().ok_or_else(|| {
        Box::new(
            crate::services::execution_runs::invalid_ticket_order_plan_problem(
                &preview.ticket.ticket_id,
                "ticket order plans are missing",
            ),
        )
    })?;
    let [long, short] = plans
        .plans_for_ticket(&preview.ticket.ticket_id)
        .map_err(|error| {
            Box::new(
                crate::services::execution_runs::invalid_ticket_order_plan_problem(
                    &preview.ticket.ticket_id,
                    &error.to_string(),
                ),
            )
        })?;
    let selected = match role {
        HedgeLegRole::Long => long.compile_plan.submission_context(),
        HedgeLegRole::Short => short.compile_plan.submission_context(),
    };
    let selected_plan = match role {
        HedgeLegRole::Long => &long.compile_plan,
        HedgeLegRole::Short => &short.compile_plan,
    };
    if preview.long_leg.mode == shared_types::ExecutionMode::Live
        || preview.short_leg.mode == shared_types::ExecutionMode::Live
    {
        if let Err(error) = selected_plan.validate_sizing_contract() {
            return Err(Box::new(
                crate::services::execution_runs::invalid_ticket_order_plan_problem(
                    &preview.ticket.ticket_id,
                    error.code(),
                ),
            ));
        }
    }
    Ok(selected)
}
