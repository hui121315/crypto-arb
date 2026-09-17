use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_uses_v3_args_shape() {
    let value: Value = serde_json::from_str(&subscribe_payload("BTC")).unwrap();
    assert_eq!(value["op"], "subscribe");
    // V3 lower-cases instType and renames `channel` → `topic`, `instId` → `symbol`.
    assert_eq!(value["args"][0]["instType"], "usdt-futures");
    assert_eq!(value["args"][0]["topic"], "books");
    assert_eq!(value["args"][0]["symbol"], "BTCUSDT");
}

#[test]
fn unsubscribe_payload_uses_v3_args_shape() {
    let value: Value = serde_json::from_str(&unsubscribe_payload("ethusdt")).unwrap();
    assert_eq!(value["op"], "unsubscribe");
    assert_eq!(value["args"][0]["instType"], "usdt-futures");
    assert_eq!(value["args"][0]["topic"], "books");
    assert_eq!(value["args"][0]["symbol"], "ETHUSDT");
}

#[test]
fn spot_payload_and_snapshot_keep_pair_identity() {
    let value: Value =
        serde_json::from_str(&subscribe_payload_for("BTC/USDT", BitgetUtaCategory::Spot)).unwrap();
    assert_eq!(value["args"][0]["instType"], "spot");
    assert_eq!(value["args"][0]["symbol"], "BTCUSDT");

    let parsed = parse_book_message_for(
        r#"{
          "action":"snapshot",
          "arg":{"instType":"spot","topic":"books","symbol":"BTCUSDT"},
          "data":[{"b":[[10,2]],"a":[[11,3]],"ts":"1000","seq":"1"}]
        }"#,
        BitgetUtaCategory::Spot,
    )
    .unwrap();
    assert_eq!(snapshot_book(&parsed).unwrap().book.symbol, "BTC/USDT");
}

#[test]
fn parse_books_snapshot_extracts_orderbook_with_short_keys() {
    // V3 books channel: `a` / `b` short keys, numeric levels (see
    // <https://www.bitget.com/api-doc/uta/websocket/public/OrderBook-Channel>).
    let raw = r#"{
        "action": "snapshot",
        "arg": { "instType": "usdt-futures", "topic": "books", "symbol": "BTCUSDT" },
        "data": [{
            "a": [[75492.2, 29.8745], [75492.3, 0.1325]],
            "b": [[75492.1, 1.4162], [75492.0, 0.0001]],
            "ts": "1779511245532",
            "seq": 1,
            "checksum": 0
        }],
        "ts": 1779511245532
    }"#;
    let parsed = parse_book_message(raw).unwrap();
    assert_eq!(parsed.stream_symbol, "BTCUSDT");
    assert_eq!(parsed.action, BookAction::Snapshot);
    assert_eq!(parsed.seq, 1);
    let cached = snapshot_book(&parsed).unwrap();
    assert_eq!(cached.book.symbol, "BTC");
    assert_eq!(cached.book.exchange, "bitget");
    assert_eq!(cached.book.timestamp, 1_779_511_245_532);
    assert_eq!(cached.book.bids, vec![[75492.1, 1.4162], [75492.0, 0.0001]]);
    assert_eq!(
        cached.book.asks,
        vec![[75492.2, 29.8745], [75492.3, 0.1325]]
    );
}

#[test]
fn parse_ignores_acks_and_other_topics() {
    // Subscribe ack carries no `data`.
    assert!(parse_book_message(
        r#"{"event":"subscribe","arg":{"instType":"usdt-futures","topic":"books","symbol":"BTCUSDT"}}"#
    )
    .is_none());
    // Ticker frames must not feed the orderbook cache.
    let raw =
        r#"{"arg":{"instType":"usdt-futures","topic":"ticker","symbol":"BTCUSDT"},"data":[]}"#;
    assert!(parse_book_message(raw).is_none());
}

#[test]
fn parse_filters_invalid_levels() {
    // Zero-size updates must survive parsing so they can delete a cached level.
    let raw = r#"{
        "action": "snapshot",
        "arg": { "instType": "usdt-futures", "topic": "books", "symbol": "BTCUSDT" },
        "data": [{
            "a": [[75492.2, 0], [75492.3, 0.5]],
            "b": [[-1, 1], [75492.1, 1.4162]],
            "ts": "1779511245532",
            "seq": "10",
            "pseq": "0"
        }]
    }"#;
    let parsed = parse_book_message(raw).unwrap();
    assert_eq!(parsed.asks, vec![[75492.2, 0.0], [75492.3, 0.5]]);
    let cached = snapshot_book(&parsed).unwrap();
    assert_eq!(cached.book.asks, vec![[75492.3, 0.5]]);
    assert_eq!(cached.book.bids, vec![[75492.1, 1.4162]]);
}

#[test]
fn incremental_update_merges_without_dropping_untouched_side() {
    let snapshot = parse_book_message(
        r#"{
          "action":"snapshot",
          "arg":{"topic":"books","symbol":"OUSDT"},
          "data":[{"b":[[0.56,100],[0.55,200]],"a":[[0.57,300],[0.58,400]],"ts":"1000","seq":"100","pseq":"0"}]
        }"#,
    )
    .unwrap();
    let mut cached = snapshot_book(&snapshot).unwrap();
    let update = parse_book_message(
        r#"{
          "action":"update",
          "arg":{"topic":"books","symbol":"OUSDT"},
          "data":[{"b":[[0.56,0],[0.555,500]],"a":[],"ts":"1050","seq":"101","pseq":"100"}]
        }"#,
    )
    .unwrap();

    assert_eq!(
        apply_incremental(&mut cached, &update),
        MergeOutcome::Applied
    );
    assert_eq!(cached.book.bids, vec![[0.555, 500.0], [0.55, 200.0]]);
    assert_eq!(cached.book.asks, vec![[0.57, 300.0], [0.58, 400.0]]);
    assert_eq!(cached.last_seq, 101);
}

#[test]
fn first_incremental_accepts_snapshot_sequence_inside_update_range() {
    let snapshot = parse_book_message(
        r#"{
          "action":"snapshot",
          "arg":{"topic":"books","symbol":"OUSDT"},
          "data":[{"b":[[0.56,100]],"a":[[0.57,300]],"ts":"1000","seq":"105","pseq":"0"}]
        }"#,
    )
    .unwrap();
    let mut cached = snapshot_book(&snapshot).unwrap();
    let update = parse_book_message(
        r#"{
          "action":"update",
          "arg":{"topic":"books","symbol":"OUSDT"},
          "data":[{"b":[[0.56,120]],"a":[],"ts":"1050","seq":"110","pseq":"100"}]
        }"#,
    )
    .unwrap();

    assert_eq!(
        apply_incremental(&mut cached, &update),
        MergeOutcome::Applied
    );
}

#[test]
fn incremental_sequence_gap_fails_closed() {
    let snapshot = parse_book_message(
        r#"{
          "action":"snapshot",
          "arg":{"topic":"books","symbol":"OUSDT"},
          "data":[{"b":[[0.56,100]],"a":[[0.57,300]],"ts":"1000","seq":"100","pseq":"0"}]
        }"#,
    )
    .unwrap();
    let mut cached = snapshot_book(&snapshot).unwrap();
    let update = parse_book_message(
        r#"{
          "action":"update",
          "arg":{"topic":"books","symbol":"OUSDT"},
          "data":[{"b":[[0.56,120]],"a":[],"ts":"1050","seq":"120","pseq":"110"}]
        }"#,
    )
    .unwrap();

    assert_eq!(apply_incremental(&mut cached, &update), MergeOutcome::Gap);
}

#[test]
fn snapshot_requires_two_sided_non_crossed_book() {
    let one_sided = parse_book_message(
        r#"{
          "action":"snapshot",
          "arg":{"topic":"books","symbol":"OUSDT"},
          "data":[{"b":[[0.56,100]],"a":[],"ts":"1000","seq":"100","pseq":"0"}]
        }"#,
    )
    .unwrap();
    assert!(snapshot_book(&one_sided).is_none());
}

#[test]
fn truncate_book_clamps_requested_depth_to_default() {
    let book = OrderBookInfo {
        symbol: "BTC".into(),
        exchange: "bitget".into(),
        bids: vec![[3.0, 1.0], [2.0, 1.0], [1.0, 1.0]],
        asks: vec![[4.0, 1.0], [5.0, 1.0], [6.0, 1.0]],
        timestamp: 1,
    };
    let one_deep = truncate_book(&book, 1);
    assert_eq!(one_deep.bids, vec![[3.0, 1.0]]);
    assert_eq!(one_deep.asks, vec![[4.0, 1.0]]);
    // depth > DEPTH_LEVEL must still return the whole book without panic.
    let cap = truncate_book(&book, DEPTH_LEVEL + 100);
    assert_eq!(cap.bids.len(), 3);
    assert_eq!(cap.asks.len(), 3);
}
