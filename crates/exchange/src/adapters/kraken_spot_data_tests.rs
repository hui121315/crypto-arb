use super::*;

#[test]
fn xstock_identity_and_native_case_are_not_crypto_or_live_permission() {
    let frame = parse_instrument_frame(include_str!(
        "../../fixtures/kraken/spot_v2_instrument_muxusd.json"
    ))
    .unwrap();
    assert_eq!(frame.rows[0].native_symbol, "MUx/USD");
    assert_eq!(frame.rows[0].canonical_symbol, "MUX");
    assert_eq!(frame.rows[0].asset_class, InstrumentAssetClass::Equity);
    assert!(!frame.rows[0].execution_supported);
    let rest = r#"{"error":[],"result":{"MUxUSD":{"wsname":"MUx/USD","aclass_base":"tokenized_asset","base":"MUx","quote":"ZUSD","pair_decimals":2,"lot_decimals":8,"ordermin":"0.01","costmin":"0.5","status":"online"}}}"#;
    let row = parse_asset_pairs(rest).unwrap().remove(0);
    assert_eq!(row.native_symbol, "MUx/USD");
    assert_eq!(row.asset_class, InstrumentAssetClass::Equity);
    assert!(!row.execution_supported);
}

const TICKER: &str = include_str!("../../fixtures/kraken/spot_v2_ticker_btcusd.json");
const INSTRUMENT: &str = include_str!("../../fixtures/kraken/spot_v2_instrument_btcusd.json");
const BOOK: &str = include_str!("../../fixtures/kraken/spot_v2_book_btcusd.json");
const ASSET_PAIRS: &str = include_str!("../../fixtures/kraken/spot_asset_pairs_pupsusd.json");

#[test]
fn ticker_parses_official_v2_shape_without_zeroing_bbo() {
    let rows = parse_ticker_frame(TICKER).expect("parse ticker fixture");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "BTC/USD");
    assert_eq!(rows[0].bid.to_string(), "42500.1");
    assert_eq!(
        rows[0].ask_size.map(|value| value.to_string()).as_deref(),
        Some("0.8")
    );
    assert_eq!(rows[0].exchange_ts_ms, Some(1_720_095_678_123));
}

#[test]
fn instrument_parses_official_execution_spec_fields() {
    let frame = parse_instrument_frame(INSTRUMENT).expect("parse instrument fixture");

    assert!(frame.snapshot);
    assert_eq!(frame.rows.len(), 1);
    let row = &frame.rows[0];
    assert_eq!(row.native_symbol, "BTC/USD");
    assert_eq!(row.canonical_symbol, "BTC");
    assert_eq!(row.price_tick, Some(0.1));
    assert_eq!(row.qty_step, Some(0.00000001));
    assert_eq!(row.min_notional, Some(0.5));
    assert!(row.has_official_provenance());
}

#[test]
fn asset_pairs_cold_start_keeps_pups_usd_selectable() {
    let rows = parse_asset_pairs(ASSET_PAIRS).expect("parse asset pairs fixture");

    assert_eq!(rows.len(), 1);
    let row = &rows[0];
    assert_eq!(row.native_symbol, "PUPS/USD");
    assert_eq!(row.canonical_symbol, "PUPS");
    assert_eq!(row.quote_asset.as_deref(), Some("USD"));
    assert_eq!(row.price_tick, Some(0.000001));
    assert_eq!(row.qty_step, Some(0.00001));
    assert_eq!(row.min_qty, Some(3000.0));
    assert_eq!(row.min_notional, Some(0.5));
    assert!(row.has_official_provenance());
}

#[test]
fn asset_pairs_cold_start_converts_legacy_names_to_v2_symbols() {
    let fixture = r#"{
        "error": [],
        "result": {
            "XXBTZUSD": {"wsname":"XBT/USD","base":"XXBT","quote":"ZUSD","pair_decimals":1,"lot_decimals":8,"ordermin":"0.00005","costmin":"0.5","status":"online"},
            "XDGUSD": {"wsname":"XDG/USD","base":"XXDG","quote":"ZUSD","pair_decimals":7,"lot_decimals":8,"ordermin":"50","costmin":"0.5","status":"online"}
        }
    }"#;

    let rows = parse_asset_pairs(fixture).expect("parse legacy asset pairs fixture");
    assert!(rows.iter().any(|row| row.native_symbol == "BTC/USD"));
    assert!(rows.iter().any(|row| row.native_symbol == "DOGE/USD"));
}

#[test]
fn book_parses_snapshot_checksum_and_levels() {
    let rows = parse_book_frame(BOOK).expect("parse book fixture");

    assert_eq!(rows.len(), 1);
    assert!(rows[0].snapshot);
    assert_eq!(rows[0].checksum, 3_310_070_434);
    assert_eq!(rows[0].bids[0].0.to_string(), "45283.5");
    assert_eq!(rows[0].asks[0].1.to_string(), "0.00100000");
}
