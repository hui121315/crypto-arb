use super::*;
use shared_types::{
    ExecutionMode, FeeProduct, HedgeLegRole, InstrumentAssetClass, InstrumentListingStatus,
    InstrumentMetadataSource, MarginMode, OrderPayloadPricePolicy, OrderSide, OrderSource,
    OrderType, TimeInForce, VenueOrderKind,
};

#[test]
fn live_preview_instrument_contract_is_fail_closed_then_attached() {
    let registry = InstrumentRegistry::default();
    assert_eq!(registry.replace_venue("binance", vec![instrument()]), 1);
    let mut missing_long = compile_plan(HedgeLegRole::Long);
    let mut missing_short = compile_plan(HedgeLegRole::Short);
    missing_short.symbol = "ETH/USDT".into();
    let mut missing_long_intent = intent(OrderSide::Buy);
    let mut missing_short_intent = intent(OrderSide::Sell);
    attach_paired_instrument_contracts(
        &registry,
        ExecutionMode::Live,
        [
            (
                &mut missing_long,
                &mut missing_long_intent,
                1_234.0,
                30_000.0,
            ),
            (
                &mut missing_short,
                &mut missing_short_intent,
                1_234.0,
                30_100.0,
            ),
        ],
    );
    assert!(
        !missing_short.blockers.is_empty(),
        "missing pair must fail closed: {:?}",
        missing_short.blockers
    );

    let mut attached_long = compile_plan(HedgeLegRole::Long);
    let mut attached_short = compile_plan(HedgeLegRole::Short);
    let mut long_intent = intent(OrderSide::Buy);
    let mut short_intent = intent(OrderSide::Sell);
    attach_paired_instrument_contracts(
        &registry,
        ExecutionMode::Live,
        [
            (&mut attached_long, &mut long_intent, 1_234.0, 30_000.0),
            (&mut attached_short, &mut short_intent, 1_234.0, 30_100.0),
        ],
    );
    assert_eq!(
        attached_long
            .instrument_spec
            .as_ref()
            .map(|spec| spec.native_symbol.as_str()),
        Some("BTCUSDT")
    );
    assert!(attached_long
        .sizing_plan
        .is_some_and(|sizing| sizing.rounded_base_qty == 0.04));
    assert_eq!(long_intent.quantity, short_intent.quantity);
    assert_eq!(long_intent.quantity, 0.04);
    assert!(attached_long.blockers.is_empty());
    assert!(attached_short.blockers.is_empty());
}

#[test]
fn paper_preview_never_leaves_a_partial_sizing_contract() {
    let registry = InstrumentRegistry::default();
    assert_eq!(registry.replace_venue("binance", vec![instrument()]), 1);
    let mut long_plan = compile_plan(HedgeLegRole::Long);
    let mut short_plan = compile_plan(HedgeLegRole::Short);
    let mut long_intent = intent(OrderSide::Buy);
    let mut short_intent = intent(OrderSide::Sell);

    attach_paired_instrument_contracts(
        &registry,
        ExecutionMode::DryRun,
        [
            (&mut long_plan, &mut long_intent, 2.0, 30_000.0),
            (&mut short_plan, &mut short_intent, 2.0, 30_000.0),
        ],
    );

    assert!(long_plan.instrument_spec.is_none());
    assert!(long_plan.sizing_plan.is_none());
    assert!(long_plan.blockers.is_empty());
}

fn compile_plan(role: HedgeLegRole) -> shared_types::OrderCompilePlan {
    shared_types::OrderCompilePlan {
        role,
        exchange: "binance".into(),
        symbol: "BTC/USDT".into(),
        client_order_id_policy: Default::default(),
        product: FeeProduct::Perp,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: OrderType::Limit,
        effective_order_type: OrderType::Limit,
        requested_time_in_force: TimeInForce::Ioc,
        effective_time_in_force: TimeInForce::Ioc,
        available_order_types: vec![OrderType::Limit],
        available_time_in_force: vec![TimeInForce::Ioc],
        available_margin_modes: vec![MarginMode::Cross],
        venue_capability: Default::default(),
        market_order_style: None,
        venue_order_kind: VenueOrderKind::Limit,
        payload_price_policy: OrderPayloadPricePolicy::LimitPrice,
        reference_price: Some(30_000.0),
        protection_price: None,
        payload_price: Some(30_000.0),
        slippage_tolerance_bps: None,
        summary: "test plan".into(),
        blockers: Vec::new(),
    }
}

fn intent(side: OrderSide) -> shared_types::OrderIntent {
    shared_types::OrderIntent {
        id: "intent-test".into(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "binance".into(),
        symbol: "BTC/USDT".into(),
        side,
        order_type: OrderType::Limit,
        quantity: 0.04,
        price: Some(30_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "intent-test".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn instrument() -> shared_types::InstrumentSpec {
    shared_types::InstrumentSpec {
        venue: "binance".into(),
        native_symbol: "BTCUSDT".into(),
        canonical_symbol: "BTC/USDT".into(),
        display_symbol: "BTC-USDT Perp".into(),
        asset_class: InstrumentAssetClass::Crypto,
        product_type: Some("perp".into()),
        quote_asset: Some("USDT".into()),
        settle_asset: Some("USDT".into()),
        margin_asset: Some("USDT".into()),
        contract_size: Some(1.0),
        execution_supported: true,
        price_tick: Some(0.1),
        qty_step: Some(0.001),
        min_qty: Some(0.001),
        min_notional: Some(5.0),
        listing_status: InstrumentListingStatus::Trading,
        funding_interval_ms: Some(28_800_000),
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some("/fapi/v1/exchangeInfo".into()),
        checked_at_ms: common::time::now_ms(),
        schema_version: Some(
            "binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11".into(),
        ),
    }
}
