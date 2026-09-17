#[test]
fn live_operation_health_guard_projects_runtime_http_context() -> Result<(), String> {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let mut rows = full_live_operation_rows("binance");
    let order_write = rows
        .iter_mut()
        .find(|row| row.operation == OP_ORDER_WRITE)
        .ok_or_else(|| "order-write operation fixture is missing".to_owned())?;
    order_write.status = VenueOperationStatus::Warn;
    order_write.message = "cancel proof missing".to_owned();
    order_write.retry_after_ms = Some(2_000);
    order_write.latency_ms = Some(37);
    order_write.latency_p95_ms = Some(51);
    let mut evidence = request_evidence("req-live-write-http");
    evidence.path = "/fapi/v1/order".to_owned();
    evidence.request_context = vec!["symbol=BTCUSDT".to_owned()];
    order_write.evidence = Some(evidence);

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
        .unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();
    let problem = outcome
        .problems
        .iter()
        .find(|problem| problem.code == codes::HEDGE_PRE_TRADE_REJECTED)
        .ok_or_else(|| "synthesized operation problem is missing".to_owned())?;
    let details = problem
        .details
        .as_ref()
        .ok_or_else(|| "synthesized operation problem details are missing".to_owned())?;

    assert!(!guard.passed);
    assert_eq!(problem.request_id.as_deref(), Some("req-live-write-http"));
    assert_eq!(problem.retry_after_ms, Some(2_000));
    assert_eq!(details["operation"], OP_ORDER_WRITE);
    assert_eq!(details["symbol"], "BTCUSDT");
    assert_eq!(details["path"], "/fapi/v1/order");
    assert_eq!(details["latencyMs"], 37);
    assert_eq!(details["latencyP95Ms"], 51);
    Ok(())
}
