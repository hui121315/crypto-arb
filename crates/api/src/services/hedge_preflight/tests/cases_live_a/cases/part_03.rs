#[test]
fn live_operation_health_guard_requires_current_balance_and_positions() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    for (operation, expected) in [
        (OP_BALANCE, "缺少余额读取运行态证据"),
        (OP_POSITIONS, "缺少持仓读取运行态证据"),
    ] {
        let rows = full_live_operation_rows("binance")
            .into_iter()
            .filter(|row| row.operation != operation)
            .collect::<Vec<_>>();
        let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
            .unwrap_or_else(missing_guard);

        assert!(!guard.passed);
        assert!(guard.detail.contains(expected));
    }
}

#[test]
fn live_operation_health_guard_blocks_degraded_account_cache() -> Result<(), String> {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let mut rows = full_live_operation_rows("binance");
    let balance = rows
        .iter_mut()
        .find(|row| row.operation == OP_BALANCE)
        .ok_or_else(|| "balance fixture is missing".to_owned())?;
    balance.status = VenueOperationStatus::Blocked;
    balance.message = "balance cache stale".to_owned();

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
        .unwrap_or_else(missing_guard);

    assert!(!guard.passed);
    assert!(guard.detail.contains("余额读取 未通过"));
    Ok(())
}

#[test]
fn builder_venue_does_not_borrow_family_private_ws_identity() {
    let long = plan("hyperliquid:xyz", "MU-USDC", Vec::new());
    let mut rows = full_live_operation_rows("hyperliquid:xyz")
        .into_iter()
        .filter(|row| {
            row.operation != OP_PRIVATE_WS_SESSION && row.operation != OP_PRIVATE_WS_SUBSCRIBE
        })
        .collect::<Vec<_>>();
    rows.extend([
        operation_row(
            "hyperliquid",
            OP_PRIVATE_WS_SESSION,
            VenueOperationStatus::Ok,
        ),
        operation_row(
            "hyperliquid",
            OP_PRIVATE_WS_SUBSCRIBE,
            VenueOperationStatus::Ok,
        ),
    ]);

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
        .unwrap_or_else(missing_guard);

    assert!(!guard.passed);
    assert!(guard.detail.contains("缺少私有 WS 会话运行态证据"));
}
