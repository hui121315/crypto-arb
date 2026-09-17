use super::*;
use shared_types::execution_sizing::SizingBlock;
use shared_types::instrument_registry::InstrumentAssetClass;
use shared_types::instruments::{InstrumentListingStatus, InstrumentMetadataSource};
use shared_types::FeeProduct;

mod resolution;

fn constructible(native: &str, canonical: &str) -> VenueInstrument {
    let evidence = instrument_metadata_evidence("binance");
    VenueInstrument {
        venue: "binance".to_owned(),
        native_symbol: native.to_owned(),
        canonical_symbol: canonical.to_owned(),
        display_symbol: native.to_owned(),
        asset_class: InstrumentAssetClass::Crypto,
        product_type: Some("perpetual".to_owned()),
        quote_asset: Some("USDT".to_owned()),
        settle_asset: Some("USDT".to_owned()),
        margin_asset: Some("USDT".to_owned()),
        contract_size: Some(1.0),
        execution_supported: true,
        price_tick: Some(0.1),
        qty_step: Some(0.001),
        min_qty: None,
        min_notional: Some(5.0),
        listing_status: InstrumentListingStatus::Trading,
        funding_interval_ms: None,
        builder_dex: None,
        source: InstrumentMetadataSource::OfficialEndpoint,
        source_url: evidence.map(|row| row.path.clone()),
        checked_at_ms: common::time::now_ms(),
        schema_version: evidence.map(|row| row.doc_version.clone()),
    }
}

fn official_spot(native: &str, canonical: &str, quote: &str) -> VenueInstrument {
    let mut instrument = constructible(native, canonical);
    instrument.product_type = Some("spot".to_owned());
    instrument.quote_asset = Some(quote.to_owned());
    instrument.settle_asset = None;
    instrument.margin_asset = None;
    instrument.source_url = Some("/api/v3/exchangeInfo".to_owned());
    instrument.schema_version = Some("binance-spot-exchange-info-2026-08-04".to_owned());
    instrument
}

#[test]
fn upsert_then_lookup_is_case_insensitive() {
    let registry = InstrumentRegistry::default();
    assert!(registry.upsert(constructible("BTCUSDT", "BTC")).is_ok());
    assert_eq!(registry.len(), 1);
    assert!(registry
        .resolve_hedge_instrument_for_product("BINANCE", "btcusdt", FeeProduct::Perp)
        .is_some());
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Perp)
        .is_some());
    assert!(registry.has_venue("BINANCE"));
    assert!(!registry.has_venue("okx"));
}

#[test]
fn same_native_symbol_keeps_spot_and_perp_specs() {
    let registry = InstrumentRegistry::default();
    let perpetual = constructible("BTCUSDT", "BTC");
    let mut spot = perpetual.clone();
    spot.product_type = Some("spot".to_owned());
    spot.display_symbol = "BTC/USDT Spot".to_owned();

    assert!(registry.upsert(perpetual).is_ok());
    assert!(registry.upsert(spot).is_ok());

    assert_eq!(registry.len(), 2);
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Perp)
        .is_some());
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Spot)
        .is_some());
}

#[test]
fn spot_refresh_preserves_perpetual_rows_and_replaces_only_spot() {
    let registry = InstrumentRegistry::default();
    assert!(registry.upsert(constructible("BTCUSDT", "BTC")).is_ok());
    assert!(registry
        .upsert(official_spot("BTCUSDT", "BTC", "USDT"))
        .is_ok());

    assert_eq!(
        registry.replace_spot_venue("binance", vec![official_spot("ETHUSDC", "ETH", "USDC")],),
        1
    );

    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Perp)
        .is_some());
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Spot)
        .is_none());
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "ETHUSDC", FeeProduct::Spot)
        .is_some());
}

#[test]
fn spot_probe_recovers_without_reopening_failed_perpetual_specs() {
    let registry = InstrumentRegistry::default();
    assert!(registry.upsert(constructible("BTCUSDT", "BTC")).is_ok());
    registry.record_refresh_failure("binance", "perpetual metadata timed out");

    assert_eq!(
        registry.replace_spot_venue("binance", vec![official_spot("SOLUSDC", "SOL", "USDC")],),
        1
    );

    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "SOLUSDC", FeeProduct::Spot)
        .is_some());
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Perp)
        .is_none());
}

#[test]
fn full_spot_ws_plan_uses_every_trading_native_pair() {
    let registry = InstrumentRegistry::default();
    let mut btc_usd = constructible("BTC/USD", "BTC");
    btc_usd.venue = "kraken".to_owned();
    btc_usd.product_type = Some("spot".to_owned());
    btc_usd.quote_asset = Some("USD".to_owned());
    btc_usd.display_symbol = "BTC/USD".to_owned();
    let mut eth_eur = btc_usd.clone();
    eth_eur.native_symbol = "ETH/EUR".to_owned();
    eth_eur.canonical_symbol = "ETH".to_owned();
    eth_eur.quote_asset = Some("EUR".to_owned());
    eth_eur.display_symbol = "ETH/EUR".to_owned();
    let mut suspended = btc_usd.clone();
    suspended.native_symbol = "SOL/USD".to_owned();
    suspended.canonical_symbol = "SOL".to_owned();
    suspended.display_symbol = "SOL/USD".to_owned();
    suspended.listing_status = InstrumentListingStatus::Suspended;

    assert!(registry.upsert(btc_usd).is_ok());
    assert!(registry.upsert(eth_eur).is_ok());
    assert!(registry.upsert(suspended).is_ok());
    assert!(registry.upsert(constructible("BTCUSDT", "BTC")).is_ok());

    let requests = registry.spot_ws_requests_by_venue();
    assert_eq!(
        requests.get("kraken"),
        Some(&vec!["BTC/USD".to_owned(), "ETH/EUR".to_owned()])
    );
    assert!(!requests.contains_key("binance"));
}

#[test]
fn full_spot_ws_plan_excludes_synthetic_routed_venues() {
    let registry = InstrumentRegistry::default();
    let mut routed = constructible("GATE_SPOT_BTC_USDT", "BTC");
    routed.venue = "gate_crossex:gate".to_owned();
    routed.product_type = Some("spot".to_owned());
    routed.native_symbol = "GATE_SPOT_BTC_USDT".to_owned();
    routed.display_symbol = "BTC/USDT".to_owned();

    assert!(registry.upsert(routed).is_ok());
    assert!(!registry
        .spot_ws_requests_by_venue()
        .contains_key("gate_crossex:gate"));
}

#[test]
fn instrument_refresh_gates_dedupe_each_scope_without_blocking_spot() {
    let registry = InstrumentRegistry::default();

    assert!(registry.begin_instrument_refresh("binance"));
    assert!(!registry.begin_instrument_refresh("BINANCE"));
    assert!(registry.begin_spot_instrument_refresh("BINANCE"));
    assert!(!registry.begin_spot_instrument_refresh("binance"));
    assert!(registry.begin_instrument_refresh("kraken"));

    registry.finish_instrument_refresh("binance");
    assert!(registry.begin_instrument_refresh("BINANCE"));
    assert!(!registry.begin_spot_instrument_refresh("BINANCE"));
    registry.finish_spot_instrument_refresh("binance");
    assert!(registry.begin_spot_instrument_refresh("BINANCE"));
}

#[test]
fn perpetual_refresh_failure_does_not_revoke_fresh_spot_specs() {
    let registry = InstrumentRegistry::default();
    assert_eq!(
        registry.replace_spot_venue("binance", vec![official_spot("SOLUSDC", "SOL", "USDC")],),
        1
    );

    registry.record_refresh_failure("binance", "perpetual metadata timed out");

    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "SOLUSDC", FeeProduct::Spot)
        .is_some());
}

#[test]
fn spot_instrument_refresh_retry_waits_for_the_bounded_backoff() {
    let registry = InstrumentRegistry::default();
    assert!(registry.spot_instrument_refresh_retry_due("binance", 1_000, 30_000));

    registry.record_spot_refresh_success("binance", 1_000);
    assert!(!registry.spot_instrument_refresh_retry_due("binance", 30_999, 30_000));
    assert!(registry.spot_instrument_refresh_retry_due("binance", 31_000, 30_000));

    registry.record_spot_unsupported("kraken", "adapter is not registered");
    assert!(!registry.spot_instrument_refresh_retry_due("kraken", i64::MAX, 30_000));
}

#[test]
fn structurally_invalid_entry_is_rejected() {
    let registry = InstrumentRegistry::default();
    let mut bad = constructible("BTCUSDT", "BTC");
    bad.canonical_symbol = "   ".to_owned();
    assert!(registry.upsert(bad).is_err());
    assert_eq!(registry.len(), 0);
}

#[test]
fn observation_only_entry_is_not_hedge_constructible() {
    let registry = InstrumentRegistry::default();
    let mut cached = constructible("ETHUSDT", "ETH");
    cached.source = InstrumentMetadataSource::CachedSnapshot;
    assert!(registry.upsert(cached).is_ok());
    assert!(registry
        .resolve_hedge_instrument_for_product("binance", "ETHUSDT", FeeProduct::Perp)
        .is_none());
    assert_eq!(
        registry.plan_leg_sizing_for_product(
            "binance",
            "ETHUSDT",
            FeeProduct::Perp,
            1000.0,
            3000.0,
        ),
        Err(SizingBlock::SpecMissing)
    );
}

#[test]
fn failed_or_stale_probe_blocks_retained_spec_and_sizing() {
    let registry = InstrumentRegistry::default();
    let instrument = constructible("BTCUSDT", "BTC");
    let checked_at_ms = instrument.checked_at_ms;
    assert!(registry.upsert(instrument).is_ok());
    assert!(registry
        .hedge_instrument_at_for_product("binance", "BTCUSDT", FeeProduct::Perp, checked_at_ms + 1,)
        .is_some());

    registry.record_refresh_failure("binance", "rate limited");
    assert!(registry
        .hedge_instrument_at_for_product(
            "binance",
            "BTCUSDT",
            FeeProduct::Perp,
            common::time::now_ms(),
        )
        .is_none());
    assert_eq!(
        registry.plan_leg_sizing_for_product(
            "binance",
            "BTCUSDT",
            FeeProduct::Perp,
            1_000.0,
            30_000.0,
        ),
        Err(SizingBlock::SpecMissing)
    );

    registry.record_refresh_success("binance", checked_at_ms);
    assert!(registry
        .hedge_instrument_at_for_product(
            "binance",
            "BTCUSDT",
            FeeProduct::Perp,
            checked_at_ms + INSTRUMENT_SPEC_FRESHNESS_MS,
        )
        .is_none());
}

#[test]
fn replace_rejects_schema_or_endpoint_drift() {
    let registry = InstrumentRegistry::default();
    let mut wrong_schema = constructible("BTCUSDT", "BTC");
    wrong_schema.schema_version = Some("drifted-schema".to_owned());
    assert_eq!(registry.replace_venue("binance", vec![wrong_schema]), 0);

    let mut wrong_endpoint = constructible("BTCUSDT", "BTC");
    wrong_endpoint.source_url = Some("/unverified/instruments".to_owned());
    assert_eq!(registry.replace_venue("binance", vec![wrong_endpoint]), 0);

    let mut endpoint_prefix_collision = constructible("BTCUSDT", "BTC");
    endpoint_prefix_collision.source_url = Some("/fapi/v1/exchangeInfo-copy".to_owned());
    assert_eq!(
        registry.replace_venue("binance", vec![endpoint_prefix_collision]),
        0
    );
    assert_eq!(registry.len(), 0);
}

#[test]
fn metadata_registry_accepts_all_venue_family_evidence() {
    let cases = [
        (
            "binance",
            "/fapi/v1/exchangeInfo",
            "binance-usdm-futures-exchange-information-usdt-usdc-2026-07-11",
        ),
        (
            "okx",
            "/api/v5/public/instruments",
            "okx-v5-public-get-instruments-2026-06-03",
        ),
        (
            "bybit",
            "/v5/market/instruments-info?category=linear",
            "bybit-v5-get-instruments-info-2026-07-13",
        ),
        (
            "bitget",
            "/api/v3/market/instruments?category=USDT-FUTURES",
            "bitget-uta-get-instruments-2026-06-03",
        ),
        (
            "gate",
            "/api/v4/futures/usdt/contracts",
            "gate-apiv4-list-futures-contracts-2026-06-03",
        ),
        (
            "kucoin",
            "https://www.kucoin.com/docs-new/rest/futures-trading/market-data/get-all-symbols",
            "kucoin-futures-native-contract-matrix-2026-07-11",
        ),
        (
            "hyperliquid",
            "/info {\"type\":\"meta\"}",
            "hyperliquid-perpetuals-meta-and-asset-ctxs-2026-06-03",
        ),
    ];

    for (venue, source_url, schema_version) in cases {
        let registry = InstrumentRegistry::default();
        let mut instrument = constructible("BTCUSDT", "BTC");
        instrument.venue = venue.to_owned();
        instrument.source_url = Some(source_url.to_owned());
        instrument.schema_version = Some(schema_version.to_owned());
        assert_eq!(
            registry.replace_venue(venue, vec![instrument]),
            1,
            "{venue} evidence must match the runtime registry"
        );
    }
}

#[test]
fn runtime_health_reports_execution_ready_schema_and_failure_state() {
    let registry = InstrumentRegistry::default();
    let instrument = constructible("BTCUSDT", "BTC");
    let checked_at_ms = instrument.checked_at_ms;
    let schema = instrument.schema_version.clone().unwrap_or_default();
    assert_eq!(registry.replace_venue("binance", vec![instrument]), 1);

    let ready = registry
        .runtime_health(checked_at_ms + 1)
        .into_iter()
        .find(|row| row.venue == "binance")
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(ready.len(), 1, "binance runtime health must be unique");
    let ready = &ready[0];
    assert_eq!(ready.status, VenueOperationStatus::Ok);
    assert_eq!(ready.rows, 1);
    assert_eq!(ready.execution_ready_rows, 1);
    assert_eq!(ready.schema_versions, vec![schema]);
    assert_eq!(ready.source_urls, vec!["/fapi/v1/exchangeInfo"]);

    registry.record_refresh_failure("binance", "rate limited");
    let failed = registry
        .runtime_health(common::time::now_ms())
        .into_iter()
        .find(|row| row.venue == "binance")
        .into_iter()
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1, "failed binance health must be unique");
    let failed = &failed[0];
    assert_eq!(failed.status, VenueOperationStatus::Warn);
    assert_eq!(failed.rows, 1);
    assert_eq!(failed.execution_ready_rows, 0);
    assert!(failed.problem.is_some());
}

#[test]
fn scheduled_registry_refresh_includes_every_configured_hyperliquid_route() {
    for market in exchange::HyperliquidMarket::CONFIGURED_MARKETS {
        assert!(
            SUPPORTED_VENUES.contains(&market.venue()),
            "{} must be warmed by the instrument registry",
            market.venue()
        );
    }
}

#[tokio::test]
async fn checkpoint_restore_is_visible_but_requires_a_fresh_probe_for_execution(
) -> anyhow::Result<()> {
    let path = std::env::temp_dir().join(format!(
        "crossline-instrument-registry-{}-{}.json",
        std::process::id(),
        common::time::now_ms()
    ));
    let registry = InstrumentRegistry::load(Some(path.clone())).await;
    let now_ms = common::time::now_ms();
    let mut perpetual = constructible("BTCUSDT", "BTC");
    perpetual.checked_at_ms = now_ms;
    let mut spot = official_spot("SOLUSDC", "SOL", "USDC");
    spot.checked_at_ms = now_ms;
    assert_eq!(registry.replace_venue("binance", vec![perpetual]), 1);
    assert_eq!(registry.replace_spot_venue("binance", vec![spot]), 1);
    registry
        .persist_checkpoint()
        .await
        .map_err(anyhow::Error::msg)?;

    let restored = InstrumentRegistry::load(Some(path.clone())).await;

    assert_eq!(restored.len(), 2);
    assert!(matches!(
        restored.spot_registry_state("binance", now_ms),
        SpotRegistryState::Stale { .. }
    ));
    assert_eq!(
        restored
            .resolve_spot_instrument_evidence("binance", "SOL", "USDC", now_ms)
            .status,
        SpotInstrumentResolutionStatus::Stale
    );
    assert_eq!(
        restored.exact_spot_listing_evidence("binance", "SOL", "USDC", now_ms),
        None
    );
    assert!(restored
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Perp)
        .is_none());
    let health = restored
        .runtime_health(now_ms)
        .into_iter()
        .find(|row| row.venue == "binance")
        .unwrap_or_else(|| panic!("binance runtime health must exist"));
    assert_eq!(health.status, VenueOperationStatus::Warn);
    assert_eq!(health.execution_ready_rows, 0);
    assert!(health.message.contains("historical instrument rows"));
    assert!(restored.spot_instrument_refresh_retry_due("binance", now_ms, 30_000));

    restored.record_refresh_success("binance", now_ms);
    restored.record_spot_refresh_success("binance", now_ms);
    assert_eq!(
        restored.exact_spot_listing_evidence("binance", "SOL", "USDC", now_ms),
        Some(true)
    );
    assert!(restored
        .resolve_hedge_instrument_for_product("binance", "BTCUSDT", FeeProduct::Perp)
        .is_some());
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn corrupt_checkpoint_starts_with_an_empty_fail_closed_registry() {
    let path = std::env::temp_dir().join(format!(
        "crossline-instrument-registry-corrupt-{}-{}.json",
        std::process::id(),
        common::time::now_ms()
    ));
    assert!(std::fs::write(&path, b"not-json").is_ok());

    let registry = InstrumentRegistry::load(Some(path.clone())).await;

    assert_eq!(registry.len(), 0);
    assert_eq!(
        registry.exact_spot_listing_evidence("binance", "SOL", "USDC", 1),
        None
    );
    let _ = std::fs::remove_file(path);
}
