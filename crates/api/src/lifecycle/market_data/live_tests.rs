use super::*;
use shared_types::{
    MarketDataCoverage, MarketDataHealth, MarketDataQuality, MarketDataSourceKind,
    OpportunityLegMarketEvidence, OpportunityQuoteConversion, PositionOrigin, PositionSeverity,
    PositionSide,
};

#[test]
fn perp_plan_deduplicates_and_normalizes_venue_symbols() {
    let mut plan = LiveRequestPlan::default();
    push_leg(&mut plan, LegMarket::Perp, " Binance ", " btcusdt ");
    push_leg(&mut plan, LegMarket::Perp, "binance", "BTCUSDT");
    push_leg(&mut plan, LegMarket::Perp, "unknown", "ETHUSDT");

    assert_eq!(plan.perp.get("binance"), Some(&vec!["BTCUSDT".to_owned()]));
    assert_eq!(
        plan.funding.get("binance"),
        Some(&vec!["BTCUSDT".to_owned()])
    );
    assert_eq!(plan.perp.len(), 1);
    assert!(plan.spot.is_empty());
}

#[test]
fn perp_plan_enforces_per_venue_bound() {
    let mut plan = BTreeMap::new();
    for index in 0..(WS_LIVE_SYMBOLS_PER_VENUE + 4) {
        push_market_symbol(&mut plan, "okx", &format!("TOKEN{index}"));
    }

    assert_eq!(plan["okx"].len(), WS_LIVE_SYMBOLS_PER_VENUE);
}

#[test]
fn open_positions_seed_live_funding_and_mark_requests() {
    let mut plan = LiveRequestPlan::default();
    append_position_requests(
        &mut plan,
        &[
            position(" Binance ", " sol "),
            position("binance", "SOL"),
            position("bitget", "ETH"),
        ],
    );

    assert_eq!(plan.funding["binance"], vec!["SOL"]);
    assert_eq!(plan.funding["bitget"], vec!["ETH"]);
    assert_eq!(plan.marks["binance"], vec!["SOL"]);
    assert_eq!(plan.marks["bitget"], vec!["ETH"]);
    assert!(plan.perp.is_empty());
    assert!(plan.spot.is_empty());
}

#[test]
fn open_positions_use_funding_without_consuming_opportunity_ticker_budget() {
    let mut opportunities = LiveRequestPlan::default();
    for index in 0..WS_LIVE_SYMBOLS_PER_VENUE {
        push_leg(
            &mut opportunities,
            LegMarket::Perp,
            "binance",
            &format!("TOKEN{index}"),
        );
    }
    let mut plan = LiveRequestPlan::default();
    append_position_requests(&mut plan, &[position("binance", "SOL")]);
    append_plan(&mut plan, &opportunities);

    assert_eq!(
        plan.funding["binance"].first().map(String::as_str),
        Some("SOL")
    );
    assert_eq!(plan.marks["binance"], vec!["SOL"]);
    assert_eq!(plan.perp["binance"].len(), WS_LIVE_SYMBOLS_PER_VENUE);
    let last_symbol = format!("TOKEN{}", WS_LIVE_SYMBOLS_PER_VENUE - 1);
    assert_eq!(
        plan.perp["binance"].last().map(String::as_str),
        Some(last_symbol.as_str())
    );
}

#[test]
fn strategy_market_plan_keeps_perp_and_spot_legs_separate() {
    assert_eq!(
        opportunity_leg_markets(Some(StrategyKind::PerpCross), None),
        (LegMarket::Perp, LegMarket::Perp)
    );
    assert_eq!(
        opportunity_leg_markets(Some(StrategyKind::SpotPerp), Some(SpotLegMode::BuySpot)),
        (LegMarket::Spot, LegMarket::Perp)
    );
    assert_eq!(
        opportunity_leg_markets(
            Some(StrategyKind::CrossSpotPerp),
            Some(SpotLegMode::SellInventory),
        ),
        (LegMarket::Perp, LegMarket::Spot)
    );
    assert_eq!(
        opportunity_leg_markets(Some(StrategyKind::SpotCross), None),
        (LegMarket::Spot, LegMarket::Spot)
    );
    assert_eq!(
        opportunity_leg_markets(Some(StrategyKind::SpotPerp), None),
        (LegMarket::None, LegMarket::None)
    );
    assert_eq!(
        leg_request_symbol(LegMarket::Spot, "BTC", Some("BTC/USDT")),
        Some("BTC/USDT")
    );
    assert_eq!(leg_request_symbol(LegMarket::Spot, "BTC", None), None);
    assert_eq!(
        leg_request_symbol(LegMarket::Perp, "BTC", Some("BTC/USDC")),
        Some("BTC")
    );
}

#[test]
fn live_candidate_plan_reuses_venues_until_the_symbol_bound() {
    let mut plan = LiveRequestPlan::default();
    for index in 0..(WS_LIVE_SYMBOLS_PER_VENUE + 4) {
        let symbol = format!("TOKEN{index}");
        push_leg(&mut plan, LegMarket::Perp, "binance", &symbol);
        push_leg(&mut plan, LegMarket::Perp, "bitget", &symbol);
    }

    assert_eq!(plan.perp["binance"].len(), WS_LIVE_SYMBOLS_PER_VENUE);
    assert_eq!(plan.perp["bitget"].len(), WS_LIVE_SYMBOLS_PER_VENUE);
    assert_eq!(plan.funding["binance"].len(), WS_LIVE_SYMBOLS_PER_VENUE);
}

#[test]
fn cross_quote_candidate_subscribes_the_exact_fx_market_on_demand() -> serde_json::Result<()> {
    let mut row = opportunity_with_spot_evidence(StrategyKind::CrossSpotPerp, "MU")?;
    row.quote_conversions = vec![OpportunityQuoteConversion {
        from_quote: "USDC".into(),
        to_quote: "USDT".into(),
        rate: 0.999,
        venue: "bitget".into(),
        symbol: "USDCUSDT".into(),
        market_evidence: None,
    }];
    let mut plan = LiveRequestPlan::default();
    for index in 0..WS_LIVE_OTHER_SYMBOLS_PER_VENUE {
        assert!(push_market_symbol_with_limit(
            &mut plan.spot,
            "bitget",
            &format!("TOKEN{index}USDT"),
            WS_LIVE_OTHER_SYMBOLS_PER_VENUE,
        ));
    }

    append_opportunity_request(&mut plan, &row, WS_LIVE_OTHER_SYMBOLS_PER_VENUE);

    assert_eq!(plan.spot["binance"], vec!["MU/USDT"]);
    assert_eq!(
        plan.spot["bitget"].last().map(String::as_str),
        Some("USDCUSDT")
    );
    assert_eq!(
        plan.spot["bitget"].len(),
        WS_LIVE_OTHER_SYMBOLS_PER_VENUE + 1
    );
    assert_eq!(plan.perp["bitget"], vec!["MU"]);
    Ok(())
}

#[path = "live_tests/planning.rs"]
mod planning_tests;

fn position(venue: &str, symbol: &str) -> PositionRow {
    PositionRow {
        venue: venue.to_owned(),
        symbol: symbol.to_owned(),
        origin: PositionOrigin::AccountPrivate,
        side: PositionSide::Long,
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 100.0,
        leverage: 1.0,
        unrealized_pnl_usd: 0.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        funding_rate_8h: 0.0,
        funding_rate_verified: false,
        maintenance_margin_ratio: 0.0,
        pair_evidence: None,
        paired_with: None,
        margin_usd: 100.0,
        severity: PositionSeverity::Unknown,
        seconds_until_funding: None,
    }
}

fn opportunity(
    strategy: StrategyKind,
    symbol: &str,
) -> serde_json::Result<ArbitrageOpportunityDto> {
    let strategy = strategy.as_query_value();
    serde_json::from_value(serde_json::json!({
        "id": format!("{strategy}-{symbol}"),
        "symbol": symbol,
        "type": "cross_exchange",
        "typeLabel": "test",
        "longExchange": "binance",
        "shortExchange": "bitget",
        "spread8h": 0.0,
        "longRate8h": 0.0,
        "shortRate8h": 0.0,
        "longRate": 0.0,
        "shortRate": 0.0,
        "singleYield": 0.001,
        "netSingleYield": 0.001,
        "rawSingleYield": 0.001,
        "settlementInterval": 8,
        "riskAdjustedYield": 0.0,
        "tradingCostRate": 0.0,
        "minHoldingPeriods": 1,
        "riskLevel": "low",
        "volatility": 0.0,
        "sharpeRatio": 0.0,
        "score": 0.0,
        "recommendation": "hold",
        "optimalPosition": 0.0,
        "maxPosition": 0.0,
        "liquidityScore": 0.0,
        "volume24h": 0.0,
        "dataSource": "test",
        "confidence": 0.0,
        "updatedAt": "2026-07-31T00:00:00Z",
        "longFundingInterval": 8,
        "shortFundingInterval": 8,
        "strategyKind": strategy
    }))
}

fn opportunity_with_spot_evidence(
    strategy: StrategyKind,
    symbol: &str,
) -> serde_json::Result<ArbitrageOpportunityDto> {
    let mut row = opportunity(strategy, symbol)?;
    row.long_leg_market_evidence = Some(market_evidence("binance", symbol));
    row.short_leg_market_evidence = Some(market_evidence("bitget", symbol));
    if matches!(
        strategy,
        StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp
    ) {
        row.spot_leg_mode = Some(SpotLegMode::BuySpot);
    }
    Ok(row)
}

fn market_evidence(venue: &str, symbol: &str) -> OpportunityLegMarketEvidence {
    OpportunityLegMarketEvidence {
        venue: venue.to_owned(),
        symbol: format!("{symbol}/USDT"),
        price: Some(1.0),
        health: MarketDataHealth {
            quality: MarketDataQuality::Fresh,
            source: MarketDataSourceKind::WsPush,
            freshness_ms: Some(0),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: Some(MarketDataCoverage::new(1, 1)),
            problem: None,
        },
    }
}

#[path = "live_tests/venue.rs"]
mod venue_tests;
