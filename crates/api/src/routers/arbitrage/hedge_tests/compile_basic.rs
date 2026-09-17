use super::super::*;
use super::*;

#[test]
fn build_leg_applies_execution_params_to_order_intent() {
    let opp = perp_opportunity();
    let params = HedgeExecutionParams {
        capital_usd: 500.0,
        leverage: 3.0,
        order_type: OrderType::Limit,
        market_order_style: None,
        margin_mode: MarginMode::Isolated,
        time_in_force: TimeInForce::Gtx,
        post_only: true,
        limit_offset_bps: 2.0,
    };

    let long = build_leg(LegBuild {
        opp: &opp,
        key: "hedge-test",
        is_long: true,
        quantity: 15.0,
        price: 100.0,
        params: &params,
        mode: ExecutionMode::DryRun,
        strategy: Some(StrategyKind::PerpCross),
    });
    let short = build_leg(LegBuild {
        opp: &opp,
        key: "hedge-test",
        is_long: false,
        quantity: 15.0,
        price: 100.0,
        params: &params,
        mode: ExecutionMode::DryRun,
        strategy: Some(StrategyKind::PerpCross),
    });

    assert!(long.post_only);
    assert_eq!(long.time_in_force, TimeInForce::Gtx);
    assert_eq!(long.margin_mode, MarginMode::Isolated);
    assert_eq!(long.leverage, 3.0);
    assert_ne!(long.client_order_id, long.id);
    assert_ne!(long.client_order_id, short.client_order_id);
    assert!(long.client_order_id.len() <= 32);
    assert!(long
        .client_order_id
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric()));
    assert_eq!(
        long.client_order_id_policy
            .as_ref()
            .map(|policy| policy.public_client_order_id.as_str()),
        Some(long.client_order_id.as_str())
    );
    assert_eq!(
        short
            .client_order_id_policy
            .as_ref()
            .map(|policy| policy.public_client_order_id.as_str()),
        Some(short.client_order_id.as_str())
    );
    assert!((long.price.unwrap_or_default() - 100.02).abs() < 1e-9);
    assert!((short.price.unwrap_or_default() - 99.98).abs() < 1e-9);
}

#[test]
fn build_leg_allows_market_order_for_hedge_preview() {
    let opp = perp_opportunity();
    let params = HedgeExecutionParams {
        capital_usd: 500.0,
        leverage: 3.0,
        order_type: OrderType::Market,
        market_order_style: None,
        margin_mode: MarginMode::Cross,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        limit_offset_bps: 5.0,
    };

    let long = build_leg(LegBuild {
        opp: &opp,
        key: "hedge-market",
        is_long: true,
        quantity: 15.0,
        price: 100.0,
        params: &params,
        mode: ExecutionMode::DryRun,
        strategy: Some(StrategyKind::PerpCross),
    });

    assert_eq!(long.order_type, OrderType::Market);
    assert!((long.price.unwrap_or_default() - 100.05).abs() < 1e-9);
    assert_eq!(long.slippage_tolerance_bps, Some(5.0));
}

#[test]
fn order_compile_plan_marks_hyperliquid_market_as_protected_ioc_with_price() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "hyperliquid:xyz".into();
    let params = market_params();
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert_eq!(plan.venue_order_kind, VenueOrderKind::ProtectedIoc);
    assert_eq!(
        plan.payload_price_policy,
        OrderPayloadPricePolicy::ProtectionPrice
    );
    assert_eq!(plan.effective_time_in_force, TimeInForce::Ioc);
    assert_eq!(plan.payload_price, intent.price);
    assert!(plan.blockers.is_empty());
}

#[test]
fn order_compile_plan_marks_gate_market_as_price_zero_ioc() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "gate".into();
    let params = market_params();
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert_eq!(plan.venue_order_kind, VenueOrderKind::PriceZeroIoc);
    assert_eq!(
        plan.payload_price_policy,
        OrderPayloadPricePolicy::ZeroPrice
    );
    assert_eq!(plan.effective_time_in_force, TimeInForce::Ioc);
    assert_eq!(plan.payload_price, Some(0.0));
    assert!(plan.blockers.is_empty());
}

#[test]
fn order_compile_plan_blocks_gate_limit_gtx_until_post_only_is_used() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "gate".into();
    let params = HedgeExecutionParams {
        capital_usd: 500.0,
        leverage: 3.0,
        order_type: OrderType::Limit,
        market_order_style: None,
        margin_mode: MarginMode::Cross,
        time_in_force: TimeInForce::Gtx,
        post_only: false,
        limit_offset_bps: 5.0,
    };
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert_eq!(plan.venue_order_kind, VenueOrderKind::Limit);
    assert_eq!(
        plan.payload_price_policy,
        OrderPayloadPricePolicy::LimitPrice
    );
    assert_eq!(plan.effective_time_in_force, TimeInForce::Gtx);
    assert!(!plan.blockers.is_empty());
    assert!(
        plan.summary.contains("不支持 GTX"),
        "unexpected summary: {}",
        plan.summary
    );
}

#[test]
fn order_compile_plan_allows_gate_limit_ioc_and_fok_without_blockers() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "gate".into();
    let mut params = HedgeExecutionParams {
        capital_usd: 500.0,
        leverage: 3.0,
        order_type: OrderType::Limit,
        market_order_style: None,
        margin_mode: MarginMode::Cross,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        limit_offset_bps: 5.0,
    };

    let ioc_build = market_leg_build(&opp, &params);
    let ioc_intent = build_leg(ioc_build);
    let ioc_plan = compile_order_plan(&ioc_build, &ioc_intent);

    params.time_in_force = TimeInForce::Fok;
    let fok_build = market_leg_build(&opp, &params);
    let fok_intent = build_leg(fok_build);
    let fok_plan = compile_order_plan(&fok_build, &fok_intent);

    assert_eq!(ioc_plan.effective_time_in_force, TimeInForce::Ioc);
    assert!(ioc_plan.blockers.is_empty());
    assert_eq!(fok_plan.effective_time_in_force, TimeInForce::Fok);
    assert!(fok_plan.blockers.is_empty());
}

#[test]
fn order_compile_plan_exposes_gate_limit_time_in_force_options() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "gate".into();
    let params = HedgeExecutionParams {
        capital_usd: 500.0,
        leverage: 3.0,
        order_type: OrderType::Limit,
        market_order_style: None,
        margin_mode: MarginMode::Cross,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        limit_offset_bps: 5.0,
    };
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert_eq!(
        plan.available_time_in_force,
        vec![TimeInForce::Ioc, TimeInForce::Fok, TimeInForce::Gtc]
    );
}

#[test]
fn order_compile_plan_marks_spot_perp_leg_products() {
    let mut opp = perp_opportunity();
    opp.arb_type = ArbitrageType::SpotFutures;
    opp.strategy_kind = Some(StrategyKind::SpotPerp);
    opp.spot_leg_mode = Some(shared_types::SpotLegMode::BuySpot);
    opp.long_exchange = "okx".into();
    opp.short_exchange = "binance".into();
    opp.long_action = "任意展示文案".into();
    opp.short_action = "任意展示文案".into();
    let params = HedgeExecutionParams::default();
    let long_build = LegBuild {
        opp: &opp,
        key: "spot-perp-product",
        is_long: true,
        quantity: 7.5,
        price: 100.0,
        params: &params,
        mode: ExecutionMode::DryRun,
        strategy: Some(StrategyKind::SpotPerp),
    };
    let long_intent = build_leg(long_build);
    let long_plan = compile_order_plan(&long_build, &long_intent);
    let short_build = LegBuild {
        is_long: false,
        ..long_build
    };
    let short_intent = build_leg(short_build);
    let short_plan = compile_order_plan(&short_build, &short_intent);

    assert_eq!(long_plan.product, FeeProduct::Spot);
    assert_eq!(short_plan.product, FeeProduct::Perp);
}

#[test]
fn order_compile_plan_exposes_client_order_id_policy() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "okx".into();
    let params = HedgeExecutionParams::default();
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert_eq!(
        intent
            .client_order_id_policy
            .as_ref()
            .map(|policy| policy.venue_field.as_str()),
        Some("clOrdId")
    );
    assert_eq!(plan.client_order_id_policy.venue_field, "clOrdId");
    assert_eq!(
        plan.client_order_id_policy.derivation,
        ClientOrderIdDerivation::Identity
    );
    assert_eq!(
        plan.client_order_id_policy.venue_client_order_id.as_deref(),
        Some(intent.client_order_id.as_str())
    );
    assert!(plan.blockers.is_empty());
}
