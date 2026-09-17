use super::*;
use pretty_assertions::assert_eq;

fn open_long(p: &mut SimPortfolio, symbol: &str, qty: f64, price: f64) -> SimPosition {
    p.open(OpenRequest {
        symbol: symbol.into(),
        exchange: "binance".into(),
        side: PositionSide::Long,
        quantity: qty,
        entry_price: price,
        leverage: 1.0,
        fees: 0.0,
        note: String::new(),
    })
    .unwrap()
}

#[test]
fn new_portfolio_initialized_correctly() {
    let p = SimPortfolio::new(10_000.0);
    assert_eq!(p.cash, 10_000.0);
    assert_eq!(p.position_count(), 0);
    assert_eq!(p.trade_count(), 0);
    assert!((p.total_equity() - 10_000.0).abs() < 1e-9);
    assert!((p.return_pct() - 0.0).abs() < 1e-12);
}

#[test]
fn open_position_deducts_cash() {
    let mut p = SimPortfolio::new(10_000.0);
    open_long(&mut p, "BTC", 0.1, 30_000.0);
    assert!((p.cash - 7_000.0).abs() < 1e-9);
    assert_eq!(p.position_count(), 1);
}

#[test]
fn open_with_fees_subtracts_them() {
    let mut p = SimPortfolio::new(10_000.0);
    p.open(OpenRequest {
        symbol: "BTC".into(),
        exchange: "binance".into(),
        side: PositionSide::Long,
        quantity: 0.1,
        entry_price: 30_000.0,
        leverage: 1.0,
        fees: 3.0,
        note: String::new(),
    })
    .unwrap();
    assert!((p.cash - 6_997.0).abs() < 1e-9);
}

#[test]
fn open_with_leverage_uses_fractional_margin() {
    let mut p = SimPortfolio::new(10_000.0);
    let pos = p
        .open(OpenRequest {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            side: PositionSide::Long,
            quantity: 0.1,
            entry_price: 30_000.0,
            leverage: 5.0,
            fees: 3.0,
            note: String::new(),
        })
        .unwrap();

    assert!((pos.margin_used() - 600.0).abs() < 1e-9);
    assert!((p.cash - 9_397.0).abs() < 1e-9);
    assert!((p.total_equity() - 9_997.0).abs() < 1e-9);
}

#[test]
fn open_invalid_leverage_errors() {
    let mut p = SimPortfolio::new(10_000.0);
    let err = p
        .open(OpenRequest {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            side: PositionSide::Long,
            quantity: 0.1,
            entry_price: 30_000.0,
            leverage: 0.0,
            fees: 0.0,
            note: String::new(),
        })
        .unwrap_err();

    assert!(matches!(err, SimError::InvalidLeverage(_)));
}

#[test]
fn open_insufficient_cash_errors() {
    let mut p = SimPortfolio::new(100.0);
    let err = p
        .open(OpenRequest {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            side: PositionSide::Long,
            quantity: 1.0,
            entry_price: 30_000.0,
            leverage: 1.0,
            fees: 0.0,
            note: String::new(),
        })
        .unwrap_err();
    assert!(matches!(err, SimError::InsufficientCash { .. }));
}

#[test]
fn open_invalid_quantity_errors() {
    let mut p = SimPortfolio::new(10_000.0);
    let err = p
        .open(OpenRequest {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            side: PositionSide::Long,
            quantity: -1.0,
            entry_price: 30_000.0,
            leverage: 1.0,
            fees: 0.0,
            note: String::new(),
        })
        .unwrap_err();
    assert!(matches!(err, SimError::InvalidQuantity(_)));
}

#[test]
fn close_long_profit_returns_cash() {
    let mut p = SimPortfolio::new(10_000.0);
    let pos = open_long(&mut p, "BTC", 0.1, 30_000.0);
    let trade = p.close(&pos.id, 35_000.0, 0.0).unwrap();
    assert!((p.cash - 10_500.0).abs() < 1e-9);
    assert!((trade.realized_pnl - 500.0).abs() < 1e-9);
    assert_eq!(p.position_count(), 0);
    assert_eq!(p.trade_count(), 1);
}

#[test]
fn close_short_profit_when_price_drops() {
    let mut p = SimPortfolio::new(10_000.0);
    let pos = p
        .open(OpenRequest {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            side: PositionSide::Short,
            quantity: 0.1,
            entry_price: 30_000.0,
            leverage: 1.0,
            fees: 0.0,
            note: String::new(),
        })
        .unwrap();
    let trade = p.close(&pos.id, 28_000.0, 0.0).unwrap();
    assert!((trade.realized_pnl - 200.0).abs() < 1e-9);
}

#[test]
fn close_with_close_fees_subtracts() {
    let mut p = SimPortfolio::new(10_000.0);
    let pos = open_long(&mut p, "BTC", 0.1, 30_000.0);
    let trade = p.close(&pos.id, 35_000.0, 5.0).unwrap();
    assert!((trade.realized_pnl - 495.0).abs() < 1e-9);
}

#[test]
fn close_missing_position_errors() {
    let mut p = SimPortfolio::new(10_000.0);
    let err = p.close("nope", 100.0, 0.0).unwrap_err();
    assert!(matches!(err, SimError::NotFound(_)));
}

#[test]
fn update_marks_changes_current_price() {
    let mut p = SimPortfolio::new(10_000.0);
    open_long(&mut p, "BTC", 0.1, 30_000.0);
    let mut prices = HashMap::new();
    prices.insert("BTC".into(), 32_000.0);
    p.update_marks(&prices);
    let pos = p.positions.values().next().unwrap();
    assert!((pos.current_price - 32_000.0).abs() < 1e-9);
    assert!((pos.unrealized_pnl() - 200.0).abs() < 1e-9);
}

#[test]
fn total_equity_includes_unrealized() {
    let mut p = SimPortfolio::new(10_000.0);
    open_long(&mut p, "BTC", 0.1, 30_000.0);
    p.update_mark("BTC", 32_000.0);
    assert!((p.total_positions_value() - 3_200.0).abs() < 1e-9);
    assert!((p.total_equity() - 10_200.0).abs() < 1e-9);
    assert!((p.return_pct() - 0.02).abs() < 1e-9);
}

#[test]
fn multiple_positions_tracked_independently() {
    let mut p = SimPortfolio::new(100_000.0);
    open_long(&mut p, "BTC", 0.1, 30_000.0);
    open_long(&mut p, "ETH", 1.0, 2_000.0);
    assert_eq!(p.position_count(), 2);
    assert!((p.cash - 95_000.0).abs() < 1e-9);
}
