use super::*;
use crate::adapters::hyperliquid_config::HyperliquidMarket;
use pretty_assertions::assert_eq;

fn btc_row() -> HyperliquidUniverseRow {
    HyperliquidUniverseRow {
        name: "BTC".into(),
        sz_decimals: 5,
        is_delisted: false,
    }
}

fn assert_recorded_schema(instrument: &VenueInstrument) {
    assert_eq!(instrument.schema_version.as_deref(), Some(SCHEMA_VERSION));
}

#[test]
fn meta_fixture_parses_universe() {
    let fixture = include_str!("../../fixtures/hyperliquid/meta.json");
    let meta: HyperliquidMeta = serde_json::from_str(fixture).expect("meta fixture");
    assert_eq!(meta.universe.len(), 4);
    assert_eq!(meta.universe[0].name, "BTC");
    assert_eq!(meta.universe[0].sz_decimals, 5);
    assert!(!meta.universe[0].is_delisted);
    assert!(meta.universe[3].is_delisted);
}

#[test]
fn into_venue_instrument_derives_ticks_from_sz_decimals() {
    let inst = HyperliquidInstrumentRule::from_row_for_market(btc_row(), None)
        .expect("rule")
        .venue_instrument_for_market(
            HyperliquidMarket::Core,
            InstrumentAssetClass::Crypto,
            1_700_000_000_000,
        );

    assert_eq!(inst.venue, "hyperliquid");
    assert_eq!(inst.native_symbol, "BTC");
    assert_eq!(inst.canonical_symbol, "BTC");
    assert_eq!(inst.quote_asset.as_deref(), Some("USDC"));
    assert_eq!(inst.settle_asset.as_deref(), Some("USDC"));
    assert_eq!(inst.contract_size, Some(1.0));
    // szDecimals=5 → qty_step 1e-5；price_tick 1e-(6-5)=1e-1。
    assert_eq!(inst.qty_step, Some(1e-5));
    assert_eq!(inst.min_qty, Some(1e-5));
    assert_eq!(inst.price_tick, Some(1e-1));
    assert_eq!(inst.min_notional, None);
    assert_eq!(inst.funding_interval_ms, Some(60 * 60 * 1000));
    assert_eq!(inst.listing_status, InstrumentListingStatus::Trading);
    assert_eq!(inst.source, InstrumentMetadataSource::OfficialEndpoint);
    assert_recorded_schema(&inst);
    assert!(inst.is_hedge_constructible());
}

#[test]
fn zero_sz_decimals_yields_unit_qty_step() {
    let inst = HyperliquidInstrumentRule::from_row_for_market(
        HyperliquidUniverseRow {
            name: "HPOS".into(),
            sz_decimals: 0,
            is_delisted: false,
        },
        None,
    )
    .expect("rule")
    .venue_instrument_for_market(HyperliquidMarket::Core, InstrumentAssetClass::Crypto, 1);

    assert_eq!(inst.qty_step, Some(1.0));
    assert_eq!(inst.min_qty, Some(1.0));
    // price_tick 1e-6。
    assert_eq!(inst.price_tick, Some(1e-6));
    assert!(inst.is_hedge_constructible());
}

#[test]
fn parser_rejects_missing_name() {
    let err = HyperliquidInstrumentRule::from_row_for_market(
        HyperliquidUniverseRow {
            name: "  ".into(),
            ..btc_row()
        },
        None,
    )
    .expect_err("missing name");
    assert!(err.to_string().contains("missing name"));
}

#[test]
fn parser_rejects_out_of_range_sz_decimals() {
    let err = HyperliquidInstrumentRule::from_row_for_market(
        HyperliquidUniverseRow {
            sz_decimals: 9,
            ..btc_row()
        },
        None,
    )
    .expect_err("sz decimals too large");
    assert!(err.to_string().contains("szDecimals"));
}

#[test]
fn delisted_asset_maps_to_non_constructible() {
    let inst = HyperliquidInstrumentRule::from_row_for_market(
        HyperliquidUniverseRow {
            name: "LOOM".into(),
            sz_decimals: 1,
            is_delisted: true,
        },
        None,
    )
    .expect("rule")
    .venue_instrument_for_market(HyperliquidMarket::Core, InstrumentAssetClass::Crypto, 1);

    assert_eq!(inst.listing_status, InstrumentListingStatus::Delisted);
    assert!(!inst.is_hedge_constructible());
}

#[test]
fn metadata_cache_maps_all_universe_entries() {
    let fixture = include_str!("../../fixtures/hyperliquid/meta.json");
    let meta: HyperliquidMeta = serde_json::from_str(fixture).expect("meta fixture");
    let mapped = instrument_cache_from_metadata(
        HyperliquidInstrumentMetadata {
            meta,
            builder_dex_index: None,
            categories: Vec::new(),
        },
        HyperliquidMarket::Core,
        1,
    )
    .expect("core metadata")
    .instruments();

    // 全部 4 个 universe 行都映射（含 delisted，状态置 Delisted）。
    assert_eq!(mapped.len(), 4);
    let trading: Vec<_> = mapped
        .iter()
        .filter(|inst| inst.listing_status == InstrumentListingStatus::Trading)
        .map(|inst| inst.native_symbol.as_str())
        .collect();
    assert_eq!(trading, vec!["BTC", "ETH", "HPOS"]);
}

#[test]
fn builder_metadata_uses_official_dex_scoped_asset_id_formula() {
    let cache = instrument_cache_from_metadata(
        HyperliquidInstrumentMetadata {
            meta: HyperliquidMeta {
                universe: vec![HyperliquidUniverseRow {
                    name: "xyz:CBRS".into(),
                    sz_decimals: 3,
                    is_delisted: false,
                }],
            },
            builder_dex_index: Some(1),
            categories: vec![("xyz:CBRS".into(), "stocks".into())],
        },
        HyperliquidMarket::XYZ,
        1,
    )
    .expect("builder metadata");

    let spec = cache.resolve("CBRS", 1).expect("cached builder spec");
    assert_eq!(spec.asset_id, 110_000);
    assert_eq!(spec.qty_step, 0.001);
    assert_eq!(spec.price_tick, 0.001);

    let instrument = cache.instruments().pop().expect("registry entry");
    assert_eq!(instrument.venue, "hyperliquid:xyz");
    assert_eq!(instrument.native_symbol, "xyz:CBRS");
    assert_eq!(instrument.builder_dex.as_deref(), Some("xyz"));
    assert_eq!(instrument.asset_class, InstrumentAssetClass::Equity);
}

#[test]
fn metadata_cache_expires_before_reusing_asset_or_lot_rules() {
    let cache = instrument_cache_from_metadata(
        HyperliquidInstrumentMetadata {
            meta: HyperliquidMeta {
                universe: vec![HyperliquidUniverseRow {
                    name: "xyz:CBRS".into(),
                    sz_decimals: 3,
                    is_delisted: false,
                }],
            },
            builder_dex_index: Some(1),
            categories: vec![("xyz:CBRS".into(), "stocks".into())],
        },
        HyperliquidMarket::XYZ,
        1,
    )
    .expect("builder metadata");

    assert!(cache.resolve("CBRS", METADATA_CACHE_TTL_MS).is_ok());
    let error = cache
        .resolve("CBRS", METADATA_CACHE_TTL_MS + 1)
        .expect_err("expired metadata must block orders");
    assert!(error.to_string().contains("cache expired"));
}

#[test]
fn builder_metadata_fails_closed_when_dex_index_is_missing() {
    let error = instrument_cache_from_metadata(
        HyperliquidInstrumentMetadata {
            meta: HyperliquidMeta::default(),
            builder_dex_index: None,
            categories: Vec::new(),
        },
        HyperliquidMarket::XYZ,
        1,
    )
    .expect_err("builder metadata must include a dex index");

    assert!(error.to_string().contains("missing its dex index"));
}

#[test]
fn official_perp_categories_classify_each_builder_instrument_independently() {
    let fixture = include_str!("../../fixtures/hyperliquid/perp_categories_identity.json");
    let categories: HyperliquidPerpCategories =
        serde_json::from_str(fixture).expect("official perp categories fixture");
    let classes = category_asset_classes(categories).expect("valid category rows");

    assert_eq!(
        classes.get("XYZ:ZHIPU"),
        Some(&InstrumentAssetClass::Equity)
    );
    assert_eq!(classes.get("XYZ:SP500"), Some(&InstrumentAssetClass::Index));
    assert_eq!(classes.get("XYZ:DXY"), Some(&InstrumentAssetClass::Forex));
    assert_eq!(classes.get("FLX:BTC"), Some(&InstrumentAssetClass::Crypto));
    assert_eq!(
        classes.get("XYZ:GOLD"),
        Some(&InstrumentAssetClass::Unknown)
    );
    assert_eq!(
        classes.get("VNTL:OPENAI"),
        Some(&InstrumentAssetClass::Unknown)
    );
}

#[test]
fn builder_instrument_without_official_category_remains_unknown() {
    let cache = instrument_cache_from_metadata(
        HyperliquidInstrumentMetadata {
            meta: HyperliquidMeta {
                universe: vec![HyperliquidUniverseRow {
                    name: "xyz:NEW".into(),
                    sz_decimals: 3,
                    is_delisted: false,
                }],
            },
            builder_dex_index: Some(1),
            categories: Vec::new(),
        },
        HyperliquidMarket::XYZ,
        1,
    )
    .expect("missing category must not erase official instrument metadata");

    assert_eq!(
        cache.instruments()[0].asset_class,
        InstrumentAssetClass::Unknown
    );
}
