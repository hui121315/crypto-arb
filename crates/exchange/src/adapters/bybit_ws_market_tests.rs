use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_matches_bybit_schema() {
    let value: Value = serde_json::from_str(&subscribe_payload("BTC")).unwrap();
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0], "orderbook.50.BTCUSDT");
}

#[test]
fn unsubscribe_payload_matches_bybit_schema() {
    let value: Value = serde_json::from_str(&unsubscribe_payload("ETHUSDT")).unwrap();
    assert_eq!(value["op"], "unsubscribe");
    assert_eq!(value["args"][0], "orderbook.50.ETHUSDT");
}

#[test]
fn stream_symbol_normalizes_base_or_contract_symbol() {
    assert_eq!(stream_symbol("BTC"), "BTCUSDT");
    assert_eq!(stream_symbol("BTCUSDT"), "BTCUSDT");
    assert_eq!(stream_symbol("BTC-USDT"), "BTCUSDT");
    assert_eq!(stream_symbol("BTCUSDC"), "BTCUSDC");
    assert_eq!(stream_symbol("BTCPERP"), "BTCPERP");
}

#[test]
fn parse_snapshot_extracts_orderbook() {
    let raw = r#"{
      "topic":"orderbook.50.BTCUSDT",
      "type":"snapshot",
      "ts":1672304486865,
      "data":{
        "s":"BTCUSDT",
        "b":[["16493.50","0.006"]],
        "a":[["16611.00","0.029"]],
        "u":18521288
      }
    }"#;
    let parsed = parse_book_message(raw).unwrap();
    assert_eq!(parsed.stream_symbol, "BTCUSDT");
    match parsed.kind {
        BookMessageKind::Snapshot(snapshot) => {
            assert_eq!(snapshot.update_id, 18_521_288);
            assert_eq!(snapshot.book.symbol, "BTC");
            assert_eq!(snapshot.book.exchange, "bybit");
            assert_eq!(snapshot.book.timestamp, 1_672_304_486_865);
            assert_eq!(snapshot.book.bids, vec![[16493.5, 0.006]]);
            assert_eq!(snapshot.book.asks, vec![[16611.0, 0.029]]);
        }
        BookMessageKind::Delta(_) => panic!("expected snapshot"),
    }
}

#[test]
fn delta_updates_delete_and_sort_levels() {
    let mut book = OrderBookInfo {
        symbol: "BTC".into(),
        exchange: "bybit".into(),
        bids: vec![[100.0, 1.0], [99.0, 2.0]],
        asks: vec![[101.0, 1.0], [102.0, 2.0]],
        timestamp: 1,
    };
    let delta = BookDelta {
        bids: vec![[100.0, 0.0], [98.0, 3.0]],
        asks: vec![[101.0, 4.0], [100.5, 1.0]],
        timestamp: 2,
        update_id: 2,
    };
    apply_delta(&mut book, &delta);
    assert_eq!(book.bids, vec![[99.0, 2.0], [98.0, 3.0]]);
    assert_eq!(book.asks, vec![[100.5, 1.0], [101.0, 4.0], [102.0, 2.0]]);
    assert_eq!(book.timestamp, 2);
}

#[test]
fn update_id_one_resets_book_even_when_labeled_delta() {
    let raw = r#"{
      "topic":"orderbook.50.BTCUSDT","type":"delta","ts":2,
      "data":{"s":"BTCUSDT","b":[["100","1"]],"a":[["101","2"]],"u":1}
    }"#;
    let parsed = parse_book_message(raw).expect("service restart snapshot parses");
    assert!(matches!(parsed.kind, BookMessageKind::Snapshot(_)));
}

#[test]
fn parser_rejects_book_without_official_update_id() {
    let raw = r#"{
      "topic":"orderbook.50.BTCUSDT","type":"snapshot",
      "data":{"s":"BTCUSDT","b":[["100","1"]],"a":[["101","2"]]}
    }"#;
    assert!(parse_book_message(raw).is_none());
}

#[test]
fn parse_ignores_acks_and_other_topics() {
    assert!(parse_book_message(r#"{"op":"subscribe","success":true}"#).is_none());
    let raw = r#"{"topic":"tickers.BTCUSDT","type":"snapshot","data":{"s":"BTCUSDT"}}"#;
    assert!(parse_book_message(raw).is_none());
}
