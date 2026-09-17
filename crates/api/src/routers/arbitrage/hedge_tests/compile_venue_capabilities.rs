use super::super::*;
use super::*;

#[test]
fn order_compile_plan_derives_hyperliquid_builder_cloid() {
    let mut opp = perp_opportunity();
    opp.long_exchange = "hyperliquid:xyz".into();
    let params = HedgeExecutionParams::default();
    let build = market_leg_build(&opp, &params);
    let intent = build_leg(build);

    let plan = compile_order_plan(&build, &intent);
    let cloid = plan
        .client_order_id_policy
        .venue_client_order_id
        .as_deref()
        .unwrap_or_default();

    assert_eq!(plan.client_order_id_policy.venue_field, "c/cloid");
    assert_eq!(
        plan.client_order_id_policy.derivation,
        ClientOrderIdDerivation::StableHash
    );
    assert!(cloid.starts_with("0x"));
    assert_eq!(cloid.len(), 34);
    assert!(plan.blockers.is_empty());
}

#[test]
fn available_order_types_reflect_runtime_capabilities() {
    let mut capabilities = ExchangeCapabilities {
        supports_testnet: true,
        supports_live: true,
        supports_spot: false,
        supports_perp: true,
        supports_limit_orders: true,
        supports_market_orders: false,
        supports_post_only: true,
        supports_reduce_only: true,
    };

    assert_eq!(
        available_order_types(&capabilities),
        vec![OrderType::Limit, OrderType::PostOnly]
    );

    capabilities.supports_market_orders = true;
    assert_eq!(
        available_order_types(&capabilities),
        vec![OrderType::Limit, OrderType::Market, OrderType::PostOnly]
    );
}

#[test]
fn account_mode_preflight_includes_bitget_and_gate_live_venues() {
    let bitget = order_plan_for_exchange("bitget");
    let gate = order_plan_for_exchange("gate");

    assert!(crate::services::hedge_preview::account_mode_plan(
        ExecutionMode::Live,
        &bitget
    ));
    assert!(crate::services::hedge_preview::account_mode_plan(
        ExecutionMode::Live,
        &gate
    ));
    assert!(!crate::services::hedge_preview::account_mode_plan(
        ExecutionMode::DryRun,
        &gate
    ));
}
