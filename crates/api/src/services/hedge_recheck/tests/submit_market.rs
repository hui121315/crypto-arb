use super::*;

#[tokio::test]
async fn cached_recheck_accepts_fresh_perp_legs() -> anyhow::Result<()> {
    let state = test_state().await?;
    let ticket = ticket(Some(StrategyKind::PerpCross), None);
    for leg in [&ticket.long_leg, &ticket.short_leg] {
        state.market_data().store_orderbook(
            orderbook(&leg.exchange, &leg.symbol),
            crate::services::market_data::MarketSource::WsPush,
        );
    }

    let rejection = orderbook_freshness_rejection(&state, &ticket, "执行前盘口复检未通过");

    assert!(rejection.is_none(), "{rejection:?}");
    Ok(())
}

#[test]
fn limit_recheck_rejects_worse_target_vwap() {
    let intent = order_intent(OrderSide::Buy, Some(100.1));
    let mut quote = leg_quote(HedgeLegRole::Long, "okx", "BTCUSDT");
    quote.open_vwap_price = Some(100.2);

    let blocker =
        leg_order_protection_blocker(&intent, &quote, OrderType::Limit).unwrap_or_default();

    assert!(blocker.contains("超出原票据保护价"), "{blocker}");
}

#[test]
fn market_recheck_uses_current_profit_proof_instead_of_stale_limit() {
    let intent = order_intent(OrderSide::Buy, Some(100.1));
    let mut quote = leg_quote(HedgeLegRole::Long, "okx", "BTCUSDT");
    quote.open_vwap_price = Some(100.2);

    assert!(leg_order_protection_blocker(&intent, &quote, OrderType::Market).is_none());
}

#[tokio::test]
async fn second_leg_recheck_uses_first_fill_and_existing_ws_cache() -> anyhow::Result<()> {
    let state = test_state().await?;
    let ticket = ticket(Some(StrategyKind::PerpCross), None);
    state.market_data().store_orderbook(
        orderbook("binance", "BTCUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );
    let mut intent = order_intent(OrderSide::Buy, Some(100.1));
    intent.quantity = 0.5;
    let fill = OrderRecord {
        intent,
        state: LiveOrderState::Filled,
        risk: None,
        identity: VenueOrderIdentity::default(),
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: Some("fill-1".to_owned()),
        message: None,
        filled_quantity: Some(0.5),
        filled_price: Some(100.4),
        filled_fee: Some(0.01),
        updated_at_ms: common::time::now_ms(),
    };

    let market = crate::services::hedge_ticket::refresh_second_leg_market(
        &state,
        &ticket,
        &fill,
        HedgeLegRole::Long,
    )
    .await;

    assert_eq!(market.long_leg.open_vwap_price, Some(100.4));
    assert_eq!(market.short_leg.open_vwap_price, Some(99.9));
    assert!((market.sizing.target_notional_usd - 100.0).abs() < 1e-9);
    assert!((market.target_base_quantity - 0.5).abs() < 1e-9);
    Ok(())
}

#[tokio::test]
async fn short_first_recheck_refreshes_long_hedge_and_preserves_short_fill() -> anyhow::Result<()> {
    let state = test_state().await?;
    let mut ticket = ticket(Some(StrategyKind::PerpCross), None);
    ticket.long_leg.open_vwap_price = Some(80.0);
    state.market_data().store_orderbook(
        orderbook("okx", "BTCUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );
    let mut intent = order_intent(OrderSide::Sell, Some(100.0));
    intent.exchange = "binance".into();
    intent.quantity = 0.25;
    let fill = OrderRecord {
        intent,
        state: LiveOrderState::Filled,
        risk: None,
        identity: VenueOrderIdentity::default(),
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: Some("fill-short".to_owned()),
        message: None,
        filled_quantity: Some(0.25),
        filled_price: Some(100.6),
        filled_fee: Some(0.01),
        updated_at_ms: common::time::now_ms(),
    };

    let market = crate::services::hedge_ticket::refresh_second_leg_market(
        &state,
        &ticket,
        &fill,
        HedgeLegRole::Short,
    )
    .await;

    assert_eq!(market.short_leg.open_vwap_price, Some(100.6));
    assert_eq!(market.long_leg.open_vwap_price, Some(100.1));
    assert!((market.target_base_quantity - 0.25).abs() < 1e-9);
    Ok(())
}
