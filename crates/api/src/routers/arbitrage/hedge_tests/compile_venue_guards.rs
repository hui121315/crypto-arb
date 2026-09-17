use super::super::*;
use super::*;

#[test]
fn order_compile_plan_allows_kucoin_live_opening_writer_position_side_precheck() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "kucoin".into();
    let params = HedgeExecutionParams {
        capital_usd: 500.0,
        leverage: 3.0,
        order_type: OrderType::Limit,
        market_order_style: None,
        margin_mode: MarginMode::Isolated,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        limit_offset_bps: 5.0,
    };
    let mut build = market_leg_build(&opp, &params);
    build.mode = ExecutionMode::Live;
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);

    assert!(plan.blockers.is_empty());
}

#[test]
fn order_compile_plan_blocks_kucoin_live_reduce_only_without_position_side_evidence() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "kucoin".into();
    let params = HedgeExecutionParams {
        capital_usd: 500.0,
        leverage: 3.0,
        order_type: OrderType::Limit,
        market_order_style: None,
        margin_mode: MarginMode::Isolated,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        limit_offset_bps: 5.0,
    };
    let mut build = market_leg_build(&opp, &params);
    build.mode = ExecutionMode::Live;
    let mut intent = build_leg(build);
    intent.reduce_only = true;

    let plan = compile_order_plan(&build, &intent);

    assert!(plan
        .blockers
        .iter()
        .any(|blocker| blocker.contains("KuCoin hedge-mode reduce-only")));
}
