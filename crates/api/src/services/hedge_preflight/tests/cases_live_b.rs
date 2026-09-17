use super::super::*;
use super::fixtures::*;

#[test]
fn live_operation_health_guard_scopes_rebuilt_runtime_to_ticket_venues() {
    let long = plan("binance", "BTCUSDT", Vec::new());
    let short = plan("okx", "BTC-USDT-SWAP", Vec::new());
    let mut rows = full_live_operation_rows("binance");
    rows.extend(full_live_operation_rows("okx"));
    let mut unrelated = operation_row("bybit", OP_ORDER_WRITE, VenueOperationStatus::Blocked);
    unrelated.problem = Some(
        ApiProblem::new("UNRELATED_VENUE_FAILURE", "must not reach ticket evidence")
            .with_request_id(Some("req-bybit-unrelated".to_owned())),
    );
    rows.push(unrelated);

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long, &short], &rows)
        .unwrap_or_else(missing_guard);
    let outcome = guard.preflight_outcome.unwrap_or_default();

    assert!(guard.passed);
    assert_eq!(outcome.scope.venues, vec!["binance", "okx"]);
    assert_eq!(outcome.observed_venues, vec!["binance", "okx"]);
    assert_eq!(outcome.row_health.len(), 12);
    assert!(outcome
        .row_health
        .iter()
        .all(|row| row.subject.venue.as_deref() != Some("bybit")));
    assert!(outcome.problems.is_empty());
    assert_ne!(outcome.request_id.as_deref(), Some("req-bybit-unrelated"));
}

#[test]
fn live_operation_health_guard_accepts_family_private_read_only() {
    let long = plan("hyperliquid:xyz", "MU-USDC", Vec::new());
    let mut rows = full_live_operation_rows("hyperliquid:xyz")
        .into_iter()
        .filter(|row| row.operation != OP_PRIVATE_READ)
        .collect::<Vec<_>>();
    rows.push(operation_row(
        "hyperliquid",
        OP_PRIVATE_READ,
        VenueOperationStatus::Ok,
    ));

    let guard = live_operation_health_guard(ExecutionMode::Live, &[&long], &rows)
        .unwrap_or_else(missing_guard);

    assert!(guard.passed);
}
