#![allow(clippy::panic)]

use super::*;
use exchange::ExchangeError;

#[test]
fn empty_rows_without_fresh_evidence_get_problem() {
    let envelope = VenueBalanceEnvelope::new(
        Vec::new(),
        balance_status(&[missing_balance_evidence_problem()], &[], &[]),
        BALANCE_SOURCE,
        1,
        vec![missing_balance_evidence_problem()],
        Vec::new(),
    );

    assert_eq!(envelope.status, ListStatus::Degraded);
    assert!(envelope
        .problems
        .iter()
        .any(|problem| problem.code == codes::BALANCE_EVIDENCE_MISSING));
}

#[test]
fn fresh_account_cache_health_is_not_degraded() {
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

    assert_eq!(balance_status(&[], &[row], &[]), ListStatus::Fresh);
}

#[test]
fn unconfigured_venue_does_not_degrade_configured_balance_data() {
    let mut row = operation_health_row(ACCOUNT_CACHE_OPERATION);
    row.status = VenueOperationStatus::Blocked;
    row.configured = Some(false);

    assert_eq!(balance_status(&[], &[row], &[]), ListStatus::Fresh);
}

#[test]
fn balance_operation_health_filters_prefetched_rows() {
    let rows = vec![
        operation_health_row(ACCOUNT_CACHE_OPERATION),
        operation_health_row(CREDENTIAL_BALANCE_PROBE),
        operation_health_row("positions"),
        operation_health_row("credential_probe:open_orders_read"),
    ];

    let filtered = operation_health_from_rows(&rows);

    assert_eq!(filtered.len(), 2);
    assert!(filtered
        .iter()
        .any(|row| row.operation == ACCOUNT_CACHE_OPERATION));
    assert!(filtered
        .iter()
        .any(|row| row.operation == CREDENTIAL_BALANCE_PROBE));
}

#[test]
fn partial_fanout_keeps_rows_and_surfaces_route_problem() {
    let envelope = build_envelope(
        1,
        vec![VenueBalanceInfo {
            venue: "binance".to_owned(),
            currency: "USDT".to_owned(),
            total: 100.0,
            available: 90.0,
            frozen: 10.0,
            unrealized_pnl: 0.0,
        }],
        Vec::new(),
        &[RouteFailure::new(
            "gate".to_owned(),
            BALANCE_OPERATION,
            ExchangeError::Network("balance route failed".to_owned()),
        )],
        &[],
        Vec::new(),
    );

    assert_eq!(envelope.status, ListStatus::Degraded);
    assert_eq!(envelope.rows.len(), 1);
    assert!(envelope.field_quality.iter().any(|row| {
        row.field == "available" && row.status == AccountFieldQualityStatus::Actual
    }));
    assert_eq!(envelope.rows[0].venue, "binance");
    assert_eq!(envelope.account_bindings.len(), 2);
    assert!(envelope
        .account_bindings
        .iter()
        .any(|binding| binding.venue == "binance"));
    assert!(envelope
        .account_bindings
        .iter()
        .any(|binding| binding.venue == "gate"));
    let Some(problem) = envelope.problems.iter().find(|problem| {
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("venue"))
            .and_then(serde_json::Value::as_str)
            == Some("gate")
    }) else {
        panic!("gate route failure should be exposed");
    };
    assert_eq!(problem.source.as_deref(), Some(BALANCE_SOURCE));
    assert_eq!(problem.status, Some(502));
    let Some(account_health) = envelope.row_health.iter().find(|row| {
        row.subject.kind == shared_types::AccountFieldSubjectKind::Account
            && row.subject.venue.as_deref() == Some("gate")
    }) else {
        panic!("gate route failure should expose account-level health");
    };
    assert_eq!(account_health.source, BALANCE_SOURCE);
    assert_eq!(
        account_health
            .last_error
            .as_ref()
            .map(|problem| problem.status),
        Some(Some(502))
    );
}

#[test]
fn bounded_cached_balance_keeps_transient_route_failure_at_row_level() {
    let route_failures = vec![RouteFailure::new(
        "binance".to_owned(),
        BALANCE_OPERATION,
        ExchangeError::Timeout { seconds: 3 },
    )];
    let mut cache_health = operation_health_row(ACCOUNT_CACHE_OPERATION);
    cache_health.venue = "binance".to_owned();
    cache_health.source = ACCOUNT_CACHE_SOURCE.to_owned();

    let envelope = build_envelope(
        1,
        vec![VenueBalanceInfo {
            venue: "binance".to_owned(),
            currency: "USDT".to_owned(),
            total: 100.0,
            available: 90.0,
            frozen: 10.0,
            unrealized_pnl: 0.0,
        }],
        vec![cache_health],
        &route_failures,
        &[],
        Vec::new(),
    );

    assert_eq!(envelope.status, ListStatus::Fresh);
    assert!(envelope.problems.is_empty());
    assert!(envelope.row_health.iter().any(|health| {
        health.subject.venue.as_deref() == Some("binance") && health.last_error.is_some()
    }));
}

#[test]
fn balance_field_quality_marks_invalid_numbers() {
    let rows = vec![VenueBalanceInfo {
        venue: "binance".to_owned(),
        currency: "USDT".to_owned(),
        total: f64::NAN,
        available: 90.0,
        frozen: 10.0,
        unrealized_pnl: 0.0,
    }];

    let quality = balance_field_quality(&rows, 10);

    assert!(quality.iter().any(|row| {
        row.field == "total"
            && row.status == AccountFieldQualityStatus::Invalid
            && row
                .problem
                .as_ref()
                .is_some_and(|problem| problem.code == codes::BALANCE_FIELD_UNAVAILABLE)
    }));
    assert_eq!(balance_status(&[], &[], &quality), ListStatus::Degraded);
}

#[test]
fn balance_row_health_keeps_source_freshness_and_retry_context() {
    let rows = vec![VenueBalanceInfo {
        venue: "Gate".to_owned(),
        currency: "USDT".to_owned(),
        total: 100.0,
        available: 90.0,
        frozen: 10.0,
        unrealized_pnl: 0.0,
    }];
    let mut health = operation_health_row(ACCOUNT_CACHE_OPERATION);
    health.venue = "gate".to_owned();
    health.status = VenueOperationStatus::Warn;
    health.source = "account_cache".to_owned();
    health.freshness_ms = Some(1_500);
    health.retry_after_ms = Some(2_000);
    health.error = Some("rate limited".to_owned());
    health.observed_at_ms = 10_000;

    let row_health = balance_row_health(&rows, &[health], &[], &[], 11_000);

    assert_eq!(row_health.len(), 1);
    assert_eq!(row_health[0].source, "account_cache");
    assert_eq!(row_health[0].freshness_ms, Some(1_500));
    assert_eq!(row_health[0].retry_after_ms, Some(2_000));
    assert_eq!(row_health[0].observed_at_ms, 10_000);
    assert!(row_health[0]
        .last_error
        .as_ref()
        .is_some_and(|problem| problem.code == codes::BALANCE_READ_DEGRADED));
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
