#[test]
fn fresh_order_stream_sample_suppresses_unresolved_order_probe() {
    let snapshot = PrivateWsRuntimeHealth {
        venue: "binance".to_owned(),
        operation: OP_PRIVATE_WS_ORDER_STREAM,
        status: VenueOperationStatus::Ok,
        message: "私有 WS 收到订单事件".to_owned(),
        request_id: None,
        requested: None,
        rows: Some(2),
        freshness_ms: Some(500),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 60_500,
        ok_count: 0,
        warn_count: 0,
        blocked_count: 0,
        last_problem: None,
        last_problem_at_ms: None,
        account_dirty: None,
    };
    let rows = private_ws_runtime_rows(
        &[credential(true, true)],
        vec![snapshot],
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

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(2));
    assert!(row.problem.is_none());
}

#[test]
fn order_stream_sample_before_unresolved_order_does_not_suppress_probe() {
    let snapshot = PrivateWsRuntimeHealth {
        venue: "binance".to_owned(),
        operation: OP_PRIVATE_WS_ORDER_STREAM,
        status: VenueOperationStatus::Ok,
        message: "私有 WS 收到较早订单事件".to_owned(),
        request_id: None,
        requested: None,
        rows: Some(2),
        freshness_ms: Some(500),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 60_500,
        ok_count: 0,
        warn_count: 0,
        blocked_count: 0,
        last_problem: None,
        last_problem_at_ms: None,
        account_dirty: None,
    };
    let rows = private_ws_runtime_rows(
        &[credential(true, true)],
        vec![snapshot],
        &[order_record(
            "binance",
            ExecutionMode::Live,
            LiveOrderState::Accepted,
            61_000,
            61_500,
        )],
        62_000,
    );
    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    let details = row
        .problem
        .as_ref()
        .and_then(|problem| problem.details.as_ref())
        .expect("order stream problem details");

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.rows, Some(0));
    assert_eq!(row.freshness_ms, Some(1_000));
    assert_eq!(
        details
            .get("oldestUnresolvedOrderCreatedAtMs")
            .and_then(serde_json::Value::as_i64),
        Some(61_000)
    );
    assert_eq!(
        details
            .get("freshnessWindowMs")
            .and_then(serde_json::Value::as_u64),
        Some(ORDER_STREAM_PROBE_RETRY_AFTER_MS)
    );
}

#[test]
fn stale_order_stream_sample_does_not_suppress_unresolved_order_probe() {
    let snapshot = PrivateWsRuntimeHealth {
        venue: "binance".to_owned(),
        operation: OP_PRIVATE_WS_ORDER_STREAM,
        status: VenueOperationStatus::Ok,
        message: "私有 WS 收到旧订单事件".to_owned(),
        request_id: None,
        requested: None,
        rows: Some(2),
        freshness_ms: Some(ORDER_STREAM_PROBE_RETRY_AFTER_MS as i64 + 1),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 1,
        ok_count: 0,
        warn_count: 0,
        blocked_count: 0,
        last_problem: None,
        last_problem_at_ms: None,
        account_dirty: None,
    };
    let rows = private_ws_runtime_rows(
        &[credential(true, true)],
        vec![snapshot],
        &[order_record(
            "binance",
            ExecutionMode::Live,
            LiveOrderState::Accepted,
            1_000,
            2_000,
        )],
        90_000,
    );
    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.rows, Some(0));
    assert!(row.problem.is_some());
}

#[test]
fn empty_order_stream_ok_does_not_suppress_unresolved_order_probe() {
    let snapshot = PrivateWsRuntimeHealth {
        venue: "binance".to_owned(),
        operation: OP_PRIVATE_WS_ORDER_STREAM,
        status: VenueOperationStatus::Ok,
        message: "私有 WS 连接正常但无订单事件".to_owned(),
        request_id: None,
        requested: None,
        rows: Some(0),
        freshness_ms: Some(500),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 60_500,
        ok_count: 0,
        warn_count: 0,
        blocked_count: 0,
        last_problem: None,
        last_problem_at_ms: None,
        account_dirty: None,
    };
    let rows = private_ws_runtime_rows(
        &[credential(true, true)],
        vec![snapshot],
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

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.rows, Some(0));
    assert!(row.problem.is_some());
}

#[test]
fn configured_reconciliation_without_runtime_stays_unknown() {
    let rows = reconciliation_runtime_rows(&[credential(true, true)], Vec::new(), 10);

    assert_eq!(rows[0].operation, OP_ORDER_RECONCILIATION);
    assert_eq!(rows[0].status, VenueOperationStatus::Unknown);
    assert_eq!(rows[0].source, SOURCE_RECONCILIATION_RUNTIME);
    assert!(rows[0].error.is_none());
}

#[test]
fn empty_global_reconciliation_snapshot_proves_idle_without_remote_query() {
    let snapshot = ReconciliationRuntimeHealth {
        venue: GLOBAL_RECONCILIATION_VENUE.to_owned(),
        status: VenueOperationStatus::Ok,
        message: "订单回查完成".to_owned(),
        requested: Some(0),
        rows: Some(0),
        freshness_ms: Some(50),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 10,
    };

    let rows = reconciliation_runtime_rows(&[credential(true, true)], vec![snapshot], 20);

    assert_eq!(rows[0].venue, "binance");
    assert_eq!(rows[0].operation, OP_ORDER_RECONCILIATION);
    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].configured, Some(true));
    assert_eq!(rows[0].requested, Some(0));
    assert_eq!(rows[0].rows, Some(0));
    assert!(rows[0].message.contains("订单回查空闲"));
}

#[test]
fn active_global_reconciliation_snapshot_does_not_prove_venue_query() {
    let snapshot = ReconciliationRuntimeHealth {
        venue: GLOBAL_RECONCILIATION_VENUE.to_owned(),
        status: VenueOperationStatus::Ok,
        message: "订单回查完成".to_owned(),
        requested: Some(1),
        rows: Some(1),
        freshness_ms: Some(50),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 10,
    };

    let rows = reconciliation_runtime_rows(&[credential(true, true)], vec![snapshot], 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Unknown);
    assert_eq!(rows[0].requested, None);
    assert!(rows[0].message.contains("未证明该交易所订单回查"));
}

#[test]
fn configured_order_finality_without_runtime_stays_unknown() {
    let rows = run_finality_runtime_rows(&[credential(true, true)], Vec::new(), 10);

    assert_eq!(rows[0].operation, OP_ORDER_FINALITY);
    assert_eq!(rows[0].status, VenueOperationStatus::Unknown);
    assert_eq!(rows[0].source, SOURCE_RUN_FINALITY_RUNTIME);
    assert!(rows[0].error.is_none());
}

#[test]
fn empty_global_run_finality_snapshot_proves_idle_without_remote_query() {
    let mut snapshot = run_finality_health(GLOBAL_RUN_FINALITY_VENUE, VenueOperationStatus::Ok);
    snapshot.message = "订单终态回查完成".to_owned();
    snapshot.requested = Some(0);
    snapshot.rows = Some(0);

    let rows = run_finality_runtime_rows(&[credential(true, true)], vec![snapshot], 20);

    assert_eq!(rows[0].venue, "binance");
    assert_eq!(rows[0].operation, OP_ORDER_FINALITY);
    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].configured, Some(true));
    assert_eq!(rows[0].requested, Some(0));
    assert_eq!(rows[0].rows, Some(0));
    assert!(rows[0].message.contains("订单终态回查空闲"));
}

#[test]
fn active_global_run_finality_snapshot_does_not_prove_venue_finality() {
    let mut snapshot = run_finality_health(GLOBAL_RUN_FINALITY_VENUE, VenueOperationStatus::Ok);
    snapshot.message = "订单终态回查完成".to_owned();
    snapshot.requested = Some(1);
    snapshot.rows = Some(1);

    let rows = run_finality_runtime_rows(&[credential(true, true)], vec![snapshot], 20);

    assert_eq!(rows[0].status, VenueOperationStatus::Unknown);
    assert_eq!(rows[0].requested, None);
    assert!(rows[0].message.contains("未证明该交易所订单终态回查"));
}
