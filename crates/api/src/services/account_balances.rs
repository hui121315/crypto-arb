use crate::services::{account_binding, trading_credentials, venue_operation_health};
use crate::state::AppState;
use crate::trading_service::private_ws_events::PrivateAccountScope;
use crate::trading_service::{AccountCacheQuality, RouteFailure};
use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    normalized_venue_name, problem::codes, AccountDataHealth, AccountFieldQuality,
    AccountFieldQualityStatus, AccountFieldSubject, ApiProblem, ListStatus, VenueBalanceEnvelope,
    VenueBalanceInfo, VenueOperationHealth, VenueOperationStatus,
};

const BALANCE_SOURCE: &str = "account_balance_runtime";
const BALANCE_ROUTE: &str = "/api/trading/balances";
const BALANCE_OPERATION: &str = "balances";
const ACCOUNT_CACHE_OPERATION: &str = "balance";
const ACCOUNT_CACHE_SOURCE: &str = "account_cache";
const CREDENTIAL_BALANCE_PROBE: &str = "credential_probe:balance_read";

mod health;
mod problems;

use health::{balance_field_quality, balance_row_health};
use problems::{
    balance_read_problem, balance_status, has_fresh_balance_evidence,
    missing_balance_evidence_problem, route_failure_problems,
};

pub(crate) async fn envelope(state: &AppState) -> VenueBalanceEnvelope {
    state
        .trading_service()
        .schedule_hyperliquid_account_evidence_refresh();
    let operation_health = operation_health(state);
    envelope_with_operation_health(state, operation_health).await
}

pub(crate) async fn envelope_with_operation_health(
    state: &AppState,
    operation_health: Vec<VenueOperationHealth>,
) -> VenueBalanceEnvelope {
    let observed_at_ms = common::time::now_ms();
    let mut problems = Vec::new();
    let rows = read_balance_rows(state, &mut problems).await;
    state
        .trading_service()
        .schedule_account_evidence_refresh(&rows);
    record_fresh_balance_cache_refreshes(state);
    let operation_health = refresh_operation_health_after_read(state, operation_health);
    let account_summaries = state.trading_service().account_summaries();
    let asset_valuations = state.trading_service().asset_valuations_for_rows(&rows);
    let route_failures = take_route_failures(state);
    build_envelope(
        observed_at_ms,
        rows,
        operation_health,
        &route_failures,
        &account_summaries,
        problems,
    )
    .with_account_summaries(account_summaries)
    .with_asset_valuations(asset_valuations)
}

fn refresh_operation_health_after_read(
    state: &AppState,
    rows: Vec<VenueOperationHealth>,
) -> Vec<VenueOperationHealth> {
    if rows.iter().any(configured_cache_needs_refresh) {
        operation_health(state)
    } else {
        rows
    }
}

fn configured_cache_needs_refresh(row: &VenueOperationHealth) -> bool {
    row.operation == ACCOUNT_CACHE_OPERATION
        && row.configured == Some(true)
        && row.status != VenueOperationStatus::Ok
}

fn build_envelope(
    observed_at_ms: i64,
    rows: Vec<VenueBalanceInfo>,
    operation_health: Vec<VenueOperationHealth>,
    route_failures: &[RouteFailure],
    account_summaries: &[shared_types::VenueAccountSummary],
    mut problems: Vec<ApiProblem>,
) -> VenueBalanceEnvelope {
    let route_failure_problems = route_failure_problems(route_failures);
    problems.extend(
        route_failures
            .iter()
            .zip(&route_failure_problems)
            .filter(|(failure, _)| !usable_cache_covers_failure(failure, &operation_health))
            .map(|(_, problem)| problem.clone()),
    );
    if rows.is_empty() && !has_fresh_balance_evidence(&operation_health) {
        problems.push(missing_balance_evidence_problem());
    }
    let field_quality = balance_field_quality(&rows, observed_at_ms);
    let row_health = balance_row_health(
        &rows,
        &operation_health,
        route_failures,
        &route_failure_problems,
        observed_at_ms,
    );
    let status = balance_status(&problems, &operation_health, &field_quality);
    let account_bindings = account_binding::evidence_for_venues_with_summaries(
        rows.iter()
            .map(|row| row.venue.clone())
            .chain(
                operation_health
                    .iter()
                    .filter(|row| row.configured != Some(false))
                    .map(|row| row.venue.clone()),
            )
            .chain(route_failures.iter().map(|row| row.venue.clone())),
        account_summaries,
        observed_at_ms,
    );
    VenueBalanceEnvelope::new(
        rows,
        status,
        BALANCE_SOURCE,
        observed_at_ms,
        problems,
        operation_health,
    )
    .with_field_quality(field_quality)
    .with_row_health(row_health)
    .with_account_bindings(account_bindings)
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

async fn read_balance_rows(
    state: &AppState,
    problems: &mut Vec<ApiProblem>,
) -> Vec<VenueBalanceInfo> {
    match state
        .trading_service()
        .list_configured_balances_low_latency(trading_credentials::current_adapter_credentials())
        .await
    {
        Ok(rows) => rows,
        Err(error) => {
            let error = AppError::from(error);
            problems.push(balance_read_problem(&error));
            Vec::new()
        }
    }
}

fn record_fresh_balance_cache_refreshes(state: &AppState) {
    for cache in state
        .trading_service()
        .balance_cache_health()
        .into_iter()
        .filter(|cache| cache.quality == AccountCacheQuality::Fresh)
    {
        state
            .private_ws_health()
            .record_account_cache_refreshed(&cache.venue, PrivateAccountScope::Balances);
    }
}

pub(crate) fn operation_health(state: &AppState) -> Vec<VenueOperationHealth> {
    operation_health_from_rows(&venue_operation_health::snapshot(state).rows)
}

pub(crate) fn operation_health_from_rows(
    rows: &[VenueOperationHealth],
) -> Vec<VenueOperationHealth> {
    rows.iter()
        .filter(|row| is_balance_evidence_row(row))
        .cloned()
        .collect()
}

fn is_balance_evidence_row(row: &VenueOperationHealth) -> bool {
    row.operation == ACCOUNT_CACHE_OPERATION || row.operation == CREDENTIAL_BALANCE_PROBE
}

fn take_route_failures(state: &AppState) -> Vec<RouteFailure> {
    state
        .trading_service()
        .take_route_failures(BALANCE_OPERATION)
}

#[cfg(test)]
mod tests;
