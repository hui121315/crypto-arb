mod assembly;
mod final_margin;
mod guards;
mod intent;
mod positions;
mod pricing;
mod readiness;
#[path = "hedge_preview/workflow_view.rs"]
mod workflow_view;

use crate::state::AppState;
use assembly::{
    build_preview_legs, initial_preview_economics, opportunity_strategy,
    preview_target_base_quantity, ticket_plans, PreviewLegBuildInput, PreviewLegBuilds,
};
use axum::http::StatusCode;
use common::AppError;
pub(crate) use final_margin::apply_final_margin_guard;
#[cfg(test)]
pub(crate) use guards::append_order_capability_guard;
use guards::append_preview_guards;
pub(crate) use intent::{build_leg, compile_order_plan, execution_mode_for_adapter, LegBuild};
use intent::{leg_price, validate_preview};
use positions::append_positions_evidence_guard;
#[cfg(test)]
pub(crate) use positions::positions_evidence_guard;
use pricing::record_blocked_preview;
pub(crate) use pricing::{estimated_costs_usd, p0_strategy_from_arb_type, preview_funding_yield};
pub(crate) use readiness::validate_executable_opportunity;
use shared_types::hedge::HedgeTicketOrderPlans;
#[cfg(test)]
use shared_types::OrderType;
use shared_types::{
    problem::codes, venue_family, ArbitrageOpportunityDto, ArbitrageType, ExecutionMode,
    HedgeExecutionParams, HedgePreflightOperation, HedgePreflightScope, HedgePreflightStatus,
    HedgePreviewPositionsEvidence, HedgePreviewRequest, HedgePreviewResponse, HedgeTicket,
    ListStatus, MarginPreflightOutcome, MissReason, MissedOpportunity, PositionInfo, RiskDecision,
    StrategyKind, VenueOperationHealth, VenueOperationStatus, VenuePositionEnvelope,
};
use uuid::Uuid;

pub(crate) async fn build_preview(
    state: &AppState,
    id: String,
    mut req: HedgePreviewRequest,
) -> Result<HedgePreviewResponse, AppError> {
    validate_preview(&id, &mut req)?;
    let (opportunity_snapshot_id, opp) =
        find_opportunity(state, &id, req.opportunity_snapshot_id.as_deref()).await?;
    validate_executable_opportunity(&opp)?;
    let params = preview_params(&req);
    let notional = params.capital_usd * params.leverage;
    let long_notional = req.long_notional_usd.unwrap_or(notional);
    let short_notional = req.short_notional_usd.unwrap_or(notional);
    let mut ticket = crate::services::hedge_ticket::build_ticket(
        state,
        &opp,
        &params,
        long_notional,
        short_notional,
    )
    .await;
    let idempotency_key = format!("hedge-{}", Uuid::new_v4());
    let mode = execution_mode_for_adapter(state.trading_service().adapter_name());
    let strategy = opportunity_strategy(&opp);
    let long_price = preview_long_price(&req, &ticket, &opp)?;
    let short_price = preview_short_price(&req, &ticket, &opp)?;
    let target_base_quantity = preview_target_base_quantity(
        &ticket,
        (long_notional, short_notional, long_price, short_price),
    );
    let PreviewLegBuilds {
        mut long_leg,
        mut short_leg,
        mut long_order_plan,
        mut short_order_plan,
    } = build_preview_legs(&PreviewLegBuildInput {
        opportunity: &opp,
        key: &idempotency_key,
        params: &params,
        mode,
        strategy,
        quantity: target_base_quantity,
        prices: [long_price, short_price],
    });
    append_preview_guards(PreviewGuardInput {
        state,
        ticket: &mut ticket,
        mode,
        long_leg: &mut long_leg,
        short_leg: &mut short_leg,
        long_order_plan: &mut long_order_plan,
        short_order_plan: &mut short_order_plan,
        long_notional,
        short_notional,
        long_price,
        short_price,
    })
    .await?;
    ticket.sizing.target_base_quantity = Some(long_leg.quantity.min(short_leg.quantity));
    let (long_risk, short_risk) = state.trading_service().check_hedge(&long_leg, &short_leg);
    let economics =
        initial_preview_economics(&ticket, &long_order_plan, long_leg.quantity, long_price);
    let metrics = append_positions_evidence_guard(
        state,
        &mut ticket,
        mode,
        economics.funding_usd,
        &economics.costs,
        &[&long_leg, &short_leg],
    )
    .await;
    let ticket_order_plans = ticket_plans(&ticket, long_order_plan, short_order_plan)?;
    let workflow_view =
        workflow_view::project(&ticket, &ticket_order_plans, common::time::now_ms());
    let preview = HedgePreviewResponse {
        opportunity_id: id,
        opportunity_snapshot_id,
        ticket,
        workflow_view,
        ticket_order_plans: Some(ticket_order_plans),
        long_leg,
        long_risk,
        short_leg,
        short_risk,
        long_order_plan: None,
        short_order_plan: None,
        estimated_funding_next_settlement_usd: Some(economics.funding_usd),
        estimated_funding_per_8h_usd: economics.funding_usd,
        estimated_gross_edge_usd: economics.gross_edge_usd,
        estimated_open_cost_usd: economics.costs.open_usd,
        estimated_close_cost_usd: economics.costs.close_usd,
        estimated_slippage_usd: economics.costs.slippage_usd,
        current_account_liq_distance_pct: metrics.current_account_liq_distance_pct,
        after_hedge_liq_distance_pct: metrics.after_hedge_liq_distance_pct,
        positions_evidence: Some(metrics.positions_evidence),
        used_capital_usd: metrics.used_capital_usd,
        max_loss_usd: metrics.max_loss_usd,
        idempotency_key,
    };
    store_preview(state, &opp, &preview);
    Ok(preview)
}

fn store_preview(
    state: &AppState,
    opportunity: &ArbitrageOpportunityDto,
    preview: &HedgePreviewResponse,
) {
    persist_preview(state, preview);
    record_blocked_preview(state, opportunity, preview);
}

pub(crate) fn persist_preview(state: &AppState, preview: &HedgePreviewResponse) {
    state
        .hedge_tickets()
        .insert(preview.ticket.ticket_id.clone(), preview.ticket.clone());
    state
        .hedge_previews()
        .insert(preview.idempotency_key.clone(), preview.clone());
}

pub(crate) fn refresh_submit_market_projection(
    preview: &mut HedgePreviewResponse,
    checked_at_ms: i64,
) {
    let notional = preview_execution_notional(preview);
    let costs = estimated_costs_usd(&preview.ticket, notional);
    preview.estimated_open_cost_usd = costs.open_usd;
    preview.estimated_close_cost_usd = costs.close_usd;
    preview.estimated_slippage_usd = costs.slippage_usd;
    preview.estimated_gross_edge_usd = pricing::gross_edge_usd(&preview.ticket, notional);
    let funding_usd = notional * preview_funding_yield(&preview.ticket);
    preview.estimated_funding_next_settlement_usd = Some(funding_usd);
    preview.estimated_funding_per_8h_usd = funding_usd;
    if let Some(plans) = preview.ticket_order_plans.as_ref() {
        preview.workflow_view = workflow_view::project(&preview.ticket, plans, checked_at_ms);
    }
}

fn preview_execution_notional(preview: &HedgePreviewResponse) -> f64 {
    preview
        .ticket
        .long_leg
        .open_vwap_price
        .map(|price| preview.long_leg.quantity * price)
        .or_else(|| {
            preview
                .ticket_order_plans
                .as_ref()
                .and_then(|plans| plans.long.compile_plan.sizing_plan)
                .map(|plan| plan.actual_notional_usd)
        })
        .filter(|notional| notional.is_finite() && *notional > 0.0)
        .unwrap_or(preview.ticket.sizing.target_notional_usd)
}

struct PreviewGuardInput<'a> {
    state: &'a AppState,
    ticket: &'a mut HedgeTicket,
    mode: ExecutionMode,
    long_leg: &'a mut shared_types::OrderIntent,
    short_leg: &'a mut shared_types::OrderIntent,
    long_order_plan: &'a mut shared_types::OrderCompilePlan,
    short_order_plan: &'a mut shared_types::OrderCompilePlan,
    long_notional: f64,
    short_notional: f64,
    long_price: f64,
    short_price: f64,
}

pub(crate) fn preview_long_price(
    req: &HedgePreviewRequest,
    ticket: &HedgeTicket,
    opp: &ArbitrageOpportunityDto,
) -> Result<f64, AppError> {
    leg_price(
        ticket
            .long_leg
            .open_vwap_price
            .or(ticket.long_leg.reference_price)
            .or(req.long_price)
            .or(opp.long_price),
        "longPrice",
    )
}

pub(crate) fn preview_short_price(
    req: &HedgePreviewRequest,
    ticket: &HedgeTicket,
    opp: &ArbitrageOpportunityDto,
) -> Result<f64, AppError> {
    leg_price(
        ticket
            .short_leg
            .open_vwap_price
            .or(ticket.short_leg.reference_price)
            .or(req.short_price)
            .or(opp.short_price),
        "shortPrice",
    )
}

async fn find_opportunity(
    state: &AppState,
    id: &str,
    expected_snapshot_id: Option<&str>,
) -> Result<(String, ArbitrageOpportunityDto), AppError> {
    match state
        .opportunity_index()
        .get_bound(id, expected_snapshot_id)
    {
        Ok(Some(bound)) => Ok(bound),
        Ok(None) => Err(AppError::domain(
            StatusCode::NOT_FOUND,
            codes::OPPORTUNITY_EXPIRED,
            format!("opportunity expired: {id}"),
        )
        .with_details(serde_json::json!({ "opportunityId": id }))),
        Err(mismatch) => state.opportunity_index().current_bound(id).ok_or_else(|| {
            AppError::domain(
                StatusCode::NOT_FOUND,
                codes::OPPORTUNITY_EXPIRED,
                format!("opportunity expired: {id}"),
            )
            .with_details(serde_json::json!({
                "opportunityId": id,
                "expectedSnapshotId": mismatch.expected,
                "actualSnapshotId": mismatch.actual,
            }))
        }),
    }
}

fn preview_params(req: &HedgePreviewRequest) -> HedgeExecutionParams {
    let mut params = req.execution_params.clone().unwrap_or_default();
    params.capital_usd = req.capital_usd;
    params.leverage = req.leverage;
    params
}

#[cfg(test)]
pub(crate) use guards::{account_mode_plan, available_order_types};
#[cfg(test)]
pub(crate) use positions::{
    current_liquidation_distance_pct, preview_metrics_from_position_envelope, used_capital_usd,
};
#[cfg(test)]
pub(crate) use pricing::{blocked_detail, PreviewCosts};

#[cfg(test)]
#[path = "hedge_preview/snapshot_tests.rs"]
mod snapshot_tests;
