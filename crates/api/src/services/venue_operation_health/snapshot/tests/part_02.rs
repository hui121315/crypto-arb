#[test]
fn market_row_builds_fallback_problem_without_exchange_problem() {
    let row = market_row(
        MarketRuntimeHealth {
            venue: "okx".to_owned(),
            operation: "rest_spot_ticks",
            quality: MarketQuality::Missing,
            source: MarketSource::RestBaseline,
            requested: 2,
            rows: 0,
            retry_after_ms: None,
            last_error: Some("no rows returned".to_owned()),
            problem: None,
            observed_at_ms: 10_000,
        },
        11_000,
    );

    let problem = row.problem.expect("fallback problem");

    assert_eq!(problem.code, "MARKET_DATA_MISSING");
    assert_eq!(problem.message, "no rows returned");
    assert_eq!(
        problem.source.as_deref(),
        Some("market_data_cache:rest_baseline")
    );
    assert!(problem
        .details
        .as_ref()
        .and_then(|details| details.get("requested"))
        .is_some_and(|requested| requested.as_u64() == Some(2)));
}

#[test]
fn configured_private_read_without_account_cache_stays_unknown() {
    let rows = account_cache_rows(&[credential(true, true)], Vec::new(), OP_BALANCE, 10);

    assert_eq!(rows[0].operation, OP_BALANCE);
    assert_eq!(rows[0].status, VenueOperationStatus::Unknown);
    assert_eq!(rows[0].source, SOURCE_ACCOUNT_CACHE);
    assert!(rows[0].error.is_none());
}

#[test]
fn fresh_account_cache_snapshot_maps_to_ok() {
    let snapshot = AccountCacheSnapshot {
        venue: "binance".to_owned(),
        rows: 2,
        freshness_ms: 50,
        observed_at_ms: 10,
        quality: AccountCacheQuality::Fresh,
    };

    let rows = account_cache_rows(&[credential(true, true)], vec![snapshot], OP_POSITIONS, 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].rows, Some(2));
    assert_eq!(rows[0].freshness_ms, Some(50));
    assert_eq!(rows[0].observed_at_ms, 10);
}

#[test]
fn configured_private_ws_without_runtime_stays_unknown() {
    let rows = private_ws_runtime_rows(&[credential(true, true)], Vec::new(), &[], 10);

    assert_eq!(rows.len(), PRIVATE_WS_OPS.len());
    assert!(rows
        .iter()
        .all(|row| row.status == VenueOperationStatus::Unknown));
    assert!(rows
        .iter()
        .all(|row| row.source == SOURCE_PRIVATE_WS_RUNTIME));
    let order_stream = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    assert_private_ws_evidence(order_stream, "binance-usdm-user-data-stream-order-update");
}

#[test]
fn private_ws_order_stream_uses_live_write_support() {
    let mut venue = credential(true, true);
    venue.live_write = false;
    let rows = private_ws_runtime_rows(&[venue], Vec::new(), &[], 10);

    let order_stream = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    let account_stream = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");

    assert_eq!(order_stream.status, VenueOperationStatus::Unsupported);
    assert_eq!(order_stream.supported, Some(false));
    assert_eq!(account_stream.status, VenueOperationStatus::Unknown);
    assert_eq!(account_stream.supported, Some(true));
}

#[test]
fn private_ws_unknown_operation_is_unsupported() {
    let venue = credential(true, true);

    assert!(!private_ws_operation_supported(
        &venue,
        "private_ws_made_up"
    ));
}

#[test]
fn private_ws_runtime_snapshot_maps_to_operation_row() {
    let snapshot = PrivateWsRuntimeHealth {
        venue: "binance".to_owned(),
        operation: OP_PRIVATE_WS_ORDER_STREAM,
        status: VenueOperationStatus::Ok,
        message: "私有 WS 订单事件已入账".to_owned(),
        request_id: None,
        requested: None,
        rows: Some(2),
        freshness_ms: Some(50),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 10,
        ok_count: 0,
        warn_count: 0,
        blocked_count: 0,
        last_problem: None,
        last_problem_at_ms: None,
        account_dirty: None,
    };

    let rows = private_ws_runtime_rows(&[credential(true, true)], vec![snapshot], &[], 20);
    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("private ws order row");

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(2));
    assert_eq!(row.freshness_ms, Some(50));
    assert_private_ws_evidence(row, "binance-usdm-user-data-stream-order-update");
}

#[test]
fn private_ws_runtime_failure_maps_to_problem() {
    let snapshot = PrivateWsRuntimeHealth {
        venue: "hyperliquid".to_owned(),
        operation: OP_PRIVATE_WS_SUBSCRIBE,
        status: VenueOperationStatus::Blocked,
        message: "私有 WS 订阅 payload 构建失败：user_events: invalid address".to_owned(),
        request_id: Some("req-private-ws-runtime".to_owned()),
        requested: Some(6),
        rows: Some(1),
        freshness_ms: Some(12),
        retry_after_ms: Some(2_000),
        error: Some("user_events: invalid address".to_owned()),
        observed_at_ms: 10,
        ok_count: 0,
        warn_count: 0,
        blocked_count: 0,
        last_problem: None,
        last_problem_at_ms: None,
        account_dirty: None,
    };

    let rows = private_ws_runtime_rows(&[credential(true, true)], vec![snapshot], &[], 20);
    let row = rows
        .iter()
        .find(|row| row.venue == "hyperliquid" && row.operation == OP_PRIVATE_WS_SUBSCRIBE)
        .expect("private ws subscribe row");
    let problem = row.problem.as_ref().expect("private ws problem");

    assert_eq!(problem.code, codes::PRIVATE_WS_RUNTIME_FAILED);
    assert_eq!(problem.source.as_deref(), Some(SOURCE_PRIVATE_WS_RUNTIME));
    assert_eq!(
        problem.request_id.as_deref(),
        Some("req-private-ws-runtime")
    );
    assert_eq!(problem.retry_after_ms, Some(2_000));
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("requestId"))
            .and_then(serde_json::Value::as_str),
        Some("req-private-ws-runtime")
    );
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("rows"))
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
}

#[test]
fn order_stream_probe_warns_on_unresolved_orders_without_ws_sample() {
    let rows = private_ws_runtime_rows(
        &[credential(true, true)],
        Vec::new(),
        &[order_record(
            "binance",
            ExecutionMode::Live,
            LiveOrderState::Accepted,
            1_000,
            2_000,
        )],
        61_000,
    );
    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    let problem = row.problem.as_ref().expect("order stream problem");

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.supported, Some(true));
    assert_eq!(row.configured, Some(true));
    assert_eq!(row.requested, Some(1));
    assert_eq!(row.rows, Some(0));
    assert_eq!(row.freshness_ms, Some(60_000));
    assert_eq!(row.retry_after_ms, Some(ORDER_STREAM_PROBE_RETRY_AFTER_MS));
    assert_private_ws_evidence(row, "binance-usdm-user-data-stream-order-update");
    assert_eq!(problem.code, codes::PRIVATE_WS_RUNTIME_FAILED);
    assert_eq!(problem.request_id, None);
}

#[test]
fn order_stream_probe_ignores_paper_orders() {
    let rows = private_ws_runtime_rows(
        &[credential(true, true)],
        Vec::new(),
        &[order_record(
            "binance",
            ExecutionMode::DryRun,
            LiveOrderState::Accepted,
            1_000,
            2_000,
        )],
        61_000,
    );
    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");

    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(row.requested, None);
    assert!(row.problem.is_none());
}
