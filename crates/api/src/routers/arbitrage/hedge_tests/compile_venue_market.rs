use super::super::*;
use super::*;

#[test]
fn order_compile_plan_omits_payload_price_for_native_no_price_market() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "binance".into();
    let params = market_params();
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert_eq!(plan.venue_order_kind, VenueOrderKind::NativeMarket);
    assert_eq!(plan.payload_price_policy, OrderPayloadPricePolicy::Omit);
    assert!(plan.payload_price.is_none());
    assert!(plan.blockers.is_empty());
}

#[test]
fn order_compile_plan_allows_bybit_market_with_slippage_tolerance_evidence() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "bybit".into();
    let params = market_params();
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert_eq!(plan.venue_order_kind, VenueOrderKind::NativeMarket);
    assert_eq!(plan.payload_price_policy, OrderPayloadPricePolicy::Omit);
    assert_eq!(plan.effective_time_in_force, TimeInForce::Ioc);
    assert_eq!(plan.slippage_tolerance_bps, Some(5.0));
    assert!(
        plan.blockers.is_empty(),
        "unexpected blockers: {:?}",
        plan.blockers
    );
    assert!(plan.summary.contains("slippageTolerance=0.05"));
}

#[test]
fn order_compile_plan_blocks_bybit_market_without_slippage_tolerance_evidence() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "bybit".into();
    let mut params = market_params();
    params.limit_offset_bps = 0.0;
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert_eq!(plan.venue_order_kind, VenueOrderKind::NativeMarket);
    assert_eq!(plan.payload_price_policy, OrderPayloadPricePolicy::Omit);
    assert_eq!(plan.effective_time_in_force, TimeInForce::Ioc);
    assert_eq!(plan.slippage_tolerance_bps, None);
    assert!(plan
        .blockers
        .iter()
        .any(|blocker| blocker.contains("slippageTolerance")));
}

#[test]
fn account_mode_guard_scope_includes_all_supported_live_venues_only() {
    let binance = order_plan_for_exchange("binance");
    let bitget = order_plan_for_exchange("bitget");
    let kucoin = order_plan_for_exchange("kucoin");
    let okx = order_plan_for_exchange("okx");
    let bybit = order_plan_for_exchange("bybit");
    let gate = order_plan_for_exchange("gate");

    assert!(account_mode_plan(ExecutionMode::Live, &binance));
    assert!(account_mode_plan(ExecutionMode::Live, &bitget));
    assert!(account_mode_plan(ExecutionMode::Live, &kucoin));
    assert!(account_mode_plan(ExecutionMode::Live, &okx));
    assert!(account_mode_plan(ExecutionMode::Live, &bybit));
    assert!(account_mode_plan(ExecutionMode::Live, &gate));
    assert!(!account_mode_plan(ExecutionMode::DryRun, &binance));
    assert!(!account_mode_plan(ExecutionMode::DryRun, &bitget));
    assert!(!account_mode_plan(ExecutionMode::DryRun, &okx));
    assert!(!account_mode_plan(ExecutionMode::DryRun, &bybit));
    assert!(!account_mode_plan(ExecutionMode::DryRun, &gate));
}
