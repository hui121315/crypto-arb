use super::super::*;
use super::fixtures::*;

#[test]
fn live_operation_health_guard_skips_non_live_mode() {
    let long = plan("binance", "BTCUSDT", Vec::new());

    assert!(live_operation_health_guard(ExecutionMode::DryRun, &[&long], &[]).is_none());
}

#[test]
fn live_operation_health_guard_accepts_restart_rebuilt_runtime() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let mut rows = full_live_operation_rows("binance");
    rows.extend([
        operation_row(
            "binance",
            "credential_probe:order_permission",
            VenueOperationStatus::Unknown,
        ),
        operation_row(
            "binance",
            "private_ws_order_stream",
            VenueOperationStatus::Unknown,
        ),
        operation_row(
            "binance",
            "private_ws_account_stream",
            VenueOperationStatus::Unknown,
        ),
        operation_row(
            "binance",
            "order_finality",
            VenueOperationStatus::Blocked,
        ),
        operation_row(
            "binance",
            "rest_orderbooks",
            VenueOperationStatus::Blocked,
        ),
    ]);

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
        .unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(guard.passed);
    assert_eq!(outcome.status, HedgePreflightStatus::Passed);
    assert_eq!(outcome.row_health.len(), 6);
    assert_eq!(
        outcome.scope.operations,
        vec![
            HedgePreflightOperation::PrivateRead,
            HedgePreflightOperation::MarginBalance,
            HedgePreflightOperation::OrderWrite,
            HedgePreflightOperation::Positions,
            HedgePreflightOperation::PrivateWs,
        ]
    );
}

#[test]
fn live_operation_health_guard_blocks_missing_private_read_runtime() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let rows = full_live_operation_rows("binance")
        .into_iter()
        .filter(|row| row.operation != OP_PRIVATE_READ)
        .collect::<Vec<_>>();

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
        .unwrap_or_else(missing_guard);

    assert!(!guard.passed);
    assert!(guard.detail.contains("缺少私有 REST运行态证据"));
}
