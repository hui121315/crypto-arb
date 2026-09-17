use crate::services::{account_binding, account_quality, venue_operation_health};
use crate::state::AppState;
use crate::trading_service::private_ws_events::PrivateAccountScope;
use crate::trading_service::{AccountCacheQuality, RouteFailure};
use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    normalized_venue_name, problem::codes, AccountFieldQuality, AccountFieldQualityStatus,
    AccountFieldSubject, ApiProblem, ListStatus, PositionInfo, VenueOperationHealth,
    VenueOperationStatus, VenuePositionEnvelope,
};

const POSITION_SOURCE: &str = "account_position_runtime";
const POSITION_ROUTE: &str = "/api/trading/positions";
const POSITION_OPERATION: &str = "positions";
const ACCOUNT_CACHE_OPERATION: &str = "positions";
const ACCOUNT_CACHE_SOURCE: &str = "account_cache";
const CREDENTIAL_POSITION_PROBE: &str = "credential_probe:positions_read";

mod assembly;
mod health;
mod projection;
mod public_marks;
mod scope;

use assembly::build_envelope;
#[cfg(test)]
use health::position_last_success_ms;
use health::position_row_health;
use projection::position_field_quality;
use public_marks::apply_fresh_public_marks;
#[cfg(test)]
use public_marks::apply_public_mark;
use scope::{
    operation_health_for_venues, position_scope_venues, refresh_operation_health_after_read,
};

#[cfg(test)]
use scope::operation_health_from_rows_for_venues;

pub(crate) async fn envelope(state: &AppState) -> VenuePositionEnvelope {
    state
        .trading_service()
        .schedule_hyperliquid_account_evidence_refresh();
    let operation_health = operation_health(state);
    envelope_with_scope(state, operation_health, None).await
}

pub(crate) async fn envelope_for_venues(
    state: &AppState,
    venues: &[String],
) -> VenuePositionEnvelope {
    let venues = position_scope_venues(venues);
    if venues.iter().any(|venue| venue == "hyperliquid") {
        state
            .trading_service()
            .schedule_hyperliquid_account_evidence_refresh();
    }
    let operation_health = operation_health_for_venues(state, &venues);
    envelope_with_scope(state, operation_health, Some(&venues)).await
}

pub(crate) async fn envelope_with_operation_health(
    state: &AppState,
    operation_health: Vec<VenueOperationHealth>,
) -> VenuePositionEnvelope {
    envelope_with_scope(state, operation_health, None).await
}

async fn envelope_with_scope(
    state: &AppState,
    operation_health: Vec<VenueOperationHealth>,
    venues: Option<&[String]>,
) -> VenuePositionEnvelope {
    let observed_at_ms = common::time::now_ms();
    let mut problems = Vec::new();
    let mut rows = match venues {
        Some(venues) => read_position_rows_for_venues(state, venues, &mut problems).await,
        None => read_position_rows(state, &mut problems).await,
    };
    let mark_field_quality = apply_fresh_public_marks(state, &mut rows, observed_at_ms);
    record_fresh_position_cache_refreshes(state);
    let operation_health = refresh_operation_health_after_read(state, operation_health, venues);
    let route_failures = match venues {
        Some(venues) => take_route_failures_for_venues(state, venues),
        None => take_route_failures(state),
    };
    let account_summaries = state.trading_service().account_summaries();
    build_envelope(
        observed_at_ms,
        rows,
        operation_health,
        (&route_failures, &account_summaries),
        mark_field_quality,
        problems,
    )
}

fn usable_cache_covers_failure(
    failure: &RouteFailure,
    operation_health: &[VenueOperationHealth],
) -> bool {
    let venue = normalized_venue_name(&failure.venue);
    operation_health.iter().any(|row| {
        normalized_venue_name(&row.venue) == venue
            && row.operation == ACCOUNT_CACHE_OPERATION
            && row.source == ACCOUNT_CACHE_SOURCE
            && row.status == VenueOperationStatus::Ok
    })
}

pub(crate) fn operation_health(state: &AppState) -> Vec<VenueOperationHealth> {
    operation_health_from_rows(&venue_operation_health::snapshot(state).rows)
}

pub(crate) fn operation_health_from_rows(
    rows: &[VenueOperationHealth],
) -> Vec<VenueOperationHealth> {
    rows.iter()
        .filter(|row| is_position_evidence_row(row))
        .cloned()
        .collect()
}

async fn read_position_rows(state: &AppState, problems: &mut Vec<ApiProblem>) -> Vec<PositionInfo> {
    match state.trading_service().list_positions_low_latency().await {
        Ok(rows) => rows,
        Err(error) => {
            let error = AppError::from(error);
            problems.push(position_read_problem(&error));
            Vec::new()
        }
    }
}

async fn read_position_rows_for_venues(
    state: &AppState,
    venues: &[String],
    problems: &mut Vec<ApiProblem>,
) -> Vec<PositionInfo> {
    match state
        .trading_service()
        .list_scoped_positions_low_latency(venues)
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            let error = AppError::from(error);
            problems.push(position_read_problem(&error));
            Vec::new()
        }
    }
}

fn record_fresh_position_cache_refreshes(state: &AppState) {
    for cache in state
        .trading_service()
        .position_cache_health()
        .into_iter()
        .filter(|cache| cache.quality == AccountCacheQuality::Fresh)
    {
        state
            .private_ws_health()
            .record_account_cache_refreshed(&cache.venue, PrivateAccountScope::Positions);
    }
}

fn is_position_evidence_row(row: &VenueOperationHealth) -> bool {
    row.operation == ACCOUNT_CACHE_OPERATION || row.operation == CREDENTIAL_POSITION_PROBE
}

fn take_route_failures(state: &AppState) -> Vec<RouteFailure> {
    state
        .trading_service()
        .take_route_failures(POSITION_OPERATION)
}

fn take_route_failures_for_venues(state: &AppState, venues: &[String]) -> Vec<RouteFailure> {
    state
        .trading_service()
        .take_route_failures_for_venues(POSITION_OPERATION, venues)
}

fn route_failure_problems(route_failures: &[RouteFailure]) -> Vec<ApiProblem> {
    route_failures.iter().map(route_failure_problem).collect()
}

fn route_failure_problem(failure: &RouteFailure) -> ApiProblem {
    failure.to_api_problem(POSITION_SOURCE, POSITION_ROUTE)
}

fn position_read_problem(error: &AppError) -> ApiProblem {
    let mut problem = error.to_api_problem().with_source(POSITION_SOURCE);
    problem.details = Some(serde_json::json!({
        "operation": POSITION_OPERATION,
        "path": POSITION_ROUTE,
        "status": error.status().as_u16(),
        "source": POSITION_SOURCE,
    }));
    problem
}

fn missing_position_evidence_problem() -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::POSITION_EVIDENCE_MISSING,
        "position rows are empty and no fresh position evidence is available",
    )
    .with_status(StatusCode::OK.as_u16())
    .with_request_id(common::request_id::current())
    .with_source(POSITION_SOURCE);
    problem.details = Some(serde_json::json!({
        "operation": POSITION_OPERATION,
        "path": POSITION_ROUTE,
        "source": POSITION_SOURCE,
    }));
    problem
}

fn position_status(
    problems: &[ApiProblem],
    operation_health: &[VenueOperationHealth],
    field_quality: &[AccountFieldQuality],
) -> ListStatus {
    if !problems.is_empty()
        || operation_health
            .iter()
            .any(account_quality::account_data_operation_degrades_snapshot)
        || account_quality::account_fields_degrade_snapshot(field_quality)
    {
        ListStatus::Degraded
    } else {
        ListStatus::Fresh
    }
}

fn has_fresh_position_evidence(operation_health: &[VenueOperationHealth]) -> bool {
    operation_health.iter().any(|row| {
        row.status == VenueOperationStatus::Ok
            && (row.operation == ACCOUNT_CACHE_OPERATION
                || row.operation == CREDENTIAL_POSITION_PROBE)
    })
}

#[cfg(test)]
mod tests;
