// Imported only for the sibling `hedge_tests` module (consumed via `use super::*`).
#[cfg(test)]
#[allow(unused_imports)]
use crate::services::hedge_confirm::{
    confirm_action_problem, confirm_action_status, finish_confirm_action, replay_confirm_response,
};
#[cfg(test)]
#[allow(unused_imports)]
use crate::services::hedge_confirm::{ticket_ready, validate_ticket_ready};
#[cfg(test)]
#[allow(unused_imports)]
use crate::services::hedge_preview::{
    account_mode_plan, append_order_capability_guard, available_order_types, blocked_detail,
    build_leg, compile_order_plan, current_liquidation_distance_pct, estimated_costs_usd,
    execution_mode_for_adapter, p0_strategy_from_arb_type, positions_evidence_guard,
    preview_funding_yield, preview_long_price, preview_metrics_from_position_envelope,
    preview_short_price, used_capital_usd, validate_executable_opportunity, LegBuild, PreviewCosts,
};

#[cfg(test)]
use crate::services::action_runs;
use crate::services::action_runs::ActionRunStart;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
#[cfg(test)]
use axum::http::StatusCode;
use axum::Json;
use common::AppError;
#[cfg(test)]
use exchange::ExchangeCapabilities;
#[cfg(test)]
use shared_types::{problem::codes, ActionRun, ActionRunStatus, ExecutionMode, HedgeTicket};
use shared_types::{
    ActionRunKind, HedgeConfirmRequest, HedgeConfirmResponse, HedgePreviewRequest,
    HedgePreviewResponse,
};
#[cfg(test)]
use shared_types::{ArbitrageOpportunityDto, ListStatus, RiskDecision, VenuePositionEnvelope};

pub(super) async fn preview_hedge(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<HedgePreviewRequest>,
) -> Result<Json<HedgePreviewResponse>, AppError> {
    crate::services::hedge_preview::build_preview(&state, id, req)
        .await
        .map(Json)
}

pub(super) async fn confirm_hedge(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<String>,
    Json(req): Json<HedgeConfirmRequest>,
) -> Result<Json<HedgeConfirmResponse>, AppError> {
    let start = ActionRunStart::new(
        ActionRunKind::HedgeConfirm,
        &headers,
        Some(id.clone()),
        "hedge confirm accepted",
    )
    .with_idempotency_key(Some(req.idempotency_key.clone()));
    crate::services::hedge_confirm::confirm(&state, id, req, start)
        .await
        .map(Json)
}

#[cfg(test)]
#[path = "hedge_tests.rs"]
mod tests;
