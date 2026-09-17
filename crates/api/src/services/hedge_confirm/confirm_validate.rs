use super::*;
use shared_types::ExecutionGuard;

pub(super) fn validate_confirm(id: &str, preview: &HedgePreviewResponse) -> Result<(), AppError> {
    if preview.opportunity_id != id {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_PREVIEW_OPPORTUNITY_MISMATCH,
            "idempotency key does not belong to opportunity",
        ));
    }
    if !preview.long_risk.allowed || !preview.short_risk.allowed {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_PREVIEW_RISK_BLOCKED,
            "hedge preview did not pass risk checks",
        ));
    }
    Ok(())
}

pub(crate) fn validate_ticket_ready(ticket: &HedgeTicket) -> Result<(), AppError> {
    if ticket_ready(ticket) {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::HEDGE_TICKET_BLOCKED,
        format!("hedge ticket blocked: {}", ticket.blockers.join("; ")),
    )
    .with_details(serde_json::json!({ "blockers": ticket.blockers.clone() })))
}

pub(crate) fn ticket_ready(ticket: &HedgeTicket) -> bool {
    ticket.blockers.is_empty() && ticket.guards.iter().all(|guard| guard.passed)
}

pub(super) async fn pre_trade_check(
    state: &AppState,
    preview: &mut HedgePreviewResponse,
) -> Result<(), AppError> {
    if let Some(detail) = crate::services::hedge_recheck::pre_submit_rejection(state, preview).await
    {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_PRE_TRADE_REJECTED,
            detail,
        ));
    }
    Ok(())
}

pub(super) async fn validate_confirm_scoped_preflight(
    state: &AppState,
    preview: &HedgePreviewResponse,
) -> Result<(), AppError> {
    if !preview_uses_live_mode(preview) {
        return Ok(());
    }
    let [long, short] = preview_ticket_order_plans(preview)?;
    let plans = [&long.compile_plan, &short.compile_plan];
    let mut guards = crate::services::hedge_preflight::recheck_hedge_live_order_preflight_guards(
        state,
        &preview.long_leg,
        &long.compile_plan,
        &preview.short_leg,
        &short.compile_plan,
    )
    .await;
    let snapshot = crate::services::venue_operation_health::snapshot(state);
    if let Some(guard) = crate::services::hedge_preflight::live_operation_health_guard(
        ExecutionMode::Live,
        &plans,
        &snapshot.rows,
    ) {
        guards.push(guard);
    }
    validate_confirm_preflight_guards(guards)
}

fn validate_confirm_preflight_guards(guards: Vec<ExecutionGuard>) -> Result<(), AppError> {
    let blocked = guards
        .into_iter()
        .filter(|guard| !guard.passed)
        .collect::<Vec<_>>();
    if blocked.is_empty() {
        return Ok(());
    }
    let detail = blocked
        .iter()
        .map(|guard| guard.detail.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    Err(AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::HEDGE_PRE_TRADE_REJECTED,
        format!("confirm scoped preflight blocked: {detail}"),
    )
    .with_details(serde_json::json!({ "guards": blocked })))
}

fn preview_uses_live_mode(preview: &HedgePreviewResponse) -> bool {
    preview.long_leg.mode == ExecutionMode::Live || preview.short_leg.mode == ExecutionMode::Live
}

fn preview_ticket_order_plans(
    preview: &HedgePreviewResponse,
) -> Result<[&shared_types::hedge::HedgeTicketOrderPlanEvidence; 2], AppError> {
    validate_ticket_order_plans(
        &preview.ticket.ticket_id,
        preview.ticket_order_plans.as_ref(),
    )
}

fn validate_ticket_order_plans<'a>(
    ticket_id: &str,
    ticket_order_plans: Option<&'a shared_types::hedge::HedgeTicketOrderPlans>,
) -> Result<[&'a shared_types::hedge::HedgeTicketOrderPlanEvidence; 2], AppError> {
    let ticket_order_plans = ticket_order_plans.ok_or_else(|| {
        AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_TICKET_BLOCKED,
            "hedge preview is missing ticket-bound order plan evidence",
        )
    })?;
    let plans = ticket_order_plans
        .plans_for_ticket(ticket_id)
        .map_err(|error| {
            AppError::domain(
                StatusCode::BAD_REQUEST,
                codes::HEDGE_TICKET_BLOCKED,
                format!("ticket order plan evidence is invalid: {error}"),
            )
        })?;
    for plan in plans {
        if !plan.compile_plan.blockers.is_empty() {
            return Err(AppError::domain(
                StatusCode::BAD_REQUEST,
                codes::HEDGE_TICKET_BLOCKED,
                format!(
                    "ticket order compile plan is blocked: {}",
                    plan.compile_plan.blockers.join("; ")
                ),
            ));
        }
        if !plan.identity_plan.is_execution_ready() {
            return Err(AppError::domain(
                StatusCode::BAD_REQUEST,
                codes::HEDGE_TICKET_BLOCKED,
                format!(
                    "ticket order identity evidence is blocked: {}",
                    plan.identity_plan.blockers.join("; ")
                ),
            ));
        }
    }
    Ok(plans)
}

pub(super) fn validate_confirm_ticket(
    req: &HedgeConfirmRequest,
    preview: &HedgePreviewResponse,
) -> Result<(), AppError> {
    let ticket = &preview.ticket;
    let requested = required_confirm_ticket_id(req)?;
    if requested != ticket.ticket_id {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_TICKET_MISMATCH,
            "ticketId does not belong to hedge preview",
        ));
    }
    if crate::services::hedge_ticket::ticket_expired(ticket) {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_TICKET_EXPIRED,
            "HedgeTicket 已过期，请重新预览",
        ));
    }
    let plans = preview_ticket_order_plans(preview)?;
    if preview_uses_live_mode(preview) {
        validate_live_sizing_contracts(plans)?;
    }
    validate_ticket_ready(ticket)
}

fn required_confirm_ticket_id(req: &HedgeConfirmRequest) -> Result<&str, AppError> {
    req.ticket_id
        .as_deref()
        .map(str::trim)
        .filter(|ticket_id| !ticket_id.is_empty())
        .ok_or_else(|| {
            AppError::domain(
                StatusCode::BAD_REQUEST,
                codes::HEDGE_TICKET_REQUIRED,
                "ticketId is required to confirm a hedge preview",
            )
            .with_details(serde_json::json!({
                "field": "ticketId",
                "reason": "required",
            }))
        })
}

fn validate_live_sizing_contracts(
    plans: [&shared_types::hedge::HedgeTicketOrderPlanEvidence; 2],
) -> Result<(), AppError> {
    for evidence in plans {
        let plan = &evidence.compile_plan;
        if let Err(error) = plan.validate_sizing_contract() {
            return Err(AppError::domain(
                StatusCode::BAD_REQUEST,
                codes::HEDGE_TICKET_BLOCKED,
                format!(
                    "ticket instrument sizing contract is invalid: {} {} {}",
                    plan.exchange,
                    plan.symbol,
                    error.code()
                ),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "confirm_validate/test_fixtures.rs"]
mod test_fixtures;

#[cfg(test)]
#[path = "confirm_validate/tests.rs"]
mod tests;
