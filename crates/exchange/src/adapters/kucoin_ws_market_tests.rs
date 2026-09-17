use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscribe_payload_matches_kucoin_schema() {
    let value: Value = serde_json::from_str(&subscribe_payload("BTC")).unwrap();
    assert_eq!(value["type"], "subscribe");
    assert_eq!(value["topic"], "/contractMarket/level2Depth50:XBTUSDTM");
    assert_eq!(value["privateChannel"], false);
    assert_eq!(value["response"], true);
}

#[test]
fn unsubscribe_payload_matches_kucoin_schema() {
    let value: Value = serde_json::from_str(&unsubscribe_payload("ETHUSDTM")).unwrap();
    assert_eq!(value["type"], "unsubscribe");
    assert_eq!(value["topic"], "/contractMarket/level2Depth50:ETHUSDTM");
}

// bullet_response_builds_token_url moved to kucoin_ws_session module.

#[test]
fn parse_depth50_snapshot_extracts_book() {
    let raw = r#"{
      "type":"message",
      "topic":"/contractMarket/level2Depth50:XBTUSDTM",
      "sn":1700000000001,
      "subject":"level2",
      "data":{
        "bids":[["50000.0",1.5],["49999.0",2.0]],
        "asks":[["50001.0",1.2],["50002.0",3.0]],
        "sequence":1700000000001,
        "timestamp":1700000000000
      }
    }"#;
    let parsed = parse_depth_snapshot(raw).unwrap();
    assert_eq!(parsed.stream_symbol, "XBTUSDTM");
    assert_eq!(parsed.book.symbol, "BTC");
    assert_eq!(parsed.book.exchange, "kucoin");
    assert_eq!(parsed.book.timestamp, 1_700_000_000_000);
    assert_eq!(parsed.book.bids, vec![[50000.0, 1.5], [49999.0, 2.0]]);
    assert_eq!(parsed.book.asks, vec![[50001.0, 1.2], [50002.0, 3.0]]);
    assert_eq!(parsed.sequence, 1_700_000_000_001);
}

#[test]
fn parse_ignores_ack_and_other_topics() {
    assert!(parse_depth_snapshot(r#"{"type":"welcome"}"#).is_none());
    assert!(
        parse_depth_snapshot(r#"{"topic":"/contractMarket/tickerV2:XBTUSDTM","data":{}}"#)
            .is_none()
    );
}
