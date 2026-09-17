use super::*;
use pretty_assertions::assert_eq;
use rust_decimal::Decimal;
use serde_json::Value;
use std::str::FromStr;

#[test]
fn subscribe_payload_matches_classic_spot_schema() {
    let value: Value = serde_json::from_str(&command_payload("subscribe", "BTC-USDT")).unwrap();
    assert_eq!(value["type"], "subscribe");
    assert_eq!(value["topic"], "/market/snapshot:BTC-USDT");
    assert_eq!(value["response"], true);
    assert!(value["id"]
        .as_str()
        .unwrap()
        .starts_with("spot-ticker-BTC-USDT-"));
}

#[test]
fn unsubscribe_payload_matches_classic_spot_schema() {
    let value: Value = serde_json::from_str(&command_payload("unsubscribe", "ETH-USDT")).unwrap();
    assert_eq!(value["type"], "unsubscribe");
    assert_eq!(value["topic"], "/market/snapshot:ETH-USDT");
    assert_eq!(value["response"], true);
}

#[test]
fn ticker_payload_matches_classic_bbo_schema() {
    let value: Value = serde_json::from_str(&all_tickers_command_payload("subscribe")).unwrap();
    assert_eq!(value["type"], "subscribe");
    assert_eq!(value["topic"], "/market/ticker:all");
    assert_eq!(value["response"], true);
}

#[test]
fn all_ticker_unsubscribe_uses_the_same_global_topic() {
    let value: Value = serde_json::from_str(&all_tickers_command_payload("unsubscribe")).unwrap();
    assert_eq!(value["type"], "unsubscribe");
    assert_eq!(value["topic"], "/market/ticker:all");
    assert_eq!(value["response"], true);
}

#[test]
fn stream_symbols_normalize_inputs_to_kucoin_spot_pairs() {
    let rows = stream_symbols(&[
        "BTC".to_owned(),
        "ETH/USDT".to_owned(),
        "SOL-USDC".to_owned(),
    ])
    .expect("symbols normalize");
    assert_eq!(rows, vec!["BTC-USDT", "ETH-USDT", "SOL-USDC"]);
    assert!(stream_symbols(&[]).is_none());
    assert!(stream_symbols(&["".to_owned()]).is_none());
}

#[test]
fn parse_spot_ticker_extracts_symbol_snapshot_payload() {
    let raw = r#"{
      "type": "message",
      "topic": "/market/snapshot:BTC-USDT",
      "subject": "trade.snapshot",
      "data": {
        "sequence": "1545896668986",
        "data": {
          "buy": 67220.0,
          "sell": 67220.1,
          "lastTradedPrice": 67220,
          "bidSize": 0.036,
          "askSize": 0.18,
          "datetime": 1704873323416,
          "volValue": 124068431.06726933
        }
      }
    }"#;
    let parsed = parse_spot_ticker(raw).expect("spot ticker parses");
    let tick = spot_tick_from_cached(&parsed.symbol, &parsed.cached).expect("spot tick builds");
    assert_eq!(parsed.symbol, "BTC-USDT");
    assert_eq!(tick.venue, "kucoin");
    assert_eq!(tick.symbol, "BTC/USDT");
    assert_eq!(tick.bid, dec("67220.0"));
    assert_eq!(tick.ask, dec("67220.1"));
    assert_eq!(tick.last, dec("67220"));
    assert_eq!(tick.bid_size, Some(dec("0.036")));
    assert_eq!(tick.ask_size, Some(dec("0.18")));
    assert_eq!(tick.volume_24h, dec("124068431.06726933"));
    assert_eq!(tick.exchange_ts_ms, Some(1_704_873_323_416));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn parse_spot_ticker_accepts_string_decimals_and_nested_24h_volume() {
    let raw = r#"{
      "topic": "/market/snapshot:ETH-USDT",
      "data": {
        "data": {
          "lastTradedPrice": "3000",
          "sell": "3001",
          "askSize": "2",
          "buy": "2999",
          "bidSize": "1",
          "datetime": 1729757723612,
          "marketChange24h": {"volValue": "999"}
        }
      }
    }"#;
    let parsed = parse_spot_ticker(raw).expect("spot ticker parses");
    let tick = spot_tick_from_cached(&parsed.symbol, &parsed.cached).unwrap();
    assert_eq!(tick.symbol, "ETH/USDT");
    assert_eq!(tick.exchange_ts_ms, Some(1_729_757_723_612));
    assert!(tick.received_at_ms > 0);
}

#[test]
fn parse_bbo_ticker_builds_row_without_fabricating_volume() {
    let raw = r#"{
      "type":"message","topic":"/market/ticker:all","subject":"ZIL-USDT",
      "data":{
        "price":"0.01231","bestBid":"0.01230","bestAsk":"0.01232",
        "bestBidSize":"1200","bestAskSize":"900","time":1729757723612
      }
    }"#;
    let parsed = parse_spot_ticker(raw).expect("bbo ticker parses");
    let tick = spot_tick_from_cached(&parsed.symbol, &parsed.cached).expect("spot tick builds");
    assert_eq!(tick.symbol, "ZIL/USDT");
    assert_eq!(tick.bid, dec("0.01230"));
    assert_eq!(tick.ask, dec("0.01232"));
    assert_eq!(tick.volume_24h, Decimal::ZERO);
}

#[test]
fn newer_bbo_keeps_snapshot_volume() {
    let snapshot = parse_spot_ticker(
        r#"{"topic":"/market/snapshot:ZIL-USDT","data":{"data":{"buy":"0.01230","sell":"0.01232","lastTradedPrice":"0.01231","bidSize":"1200","askSize":"900","datetime":1000,"volValue":"999"}}}"#,
    )
    .unwrap();
    let ticker = parse_spot_ticker(
        r#"{"topic":"/market/ticker:ZIL-USDT","data":{"price":"0.01233","bestBid":"0.01232","bestAsk":"0.01234","bestBidSize":"800","bestAskSize":"700","time":2000}}"#,
    )
    .unwrap();
    let mut merged = snapshot.cached;
    merge_cached_spot_ticker(&mut merged, ticker.cached);
    let tick = spot_tick_from_cached("ZIL-USDT", &merged).unwrap();
    assert_eq!(tick.last, dec("0.01233"));
    assert_eq!(tick.volume_24h, dec("999"));
    assert_eq!(tick.exchange_ts_ms, Some(2000));
}

#[test]
fn parse_ignores_ack_and_other_topics() {
    assert!(parse_spot_ticker(r#"{"id":"1","type":"ack"}"#).is_none());
    assert!(parse_spot_ticker(r#"{"topic":"/market/level2:BTC-USDT","data":{}}"#).is_none());
    assert!(parse_spot_ticker(r#"{"topic":"/market/snapshot:BTC-USDT","data":{}}"#).is_none());
}

#[test]
fn missing_first_frame_retries_are_bounded_and_fresh_rows_do_not_resubscribe() {
    let mut state = SubscriptionState {
        last_touched_ms: 1_000,
        detail_requested: true,
        subscribe_attempts: 1,
        last_subscribe_attempt_ms: 1_000,
    };

    assert!(!subscription_due(&state, false, 3_999));
    assert!(subscription_due(&state, false, 4_000));
    record_subscription_attempt(&mut state, 4_000);
    assert_eq!(state.subscribe_attempts, 2);
    assert!(!subscription_due(&state, true, 7_000));
    assert!(subscription_due(&state, false, 7_000));
    record_subscription_attempt(&mut state, 7_000);
    assert_eq!(state.subscribe_attempts, MAX_SUBSCRIBE_ATTEMPTS);
    assert!(!subscription_due(&state, false, 10_000));
}

#[test]
fn parse_spot_ticker_rejects_missing_exchange_timestamp() {
    let raw = r#"{
      "topic": "/market/snapshot:BTC-USDT",
      "data": {
        "data": {
          "lastTradedPrice": "67220",
          "sell": "67220.1",
          "askSize": "0.18",
          "buy": "67220.0",
          "bidSize": "0.036",
          "volValue": "1"
        }
      }
    }"#;

    assert!(parse_spot_ticker(raw).is_none());
}

#[test]
fn parse_spot_ticker_rejects_invalid_price_or_size_fields() {
    let bad_size = r#"{
      "topic": "/market/snapshot:BTC-USDT",
      "data": {
        "data": {
          "lastTradedPrice": "67220",
          "sell": "67220.1",
          "askSize": "bad",
          "buy": "67220.0",
          "bidSize": "0.036",
          "datetime": 1704873323416,
          "volValue": "1"
        }
      }
    }"#;
    assert!(parse_spot_ticker(bad_size).is_none());

    let zero_bid = r#"{
      "topic": "/market/snapshot:BTC-USDT",
      "data": {
        "data": {
          "lastTradedPrice": "67220",
          "sell": "67220.1",
          "askSize": "0.18",
          "buy": "0",
          "bidSize": "0.036",
          "datetime": 1704873323416,
          "volValue": "1"
        }
      }
    }"#;
    assert!(parse_spot_ticker(zero_bid).is_none());
}

fn dec(value: &str) -> Decimal {
    Decimal::from_str(value).expect("valid decimal")
}
