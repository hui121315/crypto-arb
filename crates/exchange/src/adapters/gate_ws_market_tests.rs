use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value as JsonValue;

#[test]
fn subscribe_payload_matches_gate_schema() {
    let value: JsonValue = serde_json::from_str(&subscribe_payload("BTC")).unwrap();
    assert_eq!(value["channel"], CHANNEL);
    assert_eq!(value["event"], "subscribe");
    assert_eq!(value["payload"][0], "ob.BTC_USDT.50");
}

#[test]
fn unsubscribe_payload_matches_gate_schema() {
    let value: JsonValue = serde_json::from_str(&unsubscribe_payload("ETH_USDT")).unwrap();
    assert_eq!(value["event"], "unsubscribe");
    assert_eq!(value["payload"][0], "ob.ETH_USDT.50");
}

#[test]
fn stream_symbol_normalizes_base_or_contract_symbol() {
    assert_eq!(stream_symbol("BTC"), "BTC_USDT");
    assert_eq!(stream_symbol("BTC_USDT"), "BTC_USDT");
    assert_eq!(stream_symbol("BTC-USDT"), "BTC_USDT");
}

#[test]
fn parse_full_snapshot_extracts_orderbook_update() {
    let raw = r#"{
      "time": 1615366381,
      "channel": "futures.obu",
      "event": "update",
      "result": {
        "s": "ob.BTC_USDT.50",
        "u": 110,
        "t": 1615366381123,
        "full": true,
        "b": [["54672.1", "10"]],
        "a": [["54672.2", "5"]]
      }
    }"#;
    let update = parse_update(raw).unwrap();
    assert!(update.full);
    assert_eq!(update.contract, "BTC_USDT");
    assert_eq!(update.first_id, 0);
    assert_eq!(update.update_id, 110);
    assert_eq!(update.bids, vec![[54672.1, 10.0]]);
    assert_eq!(update.asks, vec![[54672.2, 5.0]]);
    let book = update.to_book();
    assert_eq!(book.symbol, "BTC");
    assert_eq!(book.exchange, "gate");
    assert_eq!(book.timestamp, 1_615_366_381_123);
}

#[test]
fn apply_delta_updates_deletes_and_sorts() {
    let mut book = OrderBookInfo {
        symbol: "BTC".into(),
        exchange: "gate".into(),
        bids: vec![[100.0, 1.0], [99.0, 1.0]],
        asks: vec![[101.0, 1.0], [102.0, 1.0]],
        timestamp: 1,
    };
    let update = BookUpdate {
        contract: "BTC_USDT".into(),
        first_id: 2,
        update_id: 2,
        timestamp_ms: 2,
        full: false,
        bids: vec![[100.0, 0.0], [98.0, 3.0]],
        asks: vec![[101.0, 4.0], [100.5, 1.0]],
    };
    apply_update(&mut book, &update);
    assert_eq!(book.bids, vec![[99.0, 1.0], [98.0, 3.0]]);
    assert_eq!(book.asks, vec![[100.5, 1.0], [101.0, 4.0], [102.0, 1.0]]);
    assert_eq!(book.timestamp, 2);
}

#[test]
fn continuity_checks_allow_covered_next_id_only() {
    let update = BookUpdate {
        contract: "BTC_USDT".into(),
        first_id: 8,
        update_id: 10,
        timestamp_ms: 1,
        full: false,
        bids: vec![[1.0, 1.0]],
        asks: Vec::new(),
    };
    assert!(update.is_next_for(7));
    assert!(!update.is_next_for(8));
    assert!(!update.is_next_for(6));
    assert!(update.is_stale(10));
}

#[test]
fn official_millisecond_timestamp_is_not_scaled_again() {
    let raw = r#"{
      "channel":"futures.obu","event":"update",
      "result":{"s":"ob.BTC_USDT.50","u":1,"t":1743673026995,
      "full":true,"b":[["100","1"]],"a":[["101","2"]]}
    }"#;
    let update = parse_update(raw).expect("official millisecond frame");
    assert_eq!(update.timestamp_ms, 1_743_673_026_995);
}

#[test]
fn parse_ignores_ack_and_other_channels() {
    assert!(parse_update(r#"{"event":"subscribe","channel":"futures.obu"}"#).is_none());
    assert!(
        parse_update(r#"{"event":"update","channel":"futures.tickers","result":{}}"#).is_none()
    );
}

#[test]
fn subscription_capacity_evicts_oldest_without_evicting_new_symbol() {
    let subscriptions = DashMap::new();
    for index in 0..=MAX_ACTIVE_SUBSCRIPTIONS {
        subscriptions.insert(
            format!("ASSET{index}_USDT"),
            SubscriptionState {
                last_touched_ms: index as i64,
                sent_on_current_connection: true,
            },
        );
    }
    let protected = "ASSET0_USDT";

    let evicted = take_over_capacity(&subscriptions, protected, MAX_ACTIVE_SUBSCRIPTIONS);

    assert_eq!(subscriptions.len(), MAX_ACTIVE_SUBSCRIPTIONS);
    assert!(subscriptions.contains_key(protected));
    assert_eq!(evicted, vec!["ASSET1_USDT"]);
}

#[test]
fn depth_stream_round_trips_contract_identity() {
    assert_eq!(depth_stream("ADA_USDT"), "ob.ADA_USDT.50");
    assert_eq!(
        contract_from_stream("ob.ADA_USDT.50").as_deref(),
        Some("ADA_USDT")
    );
    assert!(contract_from_stream("ADA_USDT").is_none());
}

#[test]
fn subscription_claim_prevents_duplicate_send_on_one_connection() {
    let subscriptions = DashMap::new();
    subscriptions.insert(
        "ADA_USDT".into(),
        SubscriptionState {
            last_touched_ms: 1,
            sent_on_current_connection: false,
        },
    );

    assert!(claim_subscription(&subscriptions, "ADA_USDT"));
    assert!(!claim_subscription(&subscriptions, "ADA_USDT"));
    release_subscription(&subscriptions, "ADA_USDT");
    assert!(claim_subscription(&subscriptions, "ADA_USDT"));
}
