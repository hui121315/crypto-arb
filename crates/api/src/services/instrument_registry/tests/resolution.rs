use super::*;
use shared_types::execution_sizing::SizingBlock;
use shared_types::FeeProduct;

#[test]
fn missing_entry_blocks_sizing_fail_closed() {
    let registry = InstrumentRegistry::default();
    assert!(registry.supports_venue("BINANCE"));
    assert!(registry.supports_venue("hyperliquid:xyz"));
    assert!(registry.supports_venue("kraken"));
    assert!(registry.supports_venue("gate_crossex:kraken"));
    assert_eq!(
        registry.plan_leg_sizing_for_product("binance", "DOGEUSDT", FeeProduct::Perp, 1000.0, 0.1,),
        Err(SizingBlock::SpecMissing)
    );
}

#[test]
fn crossex_family_refresh_keeps_routes_separate_and_authorizes_exact_lookup() {
    let registry = InstrumentRegistry::default();
    let now_ms = common::time::now_ms();
    let route = |venue: &str, native: &str| {
        let mut instrument = constructible(native, "BTC");
        instrument.venue = venue.to_owned();
        instrument.source_url =
            Some("https://api.gateio.ws/api/v4/crossex/rule/symbols".to_owned());
        instrument.schema_version = Some("crossex-rest-v1.0.2".to_owned());
        instrument.product_type = Some("perp".to_owned());
        instrument.checked_at_ms = now_ms;
        instrument
    };
    assert_eq!(
        registry.replace_venue(
            "gate_crossex",
            vec![
                route("gate_crossex:gate", "GATE_FUTURE_BTC_USDT"),
                route("gate_crossex:kraken", "KRAKEN_FUTURE_BTC_USD"),
            ],
        ),
        2
    );
    assert!(registry.has_venue("gate_crossex"));
    assert!(registry
        .resolve_hedge_instrument_for_product(
            "gate_crossex:gate",
            "GATE_FUTURE_BTC_USDT",
            FeeProduct::Perp,
        )
        .is_some());
    assert!(registry
        .resolve_hedge_instrument_for_product(
            "gate_crossex:kraken",
            "GATE_FUTURE_BTC_USDT",
            FeeProduct::Perp,
        )
        .is_none());
}

#[test]
fn canonical_resolution_prefers_usdt_and_keeps_exact_usdc_access() {
    let registry = InstrumentRegistry::default();
    assert!(registry.upsert(constructible("BTCUSDT", "BTC")).is_ok());
    assert_eq!(
        registry
            .resolve_hedge_instrument_for_product("binance", "BTC", FeeProduct::Perp)
            .map(|row| row.native_symbol),
        Some("BTCUSDT".to_owned())
    );

    let mut usdc = constructible("BTCUSDC", "BTC");
    usdc.quote_asset = Some("USDC".to_owned());
    usdc.settle_asset = Some("USDC".to_owned());
    usdc.margin_asset = Some("USDC".to_owned());
    assert!(registry.upsert(usdc).is_ok());
    assert_eq!(
        registry
            .resolve_hedge_instrument_for_product("binance", "BTC", FeeProduct::Perp)
            .map(|row| row.native_symbol),
        Some("BTCUSDT".to_owned())
    );
    assert_eq!(
        registry
            .resolve_hedge_instrument_for_product("binance", "BTCUSDC", FeeProduct::Perp)
            .map(|row| row.native_symbol),
        Some("BTCUSDC".to_owned())
    );
}

#[test]
fn spot_resolution_binds_exact_pair_and_uses_usdt_only_as_base_default() {
    let registry = InstrumentRegistry::default();
    let mut usdt = constructible("BTCUSDT", "BTC");
    usdt.product_type = Some("spot".to_owned());
    let mut usdc = usdt.clone();
    usdc.native_symbol = "BTCUSDC".to_owned();
    usdc.display_symbol = "BTC/USDC Spot".to_owned();
    usdc.quote_asset = Some("USDC".to_owned());
    usdc.settle_asset = Some("USDC".to_owned());
    usdc.margin_asset = Some("USDC".to_owned());
    assert!(registry.upsert(usdt).is_ok());
    assert!(registry.upsert(usdc).is_ok());

    assert_eq!(
        registry
            .resolve_hedge_instrument_for_product("binance", "BTC/USDC", FeeProduct::Spot)
            .map(|row| row.native_symbol),
        Some("BTCUSDC".to_owned())
    );
    assert_eq!(
        registry
            .resolve_hedge_instrument_for_product("binance", "BTC", FeeProduct::Spot)
            .map(|row| row.native_symbol),
        Some("BTCUSDT".to_owned())
    );
}

#[test]
fn fresh_spot_registry_distinguishes_an_unlisted_exact_pair_from_an_unready_registry() {
    let registry = InstrumentRegistry::default();
    let now_ms = common::time::now_ms();
    assert_eq!(
        registry.exact_spot_listing_evidence("binance", "BTC", "USDC", now_ms),
        None
    );

    let mut usdc = constructible("BTCUSDC", "BTC");
    usdc.product_type = Some("spot".to_owned());
    usdc.quote_asset = Some("USDC".to_owned());
    usdc.settle_asset = Some("USDC".to_owned());
    usdc.margin_asset = Some("USDC".to_owned());
    usdc.checked_at_ms = now_ms;
    assert!(registry.upsert(usdc).is_ok());

    assert_eq!(
        registry.exact_spot_listing_evidence("binance", "BTC", "USDC", now_ms),
        Some(true)
    );
    assert_eq!(
        registry.exact_spot_listing_evidence("binance", "BTC", "USDT", now_ms),
        Some(false)
    );
}

#[test]
fn spot_execution_evidence_distinguishes_syncing_unlisted_and_incomplete() {
    let registry = InstrumentRegistry::default();
    let now_ms = common::time::now_ms();
    assert_eq!(
        registry
            .resolve_spot_instrument_evidence("binance", "SOL", "USDC", now_ms)
            .status,
        SpotInstrumentResolutionStatus::Syncing
    );

    registry.record_spot_refresh_success("binance", now_ms);
    assert_eq!(
        registry
            .resolve_spot_instrument_evidence("binance", "SOL", "USDC", now_ms)
            .status,
        SpotInstrumentResolutionStatus::Unlisted
    );

    let mut incomplete = official_spot("SOLUSDC", "SOL", "USDC");
    incomplete.checked_at_ms = now_ms;
    incomplete.qty_step = None;
    assert!(registry.upsert(incomplete).is_ok());
    let evidence = registry.resolve_spot_instrument_evidence("binance", "SOL", "USDC", now_ms);
    assert_eq!(evidence.status, SpotInstrumentResolutionStatus::Incomplete);
    assert_eq!(
        evidence
            .instrument
            .as_ref()
            .map(|instrument| instrument.native_symbol.as_str()),
        Some("SOLUSDC")
    );
}

#[test]
fn failed_spot_refresh_never_reuses_cached_specs_as_ready() {
    let registry = InstrumentRegistry::default();
    let now_ms = common::time::now_ms();
    let mut spot = official_spot("SOLUSDC", "SOL", "USDC");
    spot.checked_at_ms = now_ms;
    assert!(registry.upsert(spot).is_ok());
    registry.record_spot_refresh_failure("binance", "exchangeInfo timed out");

    let evidence = registry.resolve_spot_instrument_evidence("binance", "SOL", "USDC", now_ms);

    assert_eq!(evidence.status, SpotInstrumentResolutionStatus::Unavailable);
    assert!(evidence.instrument.is_some());
    assert!(evidence
        .problem
        .as_ref()
        .is_some_and(|problem| problem.message.contains("timed out")));
}

#[test]
fn constructible_entry_plans_rounded_sizing() {
    let registry = InstrumentRegistry::default();
    assert!(registry.upsert(constructible("BTCUSDT", "BTC")).is_ok());
    let plan = registry
        .plan_leg_sizing_for_product("binance", "BTCUSDT", FeeProduct::Perp, 1234.0, 30_000.0)
        .unwrap_or_default();
    assert!((plan.rounded_base_qty - 0.041).abs() < 1e-9);
    assert!((plan.actual_notional_usd - 1230.0).abs() < 1e-6);
}

#[test]
fn below_min_notional_is_blocked() {
    let registry = InstrumentRegistry::default();
    let mut instrument = constructible("BTCUSDT", "BTC");
    instrument.min_notional = Some(100.0);
    assert!(registry.upsert(instrument).is_ok());
    assert_eq!(
        registry.plan_leg_sizing_for_product(
            "binance",
            "BTCUSDT",
            FeeProduct::Perp,
            50.0,
            30_000.0,
        ),
        Err(SizingBlock::BelowMinNotional)
    );
}

#[test]
fn replace_venue_clears_stale_entries() {
    let registry = InstrumentRegistry::default();
    assert!(registry.upsert(constructible("BTCUSDT", "BTC")).is_ok());
    assert!(registry.upsert(constructible("ETHUSDT", "ETH")).is_ok());
    assert_eq!(registry.len(), 2);
    let accepted = registry.replace_venue("binance", vec![constructible("BTCUSDT", "BTC")]);
    assert_eq!(accepted, 1);
    assert_eq!(registry.len(), 1);
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "ETHUSDT", FeeProduct::Perp)
        .is_none());
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Perp)
        .is_some());
}
