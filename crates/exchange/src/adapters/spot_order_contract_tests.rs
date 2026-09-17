use super::*;
use shared_types::{
    ExecutionMode, InstrumentAssetClass, InstrumentListingStatus, InstrumentMetadataSource,
    MarginMode, OrderSide, OrderSource, TimeInForce,
};

#[test]
fn compiles_ticket_bound_spot_quantity_and_native_symbol() -> Result<(), String> {
    let context = context();
    let mut intent = intent();
    intent.quantity = context
        .sizing_plan
        .ok_or("missing sizing")?
        .rounded_base_qty;
    let compiled = compile("binance", &intent, &context).map_err(|error| error.to_string())?;
    assert_eq!(compiled.native_symbol, "BTCUSDT");
    assert_eq!(compiled.quantity, "0.01");
    assert_eq!(compiled.price.as_deref(), Some("50000"));
    Ok(())
}

#[test]
fn rejects_perp_or_tampered_spot_context() {
    let mut perp_context = context();
    perp_context.product = FeeProduct::Perp;
    assert!(compile("binance", &intent(), &perp_context).is_err());

    let mut spot_context = context();
    let mut intent = intent();
    intent.quantity = 1.0;
    assert!(compile("binance", &intent, &spot_context).is_err());
    spot_context.product = FeeProduct::Unknown;
    assert!(query_symbol("binance", "BTCUSDT", &spot_context).is_err());
}

fn context() -> OrderSubmissionContext {
    let instrument = shared_types::InstrumentSpec {
        venue: "binance".to_owned(),
        native_symbol: "BTCUSDT".to_owned(),
        canonical_symbol: "BTC".to_owned(),
        display_symbol: "BTC-USDT Spot".to_owned(),
        asset_class: InstrumentAssetClass::Crypto,
        product_type: Some("spot".to_owned()),
        quote_asset: Some("USDT".to_owned()),
        settle_asset: None,
        margin_asset: None,
        contract_size: Some(1.0),
        execution_supported: true,
        price_tick: Some(0.01),
        qty_step: Some(0.0001),
        min_qty: Some(0.0001),
        min_notional: Some(5.0),
        listing_status: InstrumentListingStatus::Trading,
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some("/api/v3/exchangeInfo".to_owned()),
        checked_at_ms: 1_700_000_000_000,
        schema_version: Some("binance-spot-exchange-info-2026-08-04".to_owned()),
    };
    let sizing = shared_types::plan_leg_sizing(500.0, &instrument, 50_000.0)
        .unwrap_or_else(|error| panic!("spot sizing fixture: {error:?}"));
    OrderSubmissionContext {
        product: FeeProduct::Spot,
        instrument_spec: Some(instrument),
        sizing_plan: Some(sizing),
        ..OrderSubmissionContext::default()
    }
}

fn intent() -> OrderIntent {
    OrderIntent {
        id: "spot-order-1".to_owned(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "binance".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "spot-client-1".to_owned(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
