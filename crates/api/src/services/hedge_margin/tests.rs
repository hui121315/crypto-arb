use super::*;
use shared_types::{OrderSide, OrderSource, OrderType, TimeInForce, VenueOperationStatus};

mod account_summary;
mod collateral;
mod evidence;
mod outcome;

fn assert_margin_scope(outcome: &MarginPreflightOutcome) {
    assert_eq!(outcome.scope.venues, vec!["binance"]);
    assert_eq!(outcome.scope.symbols, vec!["BTCUSDT"]);
    assert_eq!(outcome.scope.account_modes, vec!["cross"]);
    assert_eq!(
        outcome.scope.operations,
        vec![HedgePreflightOperation::MarginBalance]
    );
}

fn assert_margin_balance_row(outcome: &MarginPreflightOutcome) {
    assert_eq!(outcome.balance_rows.len(), 1);
    assert_eq!(outcome.balance_rows[0].currency, "USDT");
    assert_eq!(outcome.balance_rows[0].available, 100.0);
}

fn assert_margin_field_quality(outcome: &MarginPreflightOutcome) {
    assert!(outcome
        .field_quality
        .iter()
        .any(|row| { row.field == "equity" && row.status == AccountFieldQualityStatus::Unknown }));
}

fn assert_margin_row_health(outcome: &MarginPreflightOutcome) {
    assert_eq!(outcome.row_health.len(), 1);
    assert_eq!(
        outcome.row_health[0].subject,
        AccountFieldSubject::balance("binance", "USDT")
    );
    assert_eq!(outcome.row_health[0].source, "test");
}

fn order_intent(exchange: &str, mode: ExecutionMode, reduce_only: bool) -> OrderIntent {
    OrderIntent {
        id: format!("test-{exchange}"),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode,
        exchange: exchange.to_owned(),
        symbol: "BTCUSDT".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: format!("client-{exchange}"),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn operation_row(
    venue: &str,
    operation: &str,
    freshness_ms: Option<i64>,
    retry_after_ms: Option<u64>,
    request_id: Option<&str>,
) -> VenueOperationHealth {
    let mut problem = ApiProblem::new(codes::UPSTREAM_HTTP, "operation problem".to_owned())
        .with_retry_after_ms(retry_after_ms);
    problem.request_id = request_id.map(str::to_owned);
    VenueOperationHealth {
        venue: venue.to_owned(),
        operation: operation.to_owned(),
        status: VenueOperationStatus::Blocked,
        source: "test".to_owned(),
        message: "test".to_owned(),
        supported: Some(true),
        configured: Some(true),
        requested: Some(1),
        rows: Some(0),
        freshness_ms,
        retry_after_ms,
        latency_ms: None,
        latency_p95_ms: None,
        error: Some("operation problem".to_owned()),
        evidence: None,
        problem: Some(problem),
        observed_at_ms: 1,
    }
}

fn current_balance_operation_row(venue: &str) -> VenueOperationHealth {
    let mut row = operation_row(
        venue,
        "balance",
        Some(250),
        None,
        Some("req-balance-current"),
    );
    row.status = VenueOperationStatus::Ok;
    row.error = None;
    row.problem = None;
    row
}
