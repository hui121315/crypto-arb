use shared_types::{
    ClientOrderIdDerivation, ClientOrderIdPolicy, FeeProduct, HedgeLegRole, InstrumentAssetClass,
    InstrumentListingStatus, InstrumentMetadataSource, MarginMode, OrderCompilePlan,
    OrderPayloadPricePolicy, OrderType, TimeInForce, VenueOrderKind,
};

pub(super) fn attach_sizing_contract(plan: &mut OrderCompilePlan) -> Result<(), String> {
    let instrument = hyperliquid_instrument();
    let sizing = shared_types::plan_leg_sizing(1_234.0, &instrument, 100.0)
        .map_err(|error| error.code().to_owned())?;
    plan.instrument_spec = Some(instrument);
    plan.sizing_plan = Some(sizing);
    Ok(())
}

fn hyperliquid_instrument() -> shared_types::InstrumentSpec {
    shared_types::InstrumentSpec {
        venue: "hyperliquid:xyz".into(),
        native_symbol: "@123".into(),
        canonical_symbol: "XYZ".into(),
        display_symbol: "XYZ Perp".into(),
        asset_class: InstrumentAssetClass::Crypto,
        product_type: Some("perp".into()),
        quote_asset: Some("USDC".into()),
        settle_asset: Some("USDC".into()),
        margin_asset: Some("USDC".into()),
        contract_size: Some(1.0),
        execution_supported: true,
        price_tick: Some(0.001),
        qty_step: Some(0.01),
        min_qty: Some(0.01),
        min_notional: None,
        listing_status: InstrumentListingStatus::Trading,
        funding_interval_ms: Some(28_800_000),
        builder_dex: Some("xyz".into()),
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: Some("https://api.hyperliquid.xyz/info".into()),
        checked_at_ms: 1_700_000_000_000,
        schema_version: Some("hyperliquid-meta-v1".into()),
    }
}

pub(super) fn hyperliquid_plan(role: HedgeLegRole) -> OrderCompilePlan {
    let venue_client_order_id = match role {
        HedgeLegRole::Long => "0x00000000000000000000000000000001",
        HedgeLegRole::Short => "0x00000000000000000000000000000002",
    };
    OrderCompilePlan {
        role,
        exchange: "hyperliquid:xyz".into(),
        symbol: "XYZ".into(),
        client_order_id_policy: ClientOrderIdPolicy {
            venue: "hyperliquid:xyz".into(),
            venue_family: "hyperliquid".into(),
            venue_field: "c/cloid".into(),
            public_client_order_id: format!("public-{venue_client_order_id}"),
            venue_client_order_id: Some(venue_client_order_id.into()),
            derivation: ClientOrderIdDerivation::StableHash,
            policy_version: "hyperliquid-cloid-v1".into(),
            official_format: "0x + 32 lowercase hex".into(),
            max_length: Some(34),
            supports_query_by_client_id: true,
            supports_cancel_by_client_id: true,
            constraints: Vec::new(),
            blockers: Vec::new(),
            official_doc_urls: Vec::new(),
        },
        product: FeeProduct::Perp,
        instrument_spec: None,
        sizing_plan: None,
        requested_order_type: OrderType::Market,
        effective_order_type: OrderType::Limit,
        requested_time_in_force: TimeInForce::Ioc,
        effective_time_in_force: TimeInForce::Ioc,
        available_order_types: vec![OrderType::Limit, OrderType::Market],
        available_time_in_force: vec![TimeInForce::Ioc],
        available_margin_modes: vec![MarginMode::Cross],
        venue_capability: shared_types::VenueSymbolCapability::default(),
        market_order_style: None,
        venue_order_kind: VenueOrderKind::ProtectedIoc,
        payload_price_policy: OrderPayloadPricePolicy::ProtectionPrice,
        reference_price: Some(100.0),
        protection_price: Some(100.05),
        payload_price: Some(100.05),
        slippage_tolerance_bps: Some(5.0),
        summary: "Hyperliquid protected IOC".into(),
        blockers: Vec::new(),
    }
}
