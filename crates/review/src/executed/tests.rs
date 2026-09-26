use super::*;
use shared_types::{ExecutionMode, OrderIntent, OrderType};

#[test]
fn normalizes_net_pnl_from_components() {
    let trade = normalize_net_pnl(ExecutedTrade {
        id: "t".into(),
        strategy: StrategyKind::PerpCross,
        symbol: "BTC".into(),
        long_venue: "OKX".into(),
        short_venue: "HL".into(),
        opened_at_ms: 0,
        closed_at_ms: None,
        holding_minutes: None,
        gross_pnl_usd: 100.0,
        fee_usd: 8.0,
        funding_usd: 14.0,
        slippage_usd: 6.0,
        net_pnl_usd: 0.0,
        evidence: ReviewPnlEvidence::default(),
        actual_fields: Vec::new(),
        estimated_fields: Vec::new(),
        missing_fields: Vec::new(),
        long_orders: Vec::new(),
        short_orders: Vec::new(),
    });

    assert_eq!(trade.net_pnl_usd, 106.0);
}

#[test]
fn groups_hedge_orders_into_executed_trade() {
    let rows = vec![
        order("hedge-1-long", OrderSide::Buy, "binance", 100.0),
        order("hedge-1-short", OrderSide::Sell, "okx", 101.0),
    ];

    let trades = executed_from_orders(&rows, 100_000, 1);

    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].id, "hedge-1");
    assert_eq!(trades[0].long_venue, "binance");
    assert_eq!(trades[0].short_venue, "okx");
    assert_eq!(trades[0].long_orders.len(), 1);
    assert_eq!(trades[0].short_orders.len(), 1);
    assert!((trades[0].gross_pnl_usd - 1.0).abs() < 1e-12);
    assert!((trades[0].fee_usd - 0.1005).abs() < 1e-12);
    assert!((trades[0].net_pnl_usd - 0.8995).abs() < 1e-12);
}

#[test]
fn performance_keeps_live_paper_and_unknown_environments_separate() {
    use shared_types::{ExecutionEnvironment, ReviewPnlField};
    let records = [
        order("env-long", OrderSide::Buy, "binance", 100.0),
        order("env-short", OrderSide::Sell, "okx", 101.0),
    ];
    let mut live = executed_from_orders(&records, 100_000, 1).remove(0);
    live.actual_fields = vec![ReviewPnlField::Net];
    live.estimated_fields.clear();
    live.missing_fields.clear();
    live.net_pnl_usd = -2.0;
    for order in live.long_orders.iter_mut().chain(&mut live.short_orders) {
        order.intent.mode = ExecutionMode::Live;
    }
    let mut paper = live.clone();
    paper.net_pnl_usd = 1_000.0;
    paper.long_orders[0].intent.mode = ExecutionMode::DryRun;
    paper.short_orders[0].intent.mode = ExecutionMode::Testnet;
    let mut estimated = live.clone();
    estimated.net_pnl_usd = 4.0;
    estimated.actual_fields.clear();
    estimated.estimated_fields = vec![ReviewPnlField::Net];
    let mut missing = live.clone();
    missing.net_pnl_usd = 900.0;
    missing.missing_fields = vec![ReviewPnlField::Net];
    let mut mixed = live.clone();
    mixed.net_pnl_usd = 700.0;
    mixed.short_orders[0].intent.mode = ExecutionMode::DryRun;
    let mut one_leg = live.clone();
    one_leg.net_pnl_usd = 600.0;
    one_leg.short_orders.clear();
    assert_eq!(mixed.execution_environment(), None);
    assert_eq!(one_leg.execution_environment(), None);
    let rows = crate::compute_performance_by_environment(
        &[live, paper, estimated, missing, mixed, one_leg], StrategyKind::SpotPerp,
    );
    assert_eq!(rows.len(), 3);
    let live = rows.iter().find(|row| row.execution_environment == Some(ExecutionEnvironment::Live)).unwrap();
    assert_eq!((live.total_trades_30d, live.actual_trades_30d, live.estimated_trades_30d, live.skipped_trades_30d), (3, 1, 1, 1));
    assert_eq!((live.net_pnl_30d_usd, live.estimated_net_pnl_30d_usd, live.hit_rate_pct), (-2.0, 4.0, 0.0));
    let paper = rows.iter().find(|row| row.execution_environment == Some(ExecutionEnvironment::Paper)).unwrap();
    assert_eq!((paper.actual_trades_30d, paper.net_pnl_30d_usd), (1, 1_000.0));
    let unknown = rows.iter().find(|row| row.execution_environment.is_none()).unwrap();
    assert_eq!(unknown.total_trades_30d, 2);
    assert!(crate::compute_performance_by_environment(&[], StrategyKind::PerpCross).is_empty());
}

#[test]
fn pages_executed_groups_before_materializing_trades() {
    let rows = vec![
        order("hedge-1-long", OrderSide::Buy, "binance", 100.0),
        order("hedge-1-short", OrderSide::Sell, "okx", 101.0),
        older_order("hedge-2-long", OrderSide::Buy, "gate", 90.0),
        older_order("hedge-2-short", OrderSide::Sell, "bybit", 91.0),
    ];

    let page = executed_page_from_orders(&rows, 100_000, 1, 1, 1);

    assert_eq!(page.total_rows, 2);
    assert_eq!(page.group_ids, ["hedge-1", "hedge-2"]);
    assert_eq!(page.rows.len(), 1);
    assert_eq!(page.rows[0].id, "hedge-2");
}

#[test]
fn keeps_strategy_from_order_intent() {
    let mut rows = vec![
        order("hedge-2-long", OrderSide::Buy, "binance", 100.0),
        order("hedge-2-short", OrderSide::Sell, "okx", 101.0),
    ];
    rows[0].intent.strategy = Some(StrategyKind::SpotCross);

    let trades = executed_from_orders(&rows, 100_000, 1);

    assert_eq!(trades[0].strategy, StrategyKind::SpotCross);
}

#[test]
fn skips_ack_and_partial_orders_until_filled() {
    let working_states = [
        LiveOrderState::Submitted,
        LiveOrderState::Accepted,
        LiveOrderState::PartiallyFilled,
    ];
    for state in working_states {
        let mut rows = vec![
            order("hedge-ack-long", OrderSide::Buy, "binance", 100.0),
            order("hedge-ack-short", OrderSide::Sell, "okx", 101.0),
        ];
        for row in &mut rows {
            row.state = state;
        }

        let trades = executed_from_orders(&rows, 100_000, 1);

        assert!(trades.is_empty(), "{state:?} must not be executed");
    }
}

#[test]
fn skips_broken_hedge_groups_without_both_legs() {
    let mut rows = vec![
        order("hedge-3-long", OrderSide::Buy, "binance", 100.0),
        order("hedge-3-unwind", OrderSide::Sell, "binance", 99.0),
    ];
    rows[1].intent.reduce_only = true;

    let trades = executed_from_orders(&rows, 100_000, 1);

    assert!(trades.is_empty());
}

#[test]
fn missing_order_price_marks_legacy_review_pnl_as_missing() {
    let mut rows = vec![
        order("hedge-missing-price-long", OrderSide::Buy, "binance", 100.0),
        order("hedge-missing-price-short", OrderSide::Sell, "okx", 101.0),
    ];
    rows[0].filled_price = None;
    rows[0].intent.price = None;

    let trades = executed_from_orders(&rows, 100_000, 1);

    assert_eq!(trades.len(), 1);
    assert_eq!(trades[0].gross_pnl_usd, 0.0);
    assert_eq!(trades[0].fee_usd, 0.0);
    assert_eq!(trades[0].net_pnl_usd, 0.0);
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Gross));
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Fee));
    assert!(trades[0].missing_fields.contains(&ReviewPnlField::Net));
    assert!(!trades[0].estimated_fields.contains(&ReviewPnlField::Gross));
    assert!(!trades[0].estimated_fields.contains(&ReviewPnlField::Fee));
    assert!(!trades[0].estimated_fields.contains(&ReviewPnlField::Net));
}

fn order(id: &str, side: OrderSide, exchange: &str, price: f64) -> OrderRecord {
    order_with_created_at(id, side, exchange, price, 99_000)
}

fn older_order(id: &str, side: OrderSide, exchange: &str, price: f64) -> OrderRecord {
    order_with_created_at(id, side, exchange, price, 98_000)
}

fn order_with_created_at(
    id: &str,
    side: OrderSide,
    exchange: &str,
    price: f64,
    created_at_ms: i64,
) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: id.into(),
            source: OrderSource::ArbitragePreview,
            strategy: Some(StrategyKind::SpotPerp),
            mode: ExecutionMode::Testnet,
            exchange: exchange.into(),
            symbol: "BTCUSDT".into(),
            side,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(price),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: id.into(),
            client_order_id_policy: None,
            created_at_ms,
        },
        state: LiveOrderState::Filled,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: Some(format!("ex-{id}")),
        message: None,
        filled_quantity: Some(1.0),
        filled_price: Some(price),
        filled_fee: None,
        updated_at_ms: 99_500,
    }
}
