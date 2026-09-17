#![allow(clippy::panic)]

use super::*;
use exchange::ExchangeError;

#[test]
fn empty_rows_without_fresh_evidence_get_problem() {
    let problems = vec![missing_open_order_evidence_problem()];
    let envelope = VenueOpenOrdersEnvelope::new(
        Vec::new(),
        open_order_status(&problems, &[], &[]),
        OPEN_ORDERS_SOURCE,
        1,
        problems,
        Vec::new(),
    );

    assert_eq!(envelope.status, ListStatus::Degraded);
    assert!(envelope
        .problems
        .iter()
        .any(|problem| problem.code == codes::OPEN_ORDER_EVIDENCE_MISSING));
}

#[test]
fn fresh_open_order_probe_is_not_degraded() {
    let row = operation_health_row(CREDENTIAL_OPEN_ORDERS_PROBE, VenueOperationStatus::Ok);

    assert_eq!(open_order_status(&[], &[row], &[]), ListStatus::Fresh);
}

#[test]
fn unconfigured_venue_does_not_degrade_configured_open_orders() {
    let mut row = operation_health_row(CREDENTIAL_OPEN_ORDERS_PROBE, VenueOperationStatus::Blocked);
    row.configured = Some(false);

    assert_eq!(open_order_status(&[], &[row], &[]), ListStatus::Fresh);
}

#[test]
fn empty_rows_with_fresh_probe_have_no_missing_problem() {
    let operation_health = vec![operation_health_row(
        CREDENTIAL_OPEN_ORDERS_PROBE,
        VenueOperationStatus::Ok,
    )];

    assert!(has_fresh_open_order_evidence(&operation_health));
    assert_eq!(
        open_order_status(&[], &operation_health, &[]),
        ListStatus::Fresh
    );
}

#[test]
fn successful_empty_runtime_read_is_fresh_evidence() {
    assert!(!missing_open_order_evidence_needed(&[], true, &[]));
}

#[test]
fn failed_empty_runtime_read_still_requires_evidence() {
    assert!(missing_open_order_evidence_needed(&[], false, &[]));
}

#[test]
fn warming_open_orders_are_typed_and_retryable() {
    let envelope = warming_envelope(Vec::new());

    assert_eq!(envelope.status, ListStatus::Degraded);
    assert_eq!(envelope.row_count, 0);
    assert!(envelope.problems.iter().any(|problem| {
        problem.code == codes::OPEN_ORDER_EVIDENCE_MISSING
            && problem.retry_after_ms == Some(OPEN_ORDERS_REFRESH_RETRY_AFTER_MS)
    }));
}

#[test]
fn transient_open_order_refresh_failure_keeps_stale_while_revalidate_path() {
    let timeout = ApiProblem::new("TIMEOUT", "timeout after 3s").with_status(504);
    let envelope = VenueOpenOrdersEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        OPEN_ORDERS_SOURCE,
        1,
        vec![timeout],
        Vec::new(),
    );

    assert!(transient_refresh_failure(&envelope));
    assert_eq!(OPEN_ORDERS_REFRESH_INTERVAL_MS, 30_000);
}

#[test]
fn consumed_private_ws_change_does_not_spin_refresh_during_route_failure() {
    assert!(!refresh_needed(2_000, 42, 42));
    assert!(refresh_needed(2_000, 42, 43));
    assert!(refresh_needed(OPEN_ORDERS_REFRESH_INTERVAL_MS, 43, 43));
}

#[test]
fn open_order_operation_health_filters_unrelated_rows() {
    let rows = vec![
        operation_health_row(CREDENTIAL_OPEN_ORDERS_PROBE, VenueOperationStatus::Ok),
        operation_health_row("credential_probe:positions_read", VenueOperationStatus::Ok),
        operation_health_row("balance", VenueOperationStatus::Ok),
    ];

    let filtered = operation_health_from_rows(&rows);

    assert_eq!(filtered.len(), 1);
    assert!(filtered
        .iter()
        .all(|row| row.operation == CREDENTIAL_OPEN_ORDERS_PROBE));
}

#[test]
fn open_order_probe_attention_degrades_status() {
    let row = operation_health_row(CREDENTIAL_OPEN_ORDERS_PROBE, VenueOperationStatus::Warn);

    assert_eq!(open_order_status(&[], &[row], &[]), ListStatus::Degraded);
}

#[test]
fn open_order_field_quality_marks_row_level_problems() {
    let mut row = open_order_row("Gate");
    row.price = 0.0;
    row.filled_quantity = 2.0;
    row.fees = f64::NAN;

    let quality = open_order_field_quality(&[row], 10);

    assert!(quality
        .iter()
        .any(|row| { row.field == "price" && row.status == AccountFieldQualityStatus::Missing }));
    assert!(quality.iter().any(|row| {
        row.field == "filledQuantity" && row.status == AccountFieldQualityStatus::Invalid
    }));
    assert!(quality.iter().any(|row| {
        row.field == "fees"
            && row
                .problem
                .as_ref()
                .is_some_and(|problem| problem.code == codes::OPEN_ORDER_FIELD_UNAVAILABLE)
    }));
    assert_eq!(open_order_status(&[], &[], &quality), ListStatus::Degraded);
}

#[test]
fn open_order_field_quality_exposes_available_order_semantics_without_degrading() {
    let quality = open_order_field_quality(&[open_order_row("Gate")], 10);

    assert_eq!(quality.len(), 3);
    assert!(quality
        .iter()
        .all(|row| row.status == AccountFieldQualityStatus::Actual));
    assert!(quality.iter().any(|row| row.field == "clientOrderId"));
    assert!(quality.iter().any(|row| row.field == "venueTimeInForce"));
    assert!(quality.iter().any(|row| row.field == "reduceOnly"));
    assert_eq!(open_order_status(&[], &[], &quality), ListStatus::Fresh);
}

#[test]
fn open_order_row_health_keeps_source_freshness_and_retry_context() {
    let rows = vec![open_order_row("Gate")];
    let mut health = operation_health_row(CREDENTIAL_OPEN_ORDERS_PROBE, VenueOperationStatus::Warn);
    health.venue = "gate".to_owned();
    health.source = "open_order_probe".to_owned();
    health.freshness_ms = Some(1_500);
    health.retry_after_ms = Some(2_000);
    health.error = Some("rate limited".to_owned());
    health.observed_at_ms = 10_000;

    let row_health = open_order_row_health(&rows, &[health], &[], &[], 11_000);

    assert_eq!(row_health.len(), 1);
    assert_eq!(row_health[0].source, "open_order_probe");
    assert_eq!(row_health[0].freshness_ms, Some(1_500));
    assert_eq!(row_health[0].retry_after_ms, Some(2_000));
    assert_eq!(row_health[0].observed_at_ms, 10_000);
    assert!(row_health[0]
        .last_error
        .as_ref()
        .is_some_and(|problem| problem.code == codes::OPEN_ORDER_READ_DEGRADED));
    assert_eq!(row_health[0].subject.order_id.as_deref(), Some("o-1"));
}

#[test]
fn partial_open_order_fanout_keeps_rows_and_surfaces_route_health() {
    let route_failures = vec![RouteFailure::new(
        "gate".to_owned(),
        OPEN_ORDERS_OPERATION,
        ExchangeError::Network("open orders route failed".to_owned()),
    )];
    let route_problems = route_failure_problems(&route_failures);

    let row_health = open_order_row_health(
        &[open_order_row("binance")],
        &[],
        &route_failures,
        &route_problems,
        1,
    );

    assert_eq!(row_health.len(), 2);
    let Some(account_health) = row_health.iter().find(|row| {
        row.subject.kind == shared_types::AccountFieldSubjectKind::Account
            && row.subject.venue.as_deref() == Some("gate")
    }) else {
        panic!("gate route failure should expose account-level health");
    };
    assert_eq!(account_health.source, OPEN_ORDERS_SOURCE);
    assert_eq!(
        account_health
            .last_error
            .as_ref()
            .map(|problem| problem.status),
        Some(Some(502))
    );
}

#[test]
fn open_order_success_timestamp_requires_freshness_evidence() {
    let mut health = operation_health_row(CREDENTIAL_OPEN_ORDERS_PROBE, VenueOperationStatus::Ok);
    health.freshness_ms = None;

    assert_eq!(open_order_last_success_ms(&health), None);
}

fn operation_health_row(operation: &str, status: VenueOperationStatus) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: "mock".to_owned(),
        operation: operation.to_owned(),
        status,
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

fn open_order_row(exchange: &str) -> OrderInfo {
    OrderInfo {
        execution_style: None,
        venue_time_in_force: Some("gtc".to_owned()),
        client_order_id: Some("client-o-1".to_owned()),
        reduce_only: Some(false),
        order_id: "o-1".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        exchange: exchange.to_owned(),
        side: shared_types::OrderSide::Buy,
        order_type: shared_types::OrderType::Limit,
        status: shared_types::OrderStatus::Open,
        quantity: 1.0,
        price: 10.0,
        filled_quantity: 0.0,
        filled_price: 0.0,
        fees: 0.0,
        created_at: chrono::Utc::now(),
    }
}
