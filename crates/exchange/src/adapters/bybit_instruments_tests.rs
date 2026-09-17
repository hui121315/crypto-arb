use super::*;
use pretty_assertions::assert_eq;

#[derive(serde::Deserialize)]
struct InstrumentsInfoFixture {
    result: BybitInstrumentsPage,
}

fn btc_row() -> BybitInstrumentRow {
    BybitInstrumentRow {
        symbol: "BTCUSDT".into(),
        contract_type: "LinearPerpetual".into(),
        status: "Trading".into(),
        base_coin: "BTC".into(),
        quote_coin: "USDT".into(),
        settle_coin: "USDT".into(),
        symbol_type: String::new(),
        display_name: String::new(),
        funding_interval: 480,
        price_filter: BybitPriceFilter {
            tick_size: "0.10".into(),
        },
        lot_size_filter: BybitLotSizeFilter {
            min_order_qty: "0.001".into(),
            qty_step: "0.001".into(),
            min_notional_value: "5".into(),
        },
    }
}

fn assert_recorded_schema(instrument: &VenueInstrument) {
    assert_eq!(instrument.schema_version.as_deref(), Some(SCHEMA_VERSION));
}

#[test]
fn bybit_instrument_rule_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/bybit/instruments_info_linear_btcusdt.json");
    let parsed: InstrumentsInfoFixture =
        serde_json::from_str(fixture).expect("bybit instruments fixture");
    assert_eq!(parsed.result.next_page_cursor, "");
    let btc = parsed
        .result
        .list
        .into_iter()
        .find(|row| row.symbol == "BTCUSDT")
        .expect("btc row");
    let rule = BybitInstrumentRule::from_row(btc).expect("rule");

    assert_eq!(rule.base, "BTC");
    assert_eq!(rule.symbol, "BTCUSDT");
    assert_eq!(rule.price_tick, 0.1);
    assert_eq!(rule.qty_step, 0.001);
    assert_eq!(rule.min_qty, 0.001);
    assert_eq!(rule.min_notional, Some(5.0));
    // fundingInterval 480 分钟 → 8h 毫秒。
    assert_eq!(rule.funding_interval_ms, Some(480 * 60 * 1000));
}

#[test]
fn into_venue_instrument_maps_base_coin_sizing() {
    let inst = BybitInstrumentRule::from_row(btc_row())
        .expect("rule")
        .into_venue_instrument(1_700_000_000_000);

    assert_eq!(inst.venue, "bybit");
    assert_eq!(inst.native_symbol, "BTCUSDT");
    assert_eq!(inst.canonical_symbol, "BTC");
    assert_eq!(inst.quote_asset.as_deref(), Some("USDT"));
    assert_eq!(inst.settle_asset.as_deref(), Some("USDT"));
    // Bybit linear 直接以基础币下单 → contract_size 1。
    assert_eq!(inst.contract_size, Some(1.0));
    assert_eq!(inst.price_tick, Some(0.1));
    assert_eq!(inst.qty_step, Some(0.001));
    assert_eq!(inst.min_qty, Some(0.001));
    assert_eq!(inst.min_notional, Some(5.0));
    assert_eq!(inst.funding_interval_ms, Some(480 * 60 * 1000));
    assert_eq!(inst.listing_status, InstrumentListingStatus::Trading);
    assert_eq!(inst.source, InstrumentMetadataSource::OfficialEndpoint);
    assert_recorded_schema(&inst);
    assert!(inst.is_hedge_constructible());
}

#[test]
fn instrument_parser_rejects_unsupported_settle() {
    let err = BybitInstrumentRule::from_row(BybitInstrumentRow {
        quote_coin: "USDH".into(),
        settle_coin: "USDH".into(),
        ..btc_row()
    })
    .expect_err("unsupported settle");

    assert!(err.to_string().contains("supported linear settle"));
}

#[test]
fn instrument_parser_rejects_dated_future() {
    let err = BybitInstrumentRule::from_row(BybitInstrumentRow {
        contract_type: "LinearFutures".into(),
        ..btc_row()
    })
    .expect_err("dated future");

    assert!(err.to_string().contains("is not a perpetual"));
}

#[test]
fn instrument_parser_rejects_non_positive_qty_step() {
    let err = BybitInstrumentRule::from_row(BybitInstrumentRow {
        lot_size_filter: BybitLotSizeFilter {
            qty_step: "0".into(),
            ..btc_row().lot_size_filter
        },
        ..btc_row()
    })
    .expect_err("zero qty step");

    assert!(err.to_string().contains("qtyStep"));
}

#[test]
fn instruments_from_rows_keeps_usdt_and_usdc_perpetuals() {
    let rows = vec![
        btc_row(),
        BybitInstrumentRow {
            symbol: "ETHPERP".into(),
            base_coin: "ETH".into(),
            quote_coin: "USDC".into(),
            settle_coin: "USDC".into(),
            display_name: "ETHUSDC".into(),
            ..btc_row()
        },
        BybitInstrumentRow {
            symbol: "SOL-26SEP25".into(),
            base_coin: "SOL".into(),
            contract_type: "LinearFutures".into(),
            ..btc_row()
        },
    ];
    let mapped = instruments_from_rows(rows, 1);

    assert_eq!(mapped.len(), 2);
    assert_eq!(mapped[0].native_symbol, "BTCUSDT");
    assert_eq!(mapped[1].native_symbol, "ETHPERP");
    assert_eq!(mapped[1].settle_asset.as_deref(), Some("USDC"));
}

#[test]
fn bybit_identity_matrix_preserves_usdt_usdc_and_rwa_contracts() {
    let fixture = include_str!("../../fixtures/bybit/instruments_info_linear_identity_matrix.json");
    let parsed: InstrumentsInfoFixture =
        serde_json::from_str(fixture).expect("bybit identity matrix fixture");
    let rows = instruments_from_rows(parsed.result.list, 1_783_913_133_134);

    let find = |symbol: &str| {
        rows.iter()
            .find(|row| row.native_symbol == symbol)
            .unwrap_or_else(|| panic!("missing {symbol}"))
    };
    assert_eq!(find("BTCPERP").settle_asset.as_deref(), Some("USDC"));
    assert_eq!(find("BTCPERP").display_symbol, "BTCUSDC Perp");
    assert_eq!(find("AAPLUSDT").asset_class, InstrumentAssetClass::Equity);
    assert_eq!(find("XAUUSDT").asset_class, InstrumentAssetClass::Metal);
    assert_eq!(find("CLUSDT").asset_class, InstrumentAssetClass::Energy);
    assert!(rows.iter().all(VenueInstrument::is_hedge_constructible));
}

#[test]
fn delisted_contract_maps_to_non_constructible() {
    let inst = BybitInstrumentRule::from_row(BybitInstrumentRow {
        status: "Closed".into(),
        ..btc_row()
    })
    .expect("rule")
    .into_venue_instrument(1);

    assert_eq!(inst.listing_status, InstrumentListingStatus::Delisted);
    assert!(!inst.is_hedge_constructible());
}
