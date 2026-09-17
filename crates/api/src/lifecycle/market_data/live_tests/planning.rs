use super::*;

#[test]
fn opportunity_plan_refreshes_on_a_new_generation_without_waiting_five_seconds() {
    assert!(opportunity_plan_due(2, 11, 10));
    assert!(!opportunity_plan_due(2, 10, 10));
    assert!(opportunity_plan_due(WS_LIVE_PLAN_EVERY_TICKS, 10, 10));
}

#[test]
fn unchanged_generations_reuse_the_existing_live_request_plan() {
    assert!(!live_request_plan_due(false, false));
    assert!(live_request_plan_due(true, false));
    assert!(live_request_plan_due(false, true));
}

#[test]
fn unchanged_opportunity_symbols_advance_version_without_replacing_the_plan() {
    let mut current = VersionedOpportunityPlan {
        version: 7,
        requests: LiveRequestPlan::default(),
    };
    push_leg(&mut current.requests, LegMarket::Perp, "binance", "BTCUSDT");
    let same_requests = current.requests.clone();

    assert!(!current.replace(VersionedOpportunityPlan {
        version: 8,
        requests: same_requests,
    }));
    assert_eq!(current.version, 8);

    let mut changed_requests = current.requests.clone();
    push_leg(&mut changed_requests, LegMarket::Perp, "binance", "ETHUSDT");
    assert!(current.replace(VersionedOpportunityPlan {
        version: 9,
        requests: changed_requests,
    }));
    assert_eq!(current.version, 9);
    assert!(current.requests.perp["binance"].contains(&"ETHUSDT".into()));
}

#[test]
fn discovery_tickers_seed_bounded_ws_recovery_when_opportunities_are_empty() {
    let rows = vec![
        discovery_ticker("binance", "BTC", 1_000_000.0),
        discovery_ticker("bitget", "BTC", 900_000.0),
        discovery_ticker("okx", "BTC", 800_000.0),
        discovery_ticker("binance", "SOL", 700_000.0),
    ];
    let mut plan = LiveRequestPlan::default();

    append_discovery_perp_requests(&mut plan, &rows);

    assert_eq!(plan.perp["binance"], vec!["BTC"]);
    assert_eq!(plan.perp["bitget"], vec!["BTC"]);
    assert_eq!(plan.perp["okx"], vec!["BTC"]);
    assert_eq!(plan.funding["binance"], vec!["BTC"]);
    assert!(!plan.perp["binance"].contains(&"SOL".to_owned()));
}

#[test]
fn empty_discovery_cache_starts_a_small_ws_bootstrap_plan() {
    let mut plan = LiveRequestPlan::default();

    append_bootstrap_perp_requests(&mut plan);

    assert_eq!(plan.perp.len(), WS_PREWARM_VENUES.len() - 1);
    for venue in WS_PREWARM_VENUES
        .iter()
        .filter(|venue| **venue != VenueId::GateCrossEx)
    {
        assert_eq!(
            plan.perp[venue.as_str()],
            WS_DISCOVERY_BOOTSTRAP_SYMBOLS.map(str::to_owned)
        );
    }
    assert!(!plan.perp.contains_key(VenueId::GateCrossEx.as_str()));
    assert!(plan.funding.is_empty());
    assert!(plan.spot.is_empty());
    assert!(plan.marks.is_empty());
}

fn discovery_ticker(exchange: &str, symbol: &str, volume_24h: f64) -> TickerInfo {
    TickerInfo {
        exchange: exchange.to_owned(),
        symbol: symbol.to_owned(),
        bid: 99.0,
        ask: 100.0,
        last: 99.5,
        volume_24h,
        timestamp: 1,
    }
}

#[test]
fn non_p0_rows_do_not_claim_the_bounded_market_budget() -> serde_json::Result<()> {
    let mut rows = (0..(WS_LIVE_SYMBOLS_PER_VENUE + 4))
        .map(|index| opportunity(StrategyKind::FundingCarry, &format!("OTHER{index}")))
        .collect::<serde_json::Result<Vec<_>>>()?;
    rows.push(opportunity(StrategyKind::PerpCross, "PRIORITY")?);
    let mut plan = LiveRequestPlan::default();

    append_opportunity_requests(&mut plan, &rows);

    assert_eq!(plan.perp["binance"].len(), 1);
    assert!(plan.perp["binance"].contains(&"PRIORITY".to_owned()));
    assert!(plan.perp["bitget"].contains(&"PRIORITY".to_owned()));
    Ok(())
}

#[test]
fn non_positive_rows_do_not_start_exact_ws_confirmation() -> serde_json::Result<()> {
    let mut no_edge = opportunity(StrategyKind::PerpCross, "NOEDGE")?;
    no_edge.net_single_yield = 0.0;
    let mut invalid = opportunity(StrategyKind::SpotCross, "INVALID")?;
    invalid.net_single_yield = f64::NAN;
    let mut plan = LiveRequestPlan::default();

    append_opportunity_requests(&mut plan, &[no_edge, invalid]);

    assert!(plan.is_empty());
    Ok(())
}

#[test]
fn other_frontend_strategies_share_the_ws_budget() -> serde_json::Result<()> {
    let rows = vec![
        opportunity(StrategyKind::PerpPriceSpread, "PERPPRICE")?,
        opportunity_with_spot_evidence(StrategyKind::SpotPerp, "SPOTPERP")?,
        opportunity_with_spot_evidence(StrategyKind::CrossSpotPerp, "CROSSSPOT")?,
        opportunity_with_spot_evidence(StrategyKind::SpotCross, "SPOTCROSS")?,
    ];
    let mut plan = LiveRequestPlan::default();

    append_opportunity_requests(&mut plan, &rows);

    assert!(plan.perp["binance"].contains(&"PERPPRICE".to_owned()));
    assert!(plan.perp["bitget"].contains(&"PERPPRICE".to_owned()));
    assert!(plan.spot["binance"].contains(&"SPOTPERP/USDT".to_owned()));
    assert!(plan.perp["bitget"].contains(&"SPOTPERP".to_owned()));
    assert!(plan.spot["binance"].contains(&"CROSSSPOT/USDT".to_owned()));
    assert!(plan.perp["bitget"].contains(&"CROSSSPOT".to_owned()));
    assert!(plan.spot["binance"].contains(&"SPOTCROSS/USDT".to_owned()));
    assert!(plan.spot["bitget"].contains(&"SPOTCROSS/USDT".to_owned()));
    Ok(())
}

#[test]
fn funding_prewarm_is_wider_than_the_high_frequency_price_budget() -> serde_json::Result<()> {
    let rows = (0..(WS_LIVE_OTHER_SYMBOLS_PER_VENUE + 3))
        .map(|index| {
            opportunity_with_spot_evidence(StrategyKind::SpotPerp, &format!("BASIS{index}"))
        })
        .collect::<serde_json::Result<Vec<_>>>()?;
    let mut plan = LiveRequestPlan::default();

    append_opportunity_requests(&mut plan, &rows);

    assert_eq!(plan.spot["binance"].len(), WS_LIVE_OTHER_SYMBOLS_PER_VENUE);
    assert_eq!(plan.perp["bitget"].len(), WS_LIVE_OTHER_SYMBOLS_PER_VENUE);
    assert_eq!(plan.funding["bitget"].len(), rows.len());
    Ok(())
}

#[test]
fn funding_prewarm_remains_bounded_per_venue() -> serde_json::Result<()> {
    let rows = (0..(WS_LIVE_FUNDING_SYMBOLS_PER_VENUE + 5))
        .map(|index| opportunity(StrategyKind::PerpCross, &format!("FUNDING{index}")))
        .collect::<serde_json::Result<Vec<_>>>()?;
    let mut plan = LiveRequestPlan::default();

    append_opportunity_requests(&mut plan, &rows);

    assert_eq!(
        plan.funding["binance"].len(),
        WS_LIVE_FUNDING_SYMBOLS_PER_VENUE
    );
    assert_eq!(
        plan.funding["bitget"].len(),
        WS_LIVE_FUNDING_SYMBOLS_PER_VENUE
    );
    Ok(())
}

#[test]
fn other_strategy_budget_survives_a_saturated_perp_cross_plan() -> serde_json::Result<()> {
    let mut rows = (0..WS_LIVE_SYMBOLS_PER_VENUE)
        .map(|index| opportunity(StrategyKind::PerpCross, &format!("PRIMARY{index}")))
        .collect::<serde_json::Result<Vec<_>>>()?;
    rows.extend(
        (0..(WS_LIVE_OTHER_SYMBOLS_PER_VENUE + 1))
            .map(|index| opportunity(StrategyKind::PerpPriceSpread, &format!("OTHER{index}")))
            .collect::<serde_json::Result<Vec<_>>>()?,
    );
    let mut plan = LiveRequestPlan::default();

    append_opportunity_requests(&mut plan, &rows);

    assert_eq!(
        plan.perp["binance"].len(),
        WS_LIVE_SYMBOLS_PER_VENUE + WS_LIVE_OTHER_SYMBOLS_PER_VENUE
    );
    assert_eq!(
        plan.perp["bitget"].len(),
        WS_LIVE_SYMBOLS_PER_VENUE + WS_LIVE_OTHER_SYMBOLS_PER_VENUE
    );
    for index in 0..WS_LIVE_OTHER_SYMBOLS_PER_VENUE {
        assert!(plan.perp["binance"].contains(&format!("OTHER{index}")));
        assert!(plan.perp["bitget"].contains(&format!("OTHER{index}")));
    }
    assert!(!plan.perp["binance"].contains(&"OTHER4".to_owned()));
    assert!(!plan.perp["bitget"].contains(&"OTHER4".to_owned()));
    Ok(())
}

#[test]
fn live_candidate_pair_does_not_consume_only_one_leg() -> serde_json::Result<()> {
    let mut plan = LiveRequestPlan::default();
    for index in 0..WS_LIVE_SYMBOLS_PER_VENUE {
        push_leg(
            &mut plan,
            LegMarket::Perp,
            "binance",
            &format!("FULL{index}"),
        );
    }

    append_opportunity_request(
        &mut plan,
        &opportunity(StrategyKind::PerpCross, "OVERFLOW")?,
        WS_LIVE_SYMBOLS_PER_VENUE,
    );

    assert!(!plan.perp.contains_key("bitget"));
    assert!(!plan.funding.contains_key("bitget"));
    Ok(())
}

#[test]
fn unavailable_quote_conversion_does_not_consume_candidate_legs() -> serde_json::Result<()> {
    let mut row = opportunity_with_spot_evidence(StrategyKind::CrossSpotPerp, "ATOMIC")?;
    row.quote_conversions = vec![OpportunityQuoteConversion {
        from_quote: "USDC".into(),
        to_quote: "USDT".into(),
        rate: 0.999,
        venue: "okx".into(),
        symbol: "USDCUSDT".into(),
        market_evidence: None,
    }];
    let mut plan = LiveRequestPlan::default();
    let conversion_limit =
        WS_LIVE_OTHER_SYMBOLS_PER_VENUE + WS_LIVE_QUOTE_CONVERSION_SYMBOLS_PER_VENUE;
    for index in 0..conversion_limit {
        assert!(push_market_symbol_with_limit(
            &mut plan.spot,
            "okx",
            &format!("FX{index}"),
            conversion_limit,
        ));
    }

    append_opportunity_request(&mut plan, &row, WS_LIVE_OTHER_SYMBOLS_PER_VENUE);

    assert!(!plan.spot.contains_key("binance"));
    assert!(!plan.perp.contains_key("bitget"));
    assert!(!plan.funding.contains_key("bitget"));
    Ok(())
}
