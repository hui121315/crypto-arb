use super::*;

#[test]
fn pr_ak_leg_specs_bind_each_venue_market_symbol_and_price() {
    let mut opportunity = opportunity_with_prices(Some(90.0), Some(91.0));
    opportunity.long_leg_market_evidence = Some(OpportunityLegMarketEvidence {
        venue: "binance".into(),
        symbol: "BTCUSDT".into(),
        price: Some(100.0),
        health: fresh_market_health(),
    });
    opportunity.short_leg_market_evidence = Some(OpportunityLegMarketEvidence {
        venue: "okx".into(),
        symbol: "BTC-USDT-SWAP".into(),
        price: Some(101.0),
        health: fresh_market_health(),
    });

    let long = LegSpec::from_opp(&opportunity, HedgeLegRole::Long);
    let short = LegSpec::from_opp(&opportunity, HedgeLegRole::Short);

    assert_eq!(long.symbol, "BTCUSDT");
    assert_eq!(long.fallback_price, Some(100.0));
    assert_eq!(short.symbol, "BTC-USDT-SWAP");
    assert_eq!(short.fallback_price, Some(101.0));
}

#[test]
fn execution_leg_specs_select_spot_and_perp_books_from_strategy_evidence() {
    let mut opportunity = opportunity_with_prices(Some(100.0), Some(101.0));
    opportunity.strategy_kind = Some(StrategyKind::SpotPerp);
    opportunity.spot_leg_mode = Some(SpotLegMode::BuySpot);

    assert_eq!(
        LegSpec::from_opp(&opportunity, HedgeLegRole::Long).book_kind,
        LegBookKind::Spot
    );
    assert_eq!(
        LegSpec::from_opp(&opportunity, HedgeLegRole::Short).book_kind,
        LegBookKind::Perp
    );

    opportunity.strategy_kind = Some(StrategyKind::SpotCross);
    opportunity.spot_leg_mode = Some(SpotLegMode::SellInventory);
    assert_eq!(
        LegSpec::from_opp(&opportunity, HedgeLegRole::Long).book_kind,
        LegBookKind::Spot
    );
    assert_eq!(
        LegSpec::from_opp(&opportunity, HedgeLegRole::Short).book_kind,
        LegBookKind::Spot
    );
}

#[tokio::test]
async fn selected_spot_perp_detail_reads_each_leg_from_the_correct_cache() -> anyhow::Result<()> {
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    let state = AppState::new(config).await?;
    let mut opportunity = opportunity_with_prices(Some(100.0), Some(101.0));
    opportunity.strategy_kind = Some(StrategyKind::SpotPerp);
    opportunity.spot_leg_mode = Some(SpotLegMode::BuySpot);

    let mut long_book = book(vec![[99.9, 1.0]], vec![[100.1, 1.0]]);
    long_book.exchange = opportunity.long_exchange.clone();
    state.market_data().store_spot_orderbook(
        &opportunity.long_exchange,
        long_book,
        MarketSource::WsPush,
    );
    let mut short_book = book(vec![[100.9, 1.0]], vec![[101.1, 1.0]]);
    short_book.exchange = opportunity.short_exchange.clone();
    state
        .market_data()
        .store_orderbook(short_book, MarketSource::WsPush);

    let (long, short) = cached_opportunity_orderbooks(&state, &opportunity, common::time::now_ms());

    assert_eq!(long.read.quality, MarketQuality::Fresh);
    assert_eq!(long.read.source, MarketSource::WsPush);
    assert_eq!(short.read.quality, MarketQuality::Fresh);
    assert_eq!(short.read.source, MarketSource::WsPush);
    Ok(())
}

fn fresh_market_health() -> MarketDataHealth {
    MarketDataHealth {
        quality: MarketDataQuality::Fresh,
        source: MarketDataSourceKind::WsPush,
        freshness_ms: Some(0),
        retry_after_ms: None,
        last_error: None,
        observed_at_ms: 1,
        coverage: None,
        problem: None,
    }
}
