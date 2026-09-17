use super::*;
use pretty_assertions::assert_eq;
use std::collections::HashSet;

#[derive(serde::Deserialize)]
struct PublicMarkIndexFixture {
    mark_price: crate::adapters::okx_response::OkxResponse<OkxMarkPriceItem>,
    index_tickers: crate::adapters::okx_response::OkxResponse<OkxIndexTickerItem>,
    open_interest: crate::adapters::okx_response::OkxResponse<OkxOpenInterestItem>,
}

#[test]
fn parse_mark_index_requires_mark_and_uses_official_fields() {
    let row = parse_mark_index(
        "BTC-USDT-SWAP",
        "200",
        Some("199.5"),
        Some("5000"),
        Some("50000"),
        1_597_026_383_085,
    )
    .expect("mark/index parses");

    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.exchange, "okx");
    assert_eq!(row.mark_price, 200.0);
    assert_eq!(row.index_price, Some(199.5));
    assert_eq!(row.open_interest, Some(5000.0));
    assert_eq!(row.open_interest_value, Some(50_000.0));
    assert_eq!(row.timestamp, 1_597_026_383_085);
}

#[test]
fn parse_mark_index_rejects_missing_mark() {
    assert!(parse_mark_index("BTC-USDT-SWAP", "", Some("1"), None, None, 0).is_none());
}

#[test]
fn parse_rows_joins_index_and_open_interest() {
    let rows = parse_mark_index_rows(
        vec![OkxMarkPriceItem {
            inst_id: "BTC-USDT-SWAP".into(),
            mark_px: "200".into(),
            ts: "1597026383085".into(),
        }],
        vec![OkxIndexTickerItem {
            inst_id: "BTC-USDT".into(),
            idx_px: "199.5".into(),
        }],
        vec![OkxOpenInterestItem {
            inst_id: "BTC-USDT-SWAP".into(),
            oi: "5000".into(),
            oi_usd: "50000".into(),
        }],
        None,
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].index_price, Some(199.5));
    assert_eq!(rows[0].open_interest_value, Some(50_000.0));
}

#[test]
fn index_and_swap_ids_round_trip() {
    assert_eq!(index_id_for_swap("BTC-USDT-SWAP"), "BTC-USDT");
    assert_eq!(swap_id_for_index("BTC-USDT"), "BTC-USDT-SWAP");
    assert_eq!(swap_id_for_index("BTC-USDT-SWAP"), "BTC-USDT-SWAP");
}

#[test]
fn okx_mark_index_open_interest_parse_official_fixture() {
    let fixture = include_str!("../../fixtures/okx/public_funding_mark_index_oi_btc_eth_usdt.json");
    let response: PublicMarkIndexFixture =
        serde_json::from_str(fixture).expect("okx public mark/index fixture");
    let requested = HashSet::from(["BTC-USDT-SWAP".to_owned()]);
    let rows = parse_mark_index_rows(
        response
            .mark_price
            .into_data("mark-price")
            .expect("mark rows"),
        response
            .index_tickers
            .into_data("index-tickers")
            .expect("index rows"),
        response
            .open_interest
            .into_data("open-interest")
            .expect("open interest rows"),
        Some(&requested),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "okx");
    assert_eq!(rows[0].symbol, "BTC");
    assert_eq!(rows[0].mark_price, 66_339.1);
    assert_eq!(rows[0].index_price, Some(66_381.6));
    assert_eq!(rows[0].open_interest, Some(3_781_173.650_000_016));
    assert_eq!(rows[0].open_interest_value, Some(2_508_003_326.787_561));
    assert_eq!(rows[0].timestamp, 1_780_442_859_121);
}
