#[test]
fn live_operation_health_guard_blocks_missing_order_write_runtime_proof() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let rows = full_live_operation_rows("binance")
        .into_iter()
        .filter(|row| row.operation != OP_ORDER_WRITE)
        .collect::<Vec<_>>();

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
        .unwrap_or_else(missing_guard);

    assert!(!guard.passed);
    assert!(guard.detail.contains("缺少写单运行态证据"));
}

#[test]
fn live_operation_health_guard_blocks_incomplete_order_write_runtime_proof(
) -> Result<(), String> {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let mut rows = full_live_operation_rows("binance");
    let order_write = rows
        .iter_mut()
        .find(|row| row.operation == OP_ORDER_WRITE)
        .ok_or_else(|| "order-write fixture is missing".to_owned())?;
    order_write.status = VenueOperationStatus::Warn;
    order_write.message = "已有 live 下单 ack 样本，仍缺撤单终态远程证明".to_owned();
    order_write.retry_after_ms = Some(60_000);
    order_write.evidence = Some(request_evidence("req-live-order-write-1"));
    order_write.problem = Some(
        ApiProblem::new(codes::HEDGE_PRE_TRADE_REJECTED, "live order proof incomplete")
            .with_source("live_order_proof_runtime")
            .with_retry_after_ms(Some(60_000)),
    );

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
        .unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(!guard.passed);
    assert!(guard.detail.contains("写单 未通过"));
    assert_eq!(outcome.request_id.as_deref(), Some("req-live-order-write-1"));
    assert_eq!(outcome.retry_after_ms, Some(60_000));
    assert_eq!(outcome.problems.len(), 1);
    Ok(())
}

#[test]
fn live_operation_health_guard_requires_private_ws_session_and_subscription() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    for (operation, expected) in [
        (OP_PRIVATE_WS_SESSION, "缺少私有 WS 会话运行态证据"),
        (OP_PRIVATE_WS_SUBSCRIBE, "缺少私有 WS 订阅运行态证据"),
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
