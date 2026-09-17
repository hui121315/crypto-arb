use super::*;
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

#[test]
fn subscribe_payload_combines_ticker_and_level1_orderbook() {
    let symbols = vec!["BTCUSDT".to_owned(), "ETHUSDC".to_owned()];
    let value: Value = serde_json::from_str(&channel_payload("subscribe", &symbols)).unwrap();

    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0], "tickers.BTCUSDT");
    assert_eq!(value["args"][1], "orderbook.1.BTCUSDT");
    assert_eq!(value["args"][2], "tickers.ETHUSDC");
    assert_eq!(value["args"][3], "orderbook.1.ETHUSDC");
}

#[test]
fn depth_payload_subscribes_only_requested_symbols() {
    let value: Value =
        serde_json::from_str(&depth_channel_payload("subscribe", &["BTCUSDT".to_owned()])).unwrap();

    assert_eq!(value["args"][0], "orderbook.50.BTCUSDT");
}

#[test]
fn stream_symbols_normalize_and_deduplicate_pairs() {
    let rows = stream_symbols(&[
        "BTC".to_owned(),
        "BTC/USDT".to_owned(),
        "ETH-USDC".to_owned(),
    ])
    .expect("symbols normalize");

    assert_eq!(rows, vec!["BTCUSDT", "ETHUSDC"]);
    assert!(stream_symbols(&[]).is_none());
    assert!(stream_symbols(&["".to_owned()]).is_none());
}

#[test]
fn parse_spot_ticker_extracts_last_and_turnover() {
    let raw = r#"{
      "topic":"tickers.BTCUSDT",
      "type":"snapshot",
      "cs":102134,
      "ts":1760340000000,
      "data":{
        "symbol":"BTCUSDT",
        "lastPrice":"66666.60",
        "turnover24h":"4936790807.6521",
        "volume24h":"73191.3870"
      }
    }"#;

    let row = parse_ticker_update(raw).expect("ticker parses");
    assert_eq!(row.item.symbol, "BTCUSDT");
    assert_eq!(row.item.last_price, "66666.60");
    assert_eq!(row.item.turnover_24h, "4936790807.6521");
    assert_eq!(row.data_timestamp_ms, 1_760_340_000_000);
}

#[test]
fn parse_spot_book_extracts_best_bid_ask_and_accepts_string_ts() {
    let raw = r#"{
      "topic":"orderbook.1.BTCUSDT",
      "ts":"1760340000123",
      "type":"snapshot",
      "data":{
        "s":"BTCUSDT",
        "b":[["66666.50","0.42"]],
        "a":[["66666.70","0.39"]],
        "u":1,
        "seq":7961638724
      }
    }"#;

    let row = parse_book_update(raw).expect("book parses");
    assert_eq!(row.item.symbol, "BTCUSDT");
    assert_eq!(row.item.bids[0], ["66666.50", "0.42"]);
    assert_eq!(row.item.asks[0], ["66666.70", "0.39"]);
    assert_eq!(row.depth, 1);
    assert_eq!(row.update_id, 1);
    assert!(row.snapshot);
    assert_eq!(row.data_timestamp_ms, 1_760_340_000_123);
}

#[test]
fn parse_spot_tick_combines_ticker_and_book_rows() {
    let ticker = CachedTicker {
        item: SpotTickerUpdate {
            symbol: "BTCUSDT".into(),
            last_price: "66666.60".into(),
            turnover_24h: "4936790807.6521".into(),
        },
        cached_at_ms: now_ms(),
        data_timestamp_ms: 1_760_340_000_000,
    };
    let book = CachedBook {
        item: SpotBookUpdate {
            symbol: "BTCUSDT".into(),
            bids: vec![["66666.50".into(), "0.42".into()]],
            asks: vec![["66666.70".into(), "0.39".into()]],
            update_id: 1,
        },
        cached_at_ms: now_ms(),
        data_timestamp_ms: 1_760_340_000_123,
        update_id: 1,
    };

    let row = parse_spot_tick(&ticker, &book).expect("spot tick");
    assert_eq!(row.venue, "bybit");
    assert_eq!(row.symbol, "BTC/USDT");
    assert_eq!(row.bid, dec("66666.50"));
    assert_eq!(row.ask, dec("66666.70"));
    assert_eq!(row.last, dec("66666.60"));
    assert_eq!(row.bid_size, Some(dec("0.42")));
    assert_eq!(row.ask_size, Some(dec("0.39")));
    assert_eq!(row.volume_24h, dec("4936790807.6521"));
    assert_eq!(row.exchange_ts_ms, Some(1_760_340_000_123));
    assert!(row.received_at_ms > 0);
}

#[test]
fn depth_delta_merges_levels_instead_of_replacing_snapshot() {
    let cache = DashMap::new();
    let snapshot = parse_book_update(
        r#"{"topic":"orderbook.50.BTCUSDT","type":"snapshot","ts":1,
        "data":{"s":"BTCUSDT","b":[["100","2"],["99","3"]],
        "a":[["101","4"],["102","5"]],"u":10}}"#,
    )
    .expect("snapshot");
    ingest_book_update(&cache, snapshot);
    let delta = parse_book_update(
        r#"{"topic":"orderbook.50.BTCUSDT","type":"delta","ts":2,
        "data":{"s":"BTCUSDT","b":[["100","0"],["98","7"]],
        "a":[["101","6"]],"u":11}}"#,
    )
    .expect("delta");
    ingest_book_update(&cache, delta);

    let row = cache.get("BTCUSDT").expect("merged book");
    assert_eq!(
        row.item.bids,
        vec![
            [String::from("99"), String::from("3")],
            [String::from("98"), String::from("7")]
        ]
    );
    assert_eq!(row.item.asks[0], ["101", "6"]);
    assert_eq!(row.update_id, 11);
}

#[test]
fn parse_ignores_acks_other_topics_and_incomplete_rows() {
    assert!(parse_ticker_update(r#"{"op":"subscribe","success":true}"#).is_none());
    assert!(parse_ticker_update(r#"{"topic":"publicTrade.BTCUSDT","data":{}}"#).is_none());
    assert!(parse_ticker_update(
        r#"{"topic":"tickers.BTCUSDT","data":{"symbol":"BTCUSDT","lastPrice":"1"}}"#
    )
    .is_none());
    assert!(parse_book_update(r#"{"topic":"orderbook.50.BTCUSDT","data":{}}"#).is_none());
    assert!(parse_book_update(
        r#"{"topic":"orderbook.1.BTCUSDT","data":{"s":"BTCUSDT","b":[],"a":[]}}"#
    )
    .is_none());
}

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("valid decimal")
}
