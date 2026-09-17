use super::*;

mod portfolio_cache;
mod storage;

#[test]
fn api_health_missing_operations_is_no_evidence_not_funding_proxy() {
    let snapshot = VenueOperationHealthSnapshot::new(Vec::new(), 1_000);
    let mut problems = vec![RuntimeProblem {
        scope: "portfolio".into(),
        operation: "positions".into(),
        code: "UPSTREAM".into(),
        message: "gate positions parse".into(),
        venue: Some("gate".into()),
        retry_after_ms: None,
        problem: None,
        observed_at_ms: 1,
    }];

    let health = api_health_or_missing(&snapshot, &mut problems, 2_000, true);

    assert_eq!(health.healthy, 0);
    assert_eq!(health.total, 0);
    assert!(health.failed_venues.is_empty());
    assert_eq!(problems.len(), 2);
    assert_eq!(problems[1].scope, "trading_api");
    assert_eq!(problems[1].operation, "venue_operation_health");
    assert_eq!(problems[1].code, "TRADING_API_HEALTH_MISSING");
    assert!(problems[1].venue.is_none());
}

#[test]
fn api_health_from_operations_counts_only_ok_as_healthy() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_health("binance", "balance", VenueOperationStatus::Ok),
            operation_health("okx", "order_write", VenueOperationStatus::Unknown),
            unsupported_operation_health("bybit", "positions"),
        ],
        1_000,
    );

    let health = api_health_from_operations(&snapshot, true);
    assert!(health.is_some());
    let health = health.unwrap_or(ApiHealthSlot {
        healthy: 0,
        total: 0,
        failed_venues: Vec::new(),
    });

    assert_eq!(health.healthy, 1);
    assert_eq!(health.total, 2);
    assert_eq!(health.failed_venues, vec!["okx"]);
}

#[test]
fn operation_health_problem_keeps_typed_request_context() {
    let mut row = operation_health("gate", "positions", VenueOperationStatus::Warn);
    row.retry_after_ms = Some(2_000);
    row.problem = Some(
        ApiProblem::new("GATE_POSITION_PARSE", "gate position payload changed")
            .with_status(502)
            .with_request_id(Some("req-pr-ay-gate".to_owned()))
            .with_source("gate.GET /api/v4/futures/usdt/positions"),
    );

    let runtime = operation_health_problem(&row);
    let problem = runtime.to_api_problem();

    assert_eq!(runtime.venue.as_deref(), Some("gate"));
    assert_eq!(problem.status, Some(502));
    assert_eq!(problem.request_id.as_deref(), Some("req-pr-ay-gate"));
    assert_eq!(problem.retry_after_ms, Some(2_000));
    assert_eq!(
        problem.source.as_deref(),
        Some("gate.GET /api/v4/futures/usdt/positions")
    );
}

#[test]
fn api_health_requires_ok_rows_to_be_configured_and_supported() {
    let mut unconfigured = operation_health("okx", "order_write", VenueOperationStatus::Ok);
    unconfigured.configured = Some(false);
    let mut unsupported = operation_health("bybit", "positions", VenueOperationStatus::Ok);
    unsupported.supported = Some(false);
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_health("binance", "balance", VenueOperationStatus::Ok),
            unconfigured,
            unsupported,
        ],
        1_000,
    );

    assert_eq!(snapshot.rows[1].configured, Some(false));
    assert!(snapshot.rows[1].capability_supported());
    assert_eq!(snapshot.rows[2].supported, Some(false));
    assert!(!snapshot.rows[2].capability_supported());
    assert!(!snapshot.rows[1].is_currently_usable());
    assert!(!snapshot.rows[2].is_currently_usable());

    let health = api_health_from_operations(&snapshot, true).unwrap_or(ApiHealthSlot {
        healthy: 0,
        total: 0,
        failed_venues: Vec::new(),
    });

    assert_eq!(health.healthy, 1);
    assert_eq!(health.total, 2);
    assert_eq!(health.failed_venues, vec!["okx"]);
}

#[test]
fn api_health_from_operations_excludes_transport_rows_from_counts() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_health("bybit", "positions", VenueOperationStatus::Ok),
            operation_health(
                "gate",
                "http_rest:GET /api/v4/orders",
                VenueOperationStatus::Warn,
            ),
            operation_health("okx", "host_gate:okx", VenueOperationStatus::Blocked),
            operation_health(
                "binance",
                "rate_limiter:binance",
                VenueOperationStatus::Warn,
            ),
        ],
        1_000,
    );

    let health = api_health_from_operations(&snapshot, true).unwrap_or(ApiHealthSlot {
        healthy: 0,
        total: 0,
        failed_venues: Vec::new(),
    });

    assert_eq!(health.healthy, 1);
    assert_eq!(health.total, 1);
    assert!(health.failed_venues.is_empty());
}

#[test]
fn api_health_from_operations_counts_verified_credential_probes() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_health(
                "okx",
                "credential_probe:balance_read",
                VenueOperationStatus::Ok,
            ),
            operation_health(
                "gate",
                "credential_probe:positions_read",
                VenueOperationStatus::Unknown,
            ),
            operation_health(
                "kucoin",
                "credential_probe:open_orders_read",
                VenueOperationStatus::Ok,
            ),
            operation_health(
                "bybit",
                "credential_probe:account_mode_read",
                VenueOperationStatus::Ok,
            ),
            operation_health("okx", "credential_probe:made_up", VenueOperationStatus::Ok),
            operation_health(
                "okx",
                "credential_probe:order_permission",
                VenueOperationStatus::Unknown,
            ),
        ],
        1_000,
    );

    let health = api_health_from_operations(&snapshot, true);
    assert!(health.is_some());
    let health = health.unwrap_or(ApiHealthSlot {
        healthy: 0,
        total: 0,
        failed_venues: Vec::new(),
    });

    assert_eq!(health.healthy, 3);
    assert_eq!(health.total, 5);
    assert_eq!(health.failed_venues, vec!["gate", "okx"]);
}

#[test]
fn ws_health_from_operations_marks_unknown_stream_disconnected() {
    let snapshot = VenueOperationHealthSnapshot::new(
        vec![
            operation_health("binance", "private_ws_session", VenueOperationStatus::Ok),
            operation_health(
                "okx",
                "private_ws_order_stream",
                VenueOperationStatus::Unknown,
            ),
            unsupported_operation_health("bybit", "private_ws_account_stream"),
        ],
        1_000,
    );

    let health = ws_health_from_operations(&snapshot, true);
    assert!(health.is_some());
    let health = health.unwrap_or(WsHealthSlot {
        channels: 0,
        disconnected: Vec::new(),
    });

    assert_eq!(health.channels, 2);
    assert_eq!(health.disconnected, vec!["okx:private_ws_order_stream"]);
}

#[test]
fn ws_health_requires_ok_rows_to_be_configured_and_supported() {
    let mut unconfigured =
        operation_health("okx", "private_ws_order_stream", VenueOperationStatus::Ok);
    unconfigured.configured = Some(false);
    let mut unsupported = operation_health(
        "bybit",
        "private_ws_account_stream",
        VenueOperationStatus::Ok,
    );
    unsupported.supported = Some(false);
    let snapshot = VenueOperationHealthSnapshot::new(vec![unconfigured, unsupported], 1_000);

    assert_eq!(snapshot.rows[0].configured, Some(false));
    assert!(snapshot.rows[0].capability_supported());
    assert_eq!(snapshot.rows[1].supported, Some(false));
    assert!(!snapshot.rows[1].capability_supported());
    assert!(!snapshot.rows[0].is_currently_usable());
    assert!(!snapshot.rows[1].is_currently_usable());

    let health = ws_health_from_operations(&snapshot, true).unwrap_or(WsHealthSlot {
        channels: 0,
        disconnected: Vec::new(),
    });

    assert_eq!(health.channels, 1);
    assert_eq!(health.disconnected, vec!["okx:private_ws_order_stream"]);
}

#[test]
fn ws_health_missing_operations_is_no_evidence_not_hub_proxy() {
    let snapshot = VenueOperationHealthSnapshot::new(Vec::new(), 1_000);
    let mut problems = Vec::new();

    let health = ws_health_or_missing(&snapshot, &mut problems, 2_000, true);

    assert_eq!(health.channels, 0);
    assert!(health.disconnected.is_empty());
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].scope, "private_ws");
    assert_eq!(problems[0].operation, "venue_operation_health");
    assert_eq!(problems[0].code, "PRIVATE_WS_HEALTH_MISSING");
    assert!(problems[0].venue.is_none());
}

#[test]
fn paper_mode_treats_unconfigured_private_operations_as_optional() {
    let mut api = operation_health("okx", "private_read", VenueOperationStatus::Blocked);
    api.configured = Some(false);
    let mut ws = operation_health(
        "okx",
        "private_ws_account_stream",
        VenueOperationStatus::Blocked,
    );
    ws.configured = Some(false);
    let snapshot = VenueOperationHealthSnapshot::new(vec![api, ws], 1_000);

    let api = api_health_from_operations(&snapshot, false);
    let ws = ws_health_from_operations(&snapshot, false);

    assert_eq!(
        api.as_ref().map(|slot| (slot.healthy, slot.total)),
        Some((0, 0))
    );
    assert!(api.is_some_and(|slot| slot.failed_venues.is_empty()));
    assert_eq!(ws.as_ref().map(|slot| slot.channels), Some(0));
    assert!(ws.is_some_and(|slot| slot.disconnected.is_empty()));
}
