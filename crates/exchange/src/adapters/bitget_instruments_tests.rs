use super::*;
use pretty_assertions::assert_eq;

#[derive(serde::Deserialize)]
struct InstrumentsFixture {
    data: Vec<BitgetInstrumentRow>,
}

fn btc_row() -> BitgetInstrumentRow {
    BitgetInstrumentRow {
        symbol: "BTCUSDT".into(),
        category: "USDT-FUTURES".into(),
        base_coin: "BTC".into(),
        quote_coin: "USDT".into(),
        settle_coin: "USDT".into(),
        contract_type: "perpetual".into(),
        status: "online".into(),
        symbol_type: "crypto".into(),
        is_rwa: "NO".into(),
        is_reality: "NO".into(),
        price_multiplier: "0.1".into(),
        quantity_multiplier: "0.0001".into(),
        price_precision: "1".into(),
        quantity_precision: "4".into(),
        min_order_qty: "0.0001".into(),
        min_order_amount: "5".into(),
        fund_interval: "8".into(),
    }
}

fn assert_recorded_schema(instrument: &VenueInstrument) {
    assert_eq!(instrument.schema_version.as_deref(), Some(SCHEMA_VERSION));
}

#[test]
fn bitget_instrument_rule_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/bitget/uta_instruments_usdt_futures_btcusdt.json");
    let parsed: InstrumentsFixture =
        serde_json::from_str(fixture).expect("bitget instruments fixture");
    let btc = parsed
        .data
        .into_iter()
        .find(|row| row.symbol == "BTCUSDT")
        .expect("btc row");
    let rule = BitgetInstrumentRule::from_row(btc).expect("rule");

    assert_eq!(rule.base, "BTC");
    assert_eq!(rule.symbol, "BTCUSDT");
    assert_eq!(rule.price_tick, 0.1);
    assert_eq!(rule.qty_step, 0.0001);
    assert_eq!(rule.min_qty, 0.0001);
    assert_eq!(rule.min_notional, Some(5.0));
    // fundInterval 8 小时 → 毫秒。
    assert_eq!(rule.funding_interval_ms, Some(8 * 60 * 60 * 1000));
}

#[test]
fn into_venue_instrument_maps_base_coin_sizing() {
    let inst = BitgetInstrumentRule::from_row(btc_row())
        .expect("rule")
        .into_venue_instrument(1_700_000_000_000);

    assert_eq!(inst.venue, "bitget");
    assert_eq!(inst.native_symbol, "BTCUSDT");
    assert_eq!(inst.canonical_symbol, "BTC");
    assert_eq!(inst.quote_asset.as_deref(), Some("USDT"));
    assert_eq!(inst.settle_asset.as_deref(), Some("USDT"));
    // Bitget USDT-FUTURES 直接以基础币下单 → contract_size 1。
    assert_eq!(inst.contract_size, Some(1.0));
    assert_eq!(inst.price_tick, Some(0.1));
    assert_eq!(inst.qty_step, Some(0.0001));
    assert_eq!(inst.min_qty, Some(0.0001));
    assert_eq!(inst.min_notional, Some(5.0));
    assert_eq!(inst.funding_interval_ms, Some(8 * 60 * 60 * 1000));
    assert_eq!(inst.listing_status, InstrumentListingStatus::Trading);
    assert_eq!(inst.source, InstrumentMetadataSource::OfficialEndpoint);
    assert_recorded_schema(&inst);
    assert!(inst.is_hedge_constructible());
}

#[test]
fn coin_future_keeps_identity_but_is_observation_only() {
    let rule = BitgetInstrumentRule::from_row(BitgetInstrumentRow {
        symbol: "BTCUSD_CM".into(),
        category: "COIN-FUTURES".into(),
        quote_coin: "USD".into(),
        settle_coin: "BTC".into(),
        ..btc_row()
    })
    .expect("coin rule");
    let instrument = rule.into_venue_instrument(1);

    assert_eq!(instrument.native_symbol, "BTCUSD_CM");
    assert_eq!(instrument.quote_asset.as_deref(), Some("USD"));
    assert!(!instrument.execution_supported);
    assert!(instrument.is_observation_only());
}

#[test]
fn instrument_parser_rejects_delivery_future() {
    let err = BitgetInstrumentRule::from_row(BitgetInstrumentRow {
        contract_type: "delivery".into(),
        ..btc_row()
    })
    .expect_err("delivery future");

    assert!(err.to_string().contains("is not a perpetual"));
}

#[test]
fn instrument_parser_falls_back_to_official_quantity_precision() {
    let rule = BitgetInstrumentRule::from_row(BitgetInstrumentRow {
        quantity_multiplier: "0".into(),
        ..btc_row()
    })
    .expect("precision fallback");
    assert_eq!(rule.qty_step, 0.0001);

    let err = BitgetInstrumentRule::from_row(BitgetInstrumentRow {
        quantity_multiplier: "0".into(),
        quantity_precision: String::new(),
        ..btc_row()
    })
    .expect_err("missing qty step");

    assert!(err
        .to_string()
        .contains("quantityMultiplier/quantityPrecision"));
}

#[test]
fn instruments_from_rows_skips_delivery_and_keeps_all_native_categories() {
    let rows = vec![
        btc_row(),
        BitgetInstrumentRow {
            symbol: "ETHUSD".into(),
            base_coin: "ETH".into(),
            category: "COIN-FUTURES".into(),
            quote_coin: "USD".into(),
            ..btc_row()
        },
        BitgetInstrumentRow {
            symbol: "SOLUSDT_250926".into(),
            base_coin: "SOL".into(),
            contract_type: "delivery".into(),
            ..btc_row()
        },
    ];
    let mapped = instruments_from_rows(rows, 1);

    assert_eq!(mapped.len(), 2);
    assert_eq!(mapped[0].native_symbol, "BTCUSDT");
    assert!(!mapped[1].execution_supported);
}

#[test]
fn official_identity_matrix_covers_usdt_usdc_coin_and_reality_boundaries() {
    let fixture = include_str!("../../fixtures/bitget/uta_instruments_identity_matrix.json");
    let parsed: InstrumentsFixture = serde_json::from_str(fixture).expect("identity matrix");
    let (instruments, specs) = instruments_and_specs_from_rows(parsed.data, 1);
    assert_eq!(instruments.len(), 4);
    assert_eq!(specs.len(), 4);

    let usdc = instruments
        .iter()
        .find(|row| row.native_symbol == "BTCPERP")
        .expect("usdc future");
    assert_eq!(usdc.quote_asset.as_deref(), Some("USDC"));
    assert!(usdc.execution_supported);
    assert!(usdc.is_hedge_constructible());

    let coin = instruments
        .iter()
        .find(|row| row.native_symbol == "BTCUSD_CM")
        .expect("coin future");
    assert!(!coin.execution_supported);
    assert!(coin.is_observation_only());

    let reality = instruments
        .iter()
        .find(|row| row.native_symbol == "TSLAUSDT")
        .expect("reality future");
    assert_eq!(reality.asset_class, InstrumentAssetClass::Equity);
    assert!(!reality.execution_supported);
    assert!(reality.is_observation_only());
}

#[test]
fn native_identity_routes_exact_usdt_usdc_and_coin_symbols() {
    assert_eq!(
        native_identity("BTC").expect("USDT default"),
        BitgetNativeIdentity {
            category: BitgetUtaCategory::UsdtFutures,
            native_symbol: "BTCUSDT".to_owned(),
        }
    );
    assert_eq!(
        native_identity("BTC/USDC").expect("USDC pair"),
        BitgetNativeIdentity {
            category: BitgetUtaCategory::UsdcFutures,
            native_symbol: "BTCPERP".to_owned(),
        }
    );
    assert_eq!(
        native_identity("BTCUSD_CM").expect("COIN native"),
        BitgetNativeIdentity {
            category: BitgetUtaCategory::CoinFutures,
            native_symbol: "BTCUSD_CM".to_owned(),
        }
    );
}

#[test]
fn delisted_contract_maps_to_non_constructible() {
    let inst = BitgetInstrumentRule::from_row(BitgetInstrumentRow {
        status: "offline".into(),
        ..btc_row()
    })
    .expect("rule")
    .into_venue_instrument(1);

    assert_eq!(inst.listing_status, InstrumentListingStatus::Delisted);
    assert!(!inst.is_hedge_constructible());
}
