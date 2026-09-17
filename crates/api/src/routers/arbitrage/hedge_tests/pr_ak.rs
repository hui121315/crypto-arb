use super::*;

#[test]
fn pr_ak_build_leg_and_compile_plan_keep_distinct_market_evidence_symbols() {
    let mut opportunity = perp_opportunity();
    opportunity.long_leg_market_evidence = Some(market_evidence("bitget", "BTCUSDT"));
    opportunity.short_leg_market_evidence = Some(market_evidence("gate", "BTC_USDT"));
    let params = HedgeExecutionParams::default();

    let long_build = LegBuild {
        opp: &opportunity,
        key: "hedge-leg-symbols",
        is_long: true,
        quantity: 5.0,
        price: 100.0,
        params: &params,
        mode: ExecutionMode::DryRun,
        strategy: Some(StrategyKind::PerpCross),
    };
    let short_build = LegBuild {
        is_long: false,
        ..long_build
    };
    let long_intent = build_leg(long_build);
    let short_intent = build_leg(short_build);
    let long_plan = compile_order_plan(&long_build, &long_intent);
    let short_plan = compile_order_plan(&short_build, &short_intent);

    assert_eq!(long_intent.symbol, "BTCUSDT");
    assert_eq!(long_plan.symbol, "BTCUSDT");
    assert_eq!(short_intent.symbol, "BTC_USDT");
    assert_eq!(short_plan.symbol, "BTC_USDT");
}

#[test]
fn pr_ak_preview_rejects_cross_wired_leg_market_evidence() -> anyhow::Result<()> {
    let mut opportunity = raw_eligible_without_verified_fee_opportunity();
    let Some(long) = opportunity.long_leg_market_evidence.as_mut() else {
        anyhow::bail!("long market evidence fixture is missing")
    };
    long.venue = opportunity.short_exchange.clone();

    let error = validate_executable_opportunity(&opportunity)
        .err()
        .ok_or_else(|| anyhow::anyhow!("cross-wired leg evidence must fail closed"))?;

    assert_eq!(error.code(), codes::OPPORTUNITY_NOT_EXECUTABLE);
    assert!(error.to_string().contains("long leg market evidence venue"));
    Ok(())
}

#[test]
fn pr_ak_preview_rejects_leg_market_evidence_without_symbol_or_price() -> anyhow::Result<()> {
    let mut opportunity = raw_eligible_without_verified_fee_opportunity();
    let Some(short) = opportunity.short_leg_market_evidence.as_mut() else {
        anyhow::bail!("short market evidence fixture is missing")
    };
    short.symbol = "  ".to_owned();
    short.price = None;

    let error = validate_executable_opportunity(&opportunity)
        .err()
        .ok_or_else(|| anyhow::anyhow!("incomplete leg evidence must fail closed"))?;
    let detail = error.to_string();

    assert!(detail.contains("short leg market evidence symbol is missing"));
    assert!(detail.contains("short leg market evidence price is missing"));
    Ok(())
}

#[test]
fn pr_ak_preview_prices_prefer_ticket_evidence_over_client_values() -> anyhow::Result<()> {
    let opportunity = perp_opportunity();
    let mut ticket = ticket_with(Vec::new(), true);
    ticket.long_leg.reference_price = Some(100.25);
    ticket.short_leg.reference_price = Some(101.75);
    ticket.long_leg.open_vwap_price = Some(100.5);
    ticket.short_leg.open_vwap_price = Some(101.5);
    let request = HedgePreviewRequest {
        opportunity_id: opportunity.id.clone(),
        opportunity_snapshot_id: None,
        capital_usd: 500.0,
        leverage: 1.0,
        long_price: Some(9_999.0),
        short_price: Some(8_888.0),
        long_notional_usd: None,
        short_notional_usd: None,
        execution_params: None,
    };

    assert_eq!(preview_long_price(&request, &ticket, &opportunity)?, 100.5);
    assert_eq!(preview_short_price(&request, &ticket, &opportunity)?, 101.5);
    Ok(())
}
