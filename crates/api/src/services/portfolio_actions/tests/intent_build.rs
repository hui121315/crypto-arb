use super::*;

#[test]
fn close_intent_sells_long_reduce_only_market() -> Result<(), AppError> {
    let row = position(PositionSide::Long);
    let intent = close_intent(&row, ExecutionMode::Live)?;

    assert_eq!(intent.exchange, "binance");
    assert_eq!(intent.symbol, "MUUSDT");
    assert_eq!(intent.side, OrderSide::Sell);
    assert_eq!(intent.order_type, OrderType::Market);
    assert!(intent.reduce_only);
    assert_eq!(intent.price, Some(664.25));
    assert_eq!(intent.mode, ExecutionMode::Live);
    assert!(intent.client_order_id.len() <= 36);
    Ok(())
}

#[test]
fn close_intent_buys_short_and_normalizes_leverage() -> Result<(), AppError> {
    let mut row = position(PositionSide::Short);
    row.leverage = f64::NAN;
    let intent = close_intent(&row, ExecutionMode::DryRun)?;

    assert_eq!(intent.side, OrderSide::Buy);
    assert_eq!(intent.leverage, 1.0);
    assert_eq!(intent.mode, ExecutionMode::DryRun);
    Ok(())
}

#[test]
fn close_intent_normalizes_builder_venue_for_routing() -> Result<(), AppError> {
    let mut row = position(PositionSide::Long);
    row.venue = " Hyperliquid:XYZ ".into();

    let intent = close_intent(&row, ExecutionMode::Live)?;

    assert_eq!(intent.exchange, "hyperliquid:xyz");
    Ok(())
}

#[test]
fn close_order_plan_uses_the_shared_live_order_preflight_contract() -> Result<(), AppError> {
    let intent = close_intent(&position(PositionSide::Long), ExecutionMode::Live)?;
    let plan = close_order_plan(&intent);

    assert_eq!(plan.exchange, intent.exchange);
    assert_eq!(plan.symbol, intent.symbol);
    assert_eq!(plan.product, FeeProduct::Perp);
    assert_eq!(plan.effective_order_type, OrderType::Market);
    assert!(plan.market_order_style.is_none());
    assert!(plan.submission_context().market_order_style.is_none());
    Ok(())
}
