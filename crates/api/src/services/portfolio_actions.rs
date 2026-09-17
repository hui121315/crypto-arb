use crate::services::portfolio;
use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use serde_json::json;
use shared_types::{
    normalized_venue_name, ApiProblem, CloseAllPositionsRequest, CloseLeg, CloseLegStatus,
    ClosePositionRequest, CloseRun, CloseRunScope, CloseRunStatus, ExecutionMode, FeeProduct,
    HedgeLegRole, MarginMode, OrderCompilePlan, OrderIntent, OrderPayloadPricePolicy, OrderRecord,
    OrderSide, OrderSource, OrderType, PositionRow, PositionSide, TimeInForce, VenueOrderKind,
    VenueSymbolCapability, CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE,
};
use uuid::Uuid;

const CLOSE_REASON_MAX_CHARS: usize = 160;

#[derive(Debug, Clone)]
pub(crate) struct CloseRequestContext {
    snapshot_version: String,
    expected_leg_count: usize,
    reason: Option<String>,
    idempotency_key: Option<String>,
    scope: CloseRunScope,
}

impl CloseRequestContext {
    pub(crate) fn from_position_request(payload: &ClosePositionRequest) -> Result<Self, AppError> {
        Self::new(
            payload.snapshot_version.clone(),
            payload.expected_leg_count,
            payload.reason.clone(),
        )
    }

    pub(crate) fn from_all_request(payload: &CloseAllPositionsRequest) -> Result<Self, AppError> {
        Self::new(
            payload.snapshot_version.clone(),
            payload.expected_leg_count,
            payload.reason.clone(),
        )
    }

    fn new(
        snapshot_version: Option<String>,
        expected_leg_count: Option<usize>,
        reason: Option<String>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            snapshot_version: required_snapshot_version(snapshot_version)?,
            expected_leg_count: required_expected_leg_count(expected_leg_count)?,
            reason: normalize_reason(reason)?,
            idempotency_key: None,
            scope: CloseRunScope::Single,
        })
    }

    pub(crate) fn with_idempotency_key(mut self, key: String) -> Self {
        self.idempotency_key = Some(key);
        self
    }

    fn with_scope(mut self, scope: CloseRunScope) -> Self {
        self.scope = scope;
        self
    }
}

pub(crate) async fn close_position(
    state: &AppState,
    venue: &str,
    symbol: &str,
    side: Option<PositionSide>,
    context: CloseRequestContext,
) -> Result<CloseRun, AppError> {
    let rows = portfolio::positions(state).await?;
    let context = validate_close_context(&rows, 1, context.with_scope(CloseRunScope::Single))?;
    let row = select_position(&rows, venue, symbol, side)?;
    let started_at_ms = common::time::now_ms();
    let leg = submit_close_leg(state, row, &context, 0).await;
    Ok(close_run(
        CloseRunScope::Single,
        vec![leg],
        started_at_ms,
        context,
    ))
}

pub(crate) async fn close_position_pair(
    state: &AppState,
    venue: &str,
    symbol: &str,
    side: Option<PositionSide>,
    context: CloseRequestContext,
) -> Result<CloseRun, AppError> {
    let rows = portfolio::positions(state).await?;
    let context = validate_close_context(&rows, 2, context.with_scope(CloseRunScope::Pair))?;
    let (row, paired) = select_pair_positions(&rows, venue, symbol, side)?;
    let started_at_ms = common::time::now_ms();
    let (first, second) = tokio::join!(
        submit_close_leg(state, row, &context, 0),
        submit_close_leg(state, paired, &context, 1)
    );
    Ok(close_run(
        CloseRunScope::Pair,
        vec![first, second],
        started_at_ms,
        context,
    ))
}

pub(crate) async fn close_all_positions(
    state: &AppState,
    confirmation_phrase: &str,
    context: CloseRequestContext,
) -> Result<CloseRun, AppError> {
    if confirmation_phrase.trim() != CLOSE_ALL_POSITIONS_CONFIRMATION_PHRASE {
        return Err(AppError::BadRequest(
            "invalid close-all confirmation phrase".into(),
        ));
    }

    let rows = portfolio::positions(state).await?;
    let context =
        validate_close_context(&rows, rows.len(), context.with_scope(CloseRunScope::All))?;
    let started_at_ms = common::time::now_ms();
    let mut legs = Vec::with_capacity(rows.len());
    for (index, row) in rows.iter().enumerate() {
        legs.push(submit_close_leg(state, row, &context, index).await);
    }
    Ok(close_run(CloseRunScope::All, legs, started_at_ms, context))
}

mod intent;
mod problems;
mod request;
mod run;
#[cfg(test)]
mod tests;

use request::{
    normalize_reason, required_expected_leg_count, required_snapshot_version,
    select_pair_positions, select_position, validate_close_context,
};
use run::{close_run, submit_close_leg};
