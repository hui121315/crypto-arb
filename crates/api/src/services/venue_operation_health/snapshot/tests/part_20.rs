#[test]
fn credential_bound_private_ws_proof_restores_idle_stream_readiness() {
    let mut rows = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Ok, VenueOperationStatus::Ok),
        &[],
        20,
    );
    let mut proof = live_order_proof_health("binance", VenueOperationStatus::Ok);
    proof
        .cancel_finality
        .as_mut()
        .expect("cancel proof")
        .source = "private_ws_order".to_owned();

    overlay_order_stream_readiness_from_live_proof(&mut rows, &[proof]);

    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(1));
    assert!(row.message.contains("同凭证订单事件已有持久化证明"));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence.request_context.iter().any(|item| {
            item == "stream_readiness=current_session_plus_credential_bound_event_proof"
        })
    }));
}

#[test]
fn restart_proof_cannot_override_disconnected_or_unconfirmed_session() {
    let mut rows = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Blocked, VenueOperationStatus::Ok),
        &[],
        20,
    );
    let mut proof = live_order_proof_health("binance", VenueOperationStatus::Ok);
    proof
        .cancel_finality
        .as_mut()
        .expect("cancel proof")
        .source = "private_ws_order".to_owned();

    overlay_order_stream_readiness_from_live_proof(&mut rows, &[proof]);

    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(row.rows, Some(0));
}

#[test]
fn order_query_cancel_proof_does_not_prove_private_ws_delivery() {
    let mut rows = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Ok, VenueOperationStatus::Ok),
        &[],
        20,
    );
    let proof = live_order_proof_health("binance", VenueOperationStatus::Ok);

    overlay_order_stream_readiness_from_live_proof(&mut rows, &[proof]);

    let row = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    assert_eq!(row.status, VenueOperationStatus::Unknown);
    assert_eq!(row.rows, Some(0));
}

#[test]
fn idle_current_session_is_ready_without_claiming_event_samples() {
    let mut rows = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Ok, VenueOperationStatus::Ok),
        &[],
        20,
    );
    let balances = vec![fresh_cache("binance", 3, 18)];
    let positions = vec![fresh_cache("binance", 0, 19)];

    overlay_idle_private_ws_readiness(&mut rows, &balances, &positions);

    let order = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    let account = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");
    assert_eq!(order.status, VenueOperationStatus::Ok);
    assert_eq!(order.rows, Some(0));
    assert!(order.message.contains("无未决实盘订单"));
    assert_eq!(account.status, VenueOperationStatus::Ok);
    assert_eq!(account.rows, Some(0));
    assert!(account.message.contains("余额与持仓缓存新鲜"));
    assert!(account.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "event_sample=current_session_none_expected")
    }));
}

#[test]
fn idle_account_stream_stays_ready_during_bounded_cache_refresh() {
    let mut rows = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Ok, VenueOperationStatus::Ok),
        &[],
        25_000,
    );
    let balances = vec![stale_cache(
        "binance",
        3,
        BALANCE_CACHE_TTL_MS + 1,
        9_999,
    )];
    let positions = vec![stale_cache(
        "binance",
        0,
        POSITION_CACHE_MAX_STALE_MS - 1,
        10_001,
    )];

    overlay_idle_private_ws_readiness(&mut rows, &balances, &positions);

    let account = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");
    assert_eq!(account.status, VenueOperationStatus::Ok);
    assert!(account.message.contains("后台刷新中"));
    assert!(account.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "account_cache_readiness=bounded_refresh")
    }));
}

#[test]
fn idle_account_stream_rejects_cache_beyond_refresh_grace() {
    let mut rows = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Ok, VenueOperationStatus::Ok),
        &[],
        70_000,
    );
    let balances = vec![stale_cache(
        "binance",
        3,
        BALANCE_CACHE_TTL_MS + 8_001,
        46_999,
    )];
    let positions = vec![fresh_cache("binance", 0, 69_999)];

    overlay_idle_private_ws_readiness(&mut rows, &balances, &positions);

    let account = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");
    assert_eq!(account.status, VenueOperationStatus::Unknown);
}

#[test]
fn idle_readiness_keeps_disconnected_and_stale_account_evidence_unknown() {
    let mut disconnected = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Blocked, VenueOperationStatus::Ok),
        &[],
        20,
    );
    let balances = vec![fresh_cache("binance", 3, 18)];
    let positions = vec![fresh_cache("binance", 0, 19)];

    overlay_idle_private_ws_readiness(&mut disconnected, &balances, &positions);
    assert!(disconnected.iter().filter(|row| {
        matches!(
            row.operation.as_str(),
            OP_PRIVATE_WS_ORDER_STREAM | OP_PRIVATE_WS_ACCOUNT_STREAM
        )
    }).all(|row| row.status == VenueOperationStatus::Unknown));

    let mut missing_positions = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Ok, VenueOperationStatus::Ok),
        &[],
        20,
    );
    overlay_idle_private_ws_readiness(&mut missing_positions, &balances, &[]);
    let order = missing_positions
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    let account = missing_positions
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ACCOUNT_STREAM)
        .expect("account stream row");
    assert_eq!(order.status, VenueOperationStatus::Ok);
    assert_eq!(account.status, VenueOperationStatus::Unknown);
}

#[test]
fn unresolved_live_order_warning_is_not_overridden_by_idle_readiness() {
    let mut rows = private_ws_runtime_rows(
        &[credential(true, true)],
        private_ws_restart_rows(VenueOperationStatus::Ok, VenueOperationStatus::Ok),
        &[order_record(
            "binance",
            ExecutionMode::Live,
            LiveOrderState::Accepted,
            1_000,
            2_000,
        )],
        61_000,
    );

    overlay_idle_private_ws_readiness(&mut rows, &[], &[]);

    let order = rows
        .iter()
        .find(|row| row.operation == OP_PRIVATE_WS_ORDER_STREAM)
        .expect("order stream row");
    assert_eq!(order.status, VenueOperationStatus::Warn);
    assert!(order.problem.is_some());
}

fn private_ws_restart_rows(
    session_status: VenueOperationStatus,
    subscribe_status: VenueOperationStatus,
) -> Vec<PrivateWsRuntimeHealth> {
    vec![
        private_ws_restart_row(OP_PRIVATE_WS_SESSION, session_status, None),
        private_ws_restart_row(OP_PRIVATE_WS_SUBSCRIBE, subscribe_status, Some(5)),
        private_ws_restart_row(
            OP_PRIVATE_WS_ORDER_STREAM,
            VenueOperationStatus::Unknown,
            Some(0),
        ),
        private_ws_restart_row(
            OP_PRIVATE_WS_ACCOUNT_STREAM,
            VenueOperationStatus::Unknown,
            Some(0),
        ),
    ]
}

fn private_ws_restart_row(
    operation: &'static str,
    status: VenueOperationStatus,
    rows: Option<u64>,
) -> PrivateWsRuntimeHealth {
    PrivateWsRuntimeHealth {
        venue: "binance".to_owned(),
        operation,
        status,
        message: "runtime evidence".to_owned(),
        request_id: None,
        requested: None,
        rows,
        freshness_ms: Some(5),
        retry_after_ms: None,
        error: None,
        observed_at_ms: 15,
        ok_count: u64::from(status == VenueOperationStatus::Ok),
        warn_count: 0,
        blocked_count: u64::from(status == VenueOperationStatus::Blocked),
        last_problem: None,
        last_problem_at_ms: None,
        account_dirty: None,
    }
}

fn stale_cache(
    venue: &str,
    rows: u64,
    freshness_ms: i64,
    observed_at_ms: i64,
) -> AccountCacheSnapshot {
    AccountCacheSnapshot {
        venue: venue.to_owned(),
        rows,
        freshness_ms,
        observed_at_ms,
        quality: AccountCacheQuality::Stale,
    }
}
