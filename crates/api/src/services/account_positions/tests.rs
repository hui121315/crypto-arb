#![allow(clippy::panic)]

use super::*;
use exchange::ExchangeError;

mod error_paths;
mod quality;
mod venue_quality;

#[test]
fn empty_rows_without_fresh_evidence_get_problem() {
    let envelope = VenuePositionEnvelope::new(
        Vec::new(),
        position_status(&[missing_position_evidence_problem()], &[], &[]),
        POSITION_SOURCE,
        1,
        vec![missing_position_evidence_problem()],
        Vec::new(),
    );

    assert_eq!(envelope.status, ListStatus::Degraded);
    assert!(envelope
        .problems
        .iter()
        .any(|problem| problem.code == codes::POSITION_EVIDENCE_MISSING));
}

#[test]
fn fresh_position_cache_health_is_not_degraded() {
    let row = VenueOperationHealth {
        venue: "mock".to_owned(),
        operation: ACCOUNT_CACHE_OPERATION.to_owned(),
        status: VenueOperationStatus::Ok,
        source: "account_cache".to_owned(),
        message: "fresh".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: None,
        rows: Some(1),
        freshness_ms: Some(10),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 1,
    };

    assert_eq!(position_status(&[], &[row], &[]), ListStatus::Fresh);
}

#[test]
fn unconfigured_venue_does_not_degrade_configured_position_data() {
    let mut row = operation_health_row(ACCOUNT_CACHE_OPERATION);
    row.status = VenueOperationStatus::Unknown;
    row.configured = Some(false);

    assert_eq!(position_status(&[], &[row], &[]), ListStatus::Fresh);
}

#[test]
fn position_operation_health_filters_prefetched_rows() {
    let rows = vec![
        operation_health_row(ACCOUNT_CACHE_OPERATION),
        operation_health_row(CREDENTIAL_POSITION_PROBE),
        operation_health_row("balance"),
        operation_health_row("credential_probe:open_orders_read"),
    ];

    let filtered = operation_health_from_rows(&rows);

    assert_eq!(filtered.len(), 2);
    assert!(filtered
        .iter()
        .any(|row| row.operation == ACCOUNT_CACHE_OPERATION));
    assert!(filtered
        .iter()
        .any(|row| row.operation == CREDENTIAL_POSITION_PROBE));
}

#[test]
fn scoped_position_health_excludes_unrequested_venue_failures() {
    let mut bitget = operation_health_row(ACCOUNT_CACHE_OPERATION);
    bitget.venue = "bitget".to_owned();
    let mut gate = operation_health_row(ACCOUNT_CACHE_OPERATION);
    gate.venue = "gate".to_owned();
    gate.status = VenueOperationStatus::Blocked;
    gate.error = Some("timeout after 3s".to_owned());

    let filtered = operation_health_from_rows_for_venues(&[bitget, gate], &["bitget".to_owned()]);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].venue, "bitget");
    assert_eq!(filtered[0].status, VenueOperationStatus::Ok);
}

#[test]
fn position_row_health_keeps_source_freshness_and_retry_context() {
    let rows = vec![PositionInfo {
        exchange: "Gate".to_owned(),
        ..position_row()
    }];
    let mut health = operation_health_row(ACCOUNT_CACHE_OPERATION);
    health.venue = "gate".to_owned();
    health.status = VenueOperationStatus::Warn;
    health.source = "position_cache".to_owned();
    health.freshness_ms = Some(1_500);
    health.retry_after_ms = Some(2_000);
    health.error = Some("rate limited".to_owned());
    health.observed_at_ms = 10_000;

    let row_health = position_row_health(&rows, &[health], &[], &[], 11_000);

    assert_eq!(row_health.len(), 1);
    assert_eq!(row_health[0].source, "position_cache");
    assert_eq!(row_health[0].freshness_ms, Some(1_500));
    assert_eq!(row_health[0].retry_after_ms, Some(2_000));
    assert_eq!(row_health[0].observed_at_ms, 10_000);
    assert!(row_health[0]
        .last_error
        .as_ref()
        .is_some_and(|problem| problem.code == codes::POSITION_READ_DEGRADED));
}

#[test]
fn public_mark_updates_liquidation_distance_without_rewriting_venue_pnl() {
    let mut row = PositionInfo {
        liquidation_price: Some(80.0),
        unrealized_pnl: 7.5,
        ..position_row()
    };

    apply_public_mark(&mut row, 100.0);

    assert_eq!(row.mark_price, 100.0);
    assert_eq!(row.liquidation_distance_pct, Some(20.0));
    assert_eq!(row.unrealized_pnl, 7.5);
}

fn position_row() -> PositionInfo {
    PositionInfo {
        symbol: "BTCUSDT".to_owned(),
        exchange: "binance".to_owned(),
        side: "long".to_owned(),
        quantity: 1.0,
        entry_price: 10.0,
        mark_price: 11.0,
        unrealized_pnl: 1.0,
        leverage: 1.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 10.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

fn operation_health_row(operation: &str) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: "mock".to_owned(),
        operation: operation.to_owned(),
        status: VenueOperationStatus::Ok,
        source: "test".to_owned(),
        message: "test".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: None,
        rows: Some(1),
        freshness_ms: Some(10),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 1,
    }
}
