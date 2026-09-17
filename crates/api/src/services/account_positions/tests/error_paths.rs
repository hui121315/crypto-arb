use super::*;

#[test]
fn position_field_quality_keeps_valid_mark_price_quiet() {
    let rows = vec![position_row()];

    let quality = position_field_quality(&rows, 10);

    assert!(!quality.iter().any(|row| row.field == "markPrice"));
}

#[test]
fn partial_position_fanout_keeps_rows_and_surfaces_route_health() {
    let route_failures = vec![RouteFailure::new(
        "gate".to_owned(),
        POSITION_OPERATION,
        ExchangeError::Network("position route failed".to_owned()),
    )];
    let route_problems = route_failure_problems(&route_failures);

    let row_health = position_row_health(
        &[PositionInfo {
            exchange: "binance".to_owned(),
            ..position_row()
        }],
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
    assert_eq!(
        account_health.source,
        "gate.GET /api/v4/futures/usdt/positions"
    );
    assert_eq!(
        account_health
            .last_error
            .as_ref()
            .map(|problem| problem.status),
        Some(Some(502))
    );
}

#[test]
fn partial_position_envelope_keeps_rows_and_binds_each_venue() {
    let route_failures = vec![RouteFailure::new(
        "gate".to_owned(),
        POSITION_OPERATION,
        ExchangeError::Network("position route failed".to_owned()),
    )];

    let envelope = build_envelope(
        1,
        vec![PositionInfo {
            exchange: "binance".to_owned(),
            ..position_row()
        }],
        Vec::new(),
        (&route_failures, &[]),
        Vec::new(),
        Vec::new(),
    );

    assert_eq!(envelope.status, ListStatus::Degraded);
    assert_eq!(envelope.rows.len(), 1);
    assert_eq!(envelope.account_bindings.len(), 2);
    assert!(envelope
        .account_bindings
        .iter()
        .any(|binding| binding.venue == "binance"));
    assert!(envelope
        .account_bindings
        .iter()
        .any(|binding| binding.venue == "gate"));
}

#[test]
fn bounded_cached_position_keeps_transient_route_failure_at_row_level() {
    let route_failures = vec![RouteFailure::new(
        "binance".to_owned(),
        POSITION_OPERATION,
        ExchangeError::Timeout { seconds: 3 },
    )];
    let mut cache_health = operation_health_row(ACCOUNT_CACHE_OPERATION);
    cache_health.venue = "binance".to_owned();
    cache_health.source = ACCOUNT_CACHE_SOURCE.to_owned();
    cache_health.rows = Some(1);

    let envelope = build_envelope(
        1,
        vec![position_row()],
        vec![cache_health],
        (&route_failures, &[]),
        Vec::new(),
        Vec::new(),
    );

    assert_eq!(envelope.status, ListStatus::Fresh);
    assert!(envelope.problems.is_empty());
    assert!(envelope.row_health.iter().any(|health| {
        health.subject.venue.as_deref() == Some("binance") && health.last_error.is_some()
    }));
}

#[test]
fn position_success_timestamp_requires_freshness_evidence() {
    let mut health = operation_health_row(ACCOUNT_CACHE_OPERATION);
    health.freshness_ms = None;

    assert_eq!(position_last_success_ms(&health), None);
}

#[tokio::test]
async fn gate_rate_limit_envelope_keeps_rows_request_id_and_retry_after() {
    let route_failures = vec![RouteFailure::new(
        "gate".to_owned(),
        POSITION_OPERATION,
        ExchangeError::RateLimited {
            retry_after_secs: 30,
        },
    )];

    let envelope = common::request_id::scope("req-gate-positions-429".to_owned(), async {
        build_envelope(
            1,
            vec![PositionInfo {
                exchange: "binance".to_owned(),
                ..position_row()
            }],
            Vec::new(),
            (&route_failures, &[]),
            Vec::new(),
            Vec::new(),
        )
    })
    .await;
    let Some(problem) = envelope
        .problems
        .iter()
        .find(|problem| problem.code == "RATE_LIMITED")
    else {
        panic!("typed Gate rate-limit problem");
    };

    assert_eq!(envelope.status, ListStatus::Degraded);
    assert_eq!(envelope.rows.len(), 1);
    assert_eq!(problem.status, Some(429));
    assert_eq!(
        problem.request_id.as_deref(),
        Some("req-gate-positions-429")
    );
    assert_eq!(problem.retry_after_ms, Some(30_000));
    assert_eq!(
        problem.source.as_deref(),
        Some("gate.GET /api/v4/futures/usdt/positions")
    );
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|value| value["method"].as_str()),
        Some("GET")
    );
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|value| value["path"].as_str()),
        Some("/api/v4/futures/usdt/positions")
    );
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|value| value["apiPath"].as_str()),
        Some(POSITION_ROUTE)
    );
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|value| value["venue"].as_str()),
        Some("gate")
    );
}

#[test]
fn gate_rest_risk_fields_keep_actual_source_evidence() {
    let row = PositionInfo {
        exchange: "Gate".to_owned(),
        mark_price: 100.0,
        liquidation_price: Some(80.0),
        liquidation_distance_pct: Some(20.0),
        leverage: 5.0,
        margin: 20.0,
        maintenance_margin_ratio: 0.005,
        position_mode: Some("single".to_owned()),
        margin_mode: Some("isolated".to_owned()),
        ..position_row()
    };

    let quality = position_field_quality(&[row], 10);

    assert!(quality.iter().any(|item| {
        item.field == "liquidationDistancePct"
            && item.status == AccountFieldQualityStatus::Actual
            && item.source == "gate_rest_liq_price_derived_distance"
    }));
    assert!(quality.iter().any(|item| {
        item.field == "maintenanceMarginRatio"
            && item.status == AccountFieldQualityStatus::Actual
            && item.source == "gate_rest_average_maintenance_rate"
    }));
    assert_eq!(position_status(&[], &[], &quality), ListStatus::Fresh);
}
