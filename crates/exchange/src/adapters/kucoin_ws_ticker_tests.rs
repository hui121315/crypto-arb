use super::*;
use crate::adapters::kucoin_ws_ticker_data::ns_to_ms;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn snapshot_command_payload_targets_snapshot_topic() {
    let payload = command_payload("subscribe", SNAPSHOT_TOPIC_PREFIX, "XBTUSDTM");
    let value: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["type"], "subscribe");
    assert_eq!(value["topic"], "/contractMarket/snapshot:XBTUSDTM");
    assert_eq!(value["privateChannel"], false);
    assert_eq!(value["response"], true);
}

#[test]
fn ticker_command_payload_targets_tickerv2_topic() {
    let payload = command_payload("subscribe", TICKER_TOPIC_PREFIX, "ETHUSDTM");
    let value: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["topic"], "/contractMarket/tickerV2:ETHUSDTM");
}

#[test]
fn unsubscribe_command_payload_uses_unsubscribe_kind() {
    let payload = command_payload("unsubscribe", SNAPSHOT_TOPIC_PREFIX, "XBTUSDTM");
    let value: Value = serde_json::from_str(&payload).unwrap();
    assert_eq!(value["type"], "unsubscribe");
}

#[test]
fn parse_snapshot_24h_extracts_last_and_turnover() {
    // Schema mirrors:
    // <https://www.kucoin.com/docs-new/websocket-api/futures/get-symbol-snapshot>
    let raw = r#"{
        "type":"message",
        "topic":"/contractMarket/snapshot:XBTUSDTM",
        "subject":"snapshot.24h",
        "data":{
            "symbol":"XBTUSDTM",
            "lastPrice":75492.1,
            "highPrice":77683.0,
            "lowPrice":75188.0,
            "priceChgPct":-0.0278,
            "volume":44545.8656,
            "turnover":3404219299.89529,
            "openInterest":"31601.0065",
            "ts":1779511245529000000
        }
    }"#;
    let snap = parse_snapshot(raw).expect("snapshot parses");
    assert_eq!(snap.kucoin_symbol, "XBTUSDTM");
    assert!((snap.cached.last_price - 75_492.1).abs() < 1e-6);
    assert!((snap.cached.volume_24h_quote - 3_404_219_299.895_29).abs() < 1e-3);
    // ts is nanoseconds → ms.
    assert_eq!(snap.cached.data_timestamp_ms, 1_779_511_245_529);
}

#[test]
fn parse_snapshot_ignores_other_topics() {
    let raw = r#"{"topic":"/contractMarket/tickerV2:XBTUSDTM","data":{"bestBidPrice":"1","bestAskPrice":"2"}}"#;
    assert!(parse_snapshot(raw).is_none());
    let ack = r#"{"type":"welcome"}"#;
    assert!(parse_snapshot(ack).is_none());
}

#[test]
fn parse_snapshot_rejects_missing_volume_or_exchange_timestamp() {
    let missing_turnover = r#"{
        "topic":"/contractMarket/snapshot:XBTUSDTM",
        "data":{"lastPrice":75492.1,"ts":1779511245529000000}
    }"#;
    assert!(parse_snapshot(missing_turnover).is_none());

    let missing_ts = r#"{
        "topic":"/contractMarket/snapshot:XBTUSDTM",
        "data":{"lastPrice":75492.1,"turnover":3404219299.89529}
    }"#;
    assert!(parse_snapshot(missing_ts).is_none());
}

#[test]
fn parse_snapshot_rejects_nonpositive_exchange_timestamp() {
    let zero_ts = r#"{
        "topic":"/contractMarket/snapshot:XBTUSDTM",
        "data":{"lastPrice":75492.1,"turnover":3404219299.89529,"ts":0}
    }"#;
    assert!(parse_snapshot(zero_ts).is_none());

    let negative_ts = r#"{
        "topic":"/contractMarket/snapshot:XBTUSDTM",
        "data":{"lastPrice":75492.1,"turnover":3404219299.89529,"ts":-1}
    }"#;
    assert!(parse_snapshot(negative_ts).is_none());
}

#[test]
fn parse_ticker_v2_extracts_bid_ask() {
    let raw = r#"{
        "type":"message",
        "topic":"/contractMarket/tickerV2:XBTUSDTM",
        "subject":"tickerV2",
        "data":{
            "symbol":"XBTUSDTM",
            "sequence":45,
            "bestBidSize":2,
            "bestBidPrice":"75492.0",
            "bestAskPrice":"75492.2",
            "bestAskSize":3,
            "ts":1779511245530000000
        }
    }"#;
    let parsed = parse_ticker_v2(raw).expect("tickerV2 parses");
    assert_eq!(parsed.kucoin_symbol, "XBTUSDTM");
    assert!((parsed.cached.bid - 75_492.0).abs() < 1e-6);
    assert!((parsed.cached.ask - 75_492.2).abs() < 1e-6);
    assert_eq!(parsed.cached.data_timestamp_ms, 1_779_511_245_530);
}

#[test]
fn parse_ticker_v2_returns_none_when_bid_ask_missing() {
    let raw = r#"{"topic":"/contractMarket/tickerV2:XBTUSDTM","data":{}}"#;
    assert!(parse_ticker_v2(raw).is_none());
}

#[test]
fn parse_ticker_v2_rejects_missing_exchange_timestamp() {
    let raw = r#"{
        "topic":"/contractMarket/tickerV2:XBTUSDTM",
        "data":{"bestBidPrice":"75492.0","bestAskPrice":"75492.2"}
    }"#;

    assert!(parse_ticker_v2(raw).is_none());
}

#[test]
fn parse_ticker_v2_rejects_nonpositive_exchange_timestamp() {
    let raw = r#"{
        "topic":"/contractMarket/tickerV2:XBTUSDTM",
        "data":{"bestBidPrice":"75492.0","bestAskPrice":"75492.2","ts":0}
    }"#;

    assert!(parse_ticker_v2(raw).is_none());
}

#[test]
fn ns_to_ms_rounds_and_filters_invalid_values() {
    assert_eq!(
        ns_to_ms(Some(1_779_511_245_529_000_000)),
        Some(1_779_511_245_529)
    );
    assert_eq!(ns_to_ms(Some(0)), None);
    assert_eq!(ns_to_ms(Some(-1)), None);
    assert_eq!(ns_to_ms(None), None);
}
