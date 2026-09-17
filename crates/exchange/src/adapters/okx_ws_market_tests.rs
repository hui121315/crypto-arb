use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_matches_okx_schema() {
    let value: Value = serde_json::from_str(&subscribe_payload("BTC")).unwrap();
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0]["channel"], "books");
    assert_eq!(value["args"][0]["instId"], "BTC-USDT-SWAP");
}

#[test]
fn unsubscribe_payload_matches_okx_schema() {
    let value: Value = serde_json::from_str(&unsubscribe_payload("ETH-USDT-SWAP")).unwrap();
    assert_eq!(value["op"], "unsubscribe");
    assert_eq!(value["args"][0]["channel"], "books");
    assert_eq!(value["args"][0]["instId"], "ETH-USDT-SWAP");
}

#[test]
fn stream_symbol_normalizes_base_or_contract_symbol() {
    assert_eq!(stream_symbol("BTC"), "BTC-USDT-SWAP");
    assert_eq!(stream_symbol("BTC-USDT"), "BTC-USDT-SWAP");
    assert_eq!(stream_symbol("BTC-USDT-SWAP"), "BTC-USDT-SWAP");
}

#[test]
fn parse_books_snapshot_extracts_orderbook_and_sequence() {
    let raw = r#"{
      "arg":{"channel":"books","instId":"BTC-USDT-SWAP"},
      "action":"snapshot",
      "data":[{
        "asks":[["41006.8","0.60038921","0","1"]],
        "bids":[["41006.3","0.30178218","0","2"]],
        "ts":"1629966436396",
        "seqId":10,
        "prevSeqId":-1
      }]
    }"#;
    let parsed = parse_book_update(raw).unwrap();
    assert_eq!(parsed.stream_symbol, "BTC-USDT-SWAP");
    assert_eq!(parsed.action, BookAction::Snapshot);
    assert_eq!(parsed.seq_id, 10);
    assert_eq!(parsed.prev_seq_id, -1);
    let book = parsed.to_book();
    assert_eq!(book.symbol, "BTC");
    assert_eq!(book.exchange, "okx");
    assert_eq!(book.timestamp, 1_629_966_436_396);
    assert_eq!(book.bids, vec![[41006.3, 0.301_782_18]]);
    assert_eq!(book.asks, vec![[41006.8, 0.600_389_21]]);
}

#[test]
fn parse_books_ignores_ack_and_other_channels() {
    assert!(parse_book_update(r#"{"event":"subscribe","arg":{"channel":"books"}}"#).is_none());
    let raw = r#"{"arg":{"channel":"tickers","instId":"BTC-USDT-SWAP"},"data":[]}"#;
    assert!(parse_book_update(raw).is_none());
}

#[test]
fn parse_levels_skips_invalid_rows() {
    let raw = vec![
        vec!["1".into(), "2".into(), "0".into(), "1".into()],
        vec!["bad".into(), "3".into(), "0".into(), "1".into()],
    ];
    assert_eq!(parse_levels(&raw), vec![[1.0, 2.0]]);
}

#[test]
fn incremental_update_deletes_inserts_and_sorts_levels() {
    let mut book = OrderBookInfo {
        symbol: "BTC".into(),
        exchange: "okx".into(),
        bids: vec![[100.0, 1.0], [99.0, 2.0]],
        asks: vec![[101.0, 1.0], [102.0, 2.0]],
        timestamp: 1,
    };
    let update = BookUpdate {
        stream_symbol: "BTC-USDT-SWAP".into(),
        action: BookAction::Update,
        bids: vec![[100.0, 0.0], [98.0, 3.0]],
        asks: vec![[101.0, 4.0], [100.5, 1.0]],
        timestamp_ms: 2,
        seq_id: 12,
        prev_seq_id: 10,
    };

    apply_update(&mut book, &update);

    assert_eq!(book.bids, vec![[99.0, 2.0], [98.0, 3.0]]);
    assert_eq!(book.asks, vec![[100.5, 1.0], [101.0, 4.0], [102.0, 2.0]]);
    assert_eq!(book.timestamp, 2);
}

#[test]
fn empty_incremental_heartbeat_is_preserved_for_sequence_continuity() {
    let raw = r#"{
      "arg":{"channel":"books","instId":"BTC-USDT-SWAP"},
      "action":"update",
      "data":[{"asks":[],"bids":[],"ts":"2","seqId":10,"prevSeqId":10}]
    }"#;

    let parsed = parse_book_update(raw).expect("official empty heartbeat");

    assert_eq!(parsed.action, BookAction::Update);
    assert!(parsed.bids.is_empty());
    assert!(parsed.asks.is_empty());
    assert_eq!(parsed.seq_id, parsed.prev_seq_id);
}

#[test]
fn subscription_claim_prevents_duplicate_send_on_one_connection() {
    let subscriptions = DashMap::new();
    subscriptions.insert(
        "BTC-USDT-SWAP".into(),
        SubscriptionState {
            last_touched_ms: 1,
            sent_on_current_connection: false,
        },
    );

    assert!(claim_subscription(&subscriptions, "BTC-USDT-SWAP"));
    assert!(!claim_subscription(&subscriptions, "BTC-USDT-SWAP"));
    release_subscription(&subscriptions, "BTC-USDT-SWAP");
    assert!(claim_subscription(&subscriptions, "BTC-USDT-SWAP"));
}
