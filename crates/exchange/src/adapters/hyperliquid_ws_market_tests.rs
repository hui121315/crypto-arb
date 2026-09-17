use super::*;
use pretty_assertions::assert_eq;

#[test]
fn subscribe_payload_matches_hl_schema() {
    let value: Value = serde_json::from_str(&subscribe_payload("BTC")).unwrap();
    assert_eq!(value["method"], "subscribe");
    assert_eq!(value["subscription"]["type"], "l2Book");
    assert_eq!(value["subscription"]["coin"], "BTC");
}

#[test]
fn unsubscribe_payload_matches_hl_schema() {
    let value: Value = serde_json::from_str(&unsubscribe_payload("xyz:SNDK")).unwrap();
    assert_eq!(value["method"], "unsubscribe");
    assert_eq!(value["subscription"]["coin"], "xyz:SNDK");
}

#[test]
fn clean_coin_strips_dex_prefix() {
    assert_eq!(clean_coin("BTC"), "BTC");
    assert_eq!(clean_coin("xyz:SNDK"), "SNDK");
    assert_eq!(clean_coin("cash:USDC"), "USDC");
}

#[test]
fn parse_l2_book_extracts_levels() {
    let raw = r#"{
        "channel":"l2Book",
        "data":{
            "coin":"BTC",
            "levels":[
                [{"px":"50000.5","sz":"1.5"},{"px":"50000.0","sz":"2.0"}],
                [{"px":"50001.0","sz":"1.2"},{"px":"50001.5","sz":"3.0"}]
            ],
            "time":1700000000000
        }
    }"#;
    let parsed = parse_l2_book(raw).expect("parsed");
    assert_eq!(parsed.coin, "BTC");
    assert_eq!(parsed.time, 1_700_000_000_000);
    assert_eq!(parsed.bids.len(), 2);
    assert_eq!(parsed.bids[0], [50_000.5, 1.5]);
    assert_eq!(parsed.asks[0], [50_001.0, 1.2]);
}

#[test]
fn parse_l2_book_ignores_non_l2book_channel() {
    let raw = r#"{"channel":"trades","data":{"coin":"BTC"}}"#;
    assert!(parse_l2_book(raw).is_none());
}

#[test]
fn parse_l2_book_skips_invalid_levels() {
    let raw = r#"{
        "channel":"l2Book",
        "data":{
            "coin":"ETH",
            "levels":[
                [{"px":"abc","sz":"1.0"},{"px":"3000","sz":"2.0"}],
                [{"px":"3001","sz":"xyz"}]
            ],
            "time":1
        }
    }"#;
    let parsed = parse_l2_book(raw).expect("parsed");
    assert_eq!(parsed.bids, vec![[3000.0, 2.0]]);
    assert!(parsed.asks.is_empty());
}

#[test]
fn parse_l2_book_handles_builder_dex_coin() {
    let raw = r#"{
        "channel":"l2Book",
        "data":{
            "coin":"xyz:SNDK",
            "levels":[
                [{"px":"10","sz":"5"}],
                [{"px":"11","sz":"5"}]
            ],
            "time":42
        }
    }"#;
    let parsed = parse_l2_book(raw).expect("parsed");
    assert_eq!(parsed.coin, "xyz:SNDK");
    assert_eq!(clean_coin(&parsed.coin), "SNDK");
}

#[test]
fn parse_l2_book_returns_none_for_invalid_json() {
    assert!(parse_l2_book("not json").is_none());
    assert!(parse_l2_book("{\"channel\":\"l2Book\"}").is_none());
}
