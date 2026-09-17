use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn stream_symbol_uses_the_official_futures_contract_identity() {
    assert_eq!(normalized_to_kucoin("BTC"), "XBTUSDTM");
    assert_eq!(normalized_to_kucoin("SOL-USDT"), "SOLUSDTM");
    assert_eq!(kucoin_to_normalized("XBTUSDTM"), "BTC");
}

#[test]
fn command_payload_targets_pro_channel() {
    let payload = command_payload("SUBSCRIBE", CHANNEL_MARK_PRICE, "XBTUSDTM");
    let value: Value = serde_json::from_str(&payload).expect("payload parses");
    assert_eq!(value["action"], "SUBSCRIBE");
    assert_eq!(value["channel"], "mark-price");
    assert_eq!(value["symbol"], "XBTUSDTM");
    assert!(value["id"].as_str().is_some_and(|id| id.len() <= 40));
}

#[test]
fn unsubscribe_payload_keeps_the_same_symbol_identity() {
    let payload = command_payload("UNSUBSCRIBE", CHANNEL_FUNDING_FEE, "ETHUSDTM");
    let value: Value = serde_json::from_str(&payload).expect("payload parses");
    assert_eq!(value["action"], "UNSUBSCRIBE");
    assert_eq!(value["channel"], "funding-fee");
    assert_eq!(value["symbol"], "ETHUSDTM");
}

#[test]
fn subscription_commands_skip_duplicate_or_obsolete_work() {
    let unsent = SubscriptionState {
        last_touched_ms: 1,
        sent_on_current_connection: false,
    };
    let sent = SubscriptionState {
        sent_on_current_connection: true,
        ..unsent
    };

    assert!(!command_is_obsolete("SUBSCRIBE", Some(unsent)));
    assert!(command_is_obsolete("SUBSCRIBE", Some(sent)));
    assert!(command_is_obsolete("SUBSCRIBE", None));
    assert!(command_is_obsolete("UNSUBSCRIBE", Some(unsent)));
    assert!(!command_is_obsolete("UNSUBSCRIBE", None));
    assert!(command_is_obsolete("unknown", Some(unsent)));
}

#[test]
fn pro_mark_price_fixture_preserves_mark_index_and_open_interest() {
    let raw = include_str!("../../fixtures/kucoin/ws_pro_mark_price_xbtusdtm.json");
    let parsed = parse_mark_index_update(raw).expect("mark-price parses");
    assert_eq!(parsed.kucoin_symbol, "XBTUSDTM");
    assert_eq!(parsed.cached.mark_price, 62_876.5);
    assert_eq!(parsed.cached.index_price, Some(62_887.48));
    assert_eq!(parsed.cached.open_interest, Some(24_488_739.0));
    assert_eq!(parsed.cached.data_timestamp_ms, 1_785_518_219_000);
}

#[test]
fn pro_funding_fixture_preserves_rate_settlement_and_interval() {
    let raw = include_str!("../../fixtures/kucoin/ws_pro_funding_fee_xbtusdtm.json");
    let parsed = parse_funding_update(raw).expect("funding-fee parses");
    assert_eq!(parsed.kucoin_symbol, "XBTUSDTM");
    assert_eq!(parsed.cached.rate, 0.0001);
    assert_eq!(parsed.cached.interval_hours, 8);
    assert_eq!(parsed.cached.next_funding_time_ms, 1_785_542_400_000);
    assert_eq!(parsed.cached.data_timestamp_ms, 1_785_518_221_111);
}

#[test]
fn funding_parser_rejects_non_hour_granularity_and_wrong_channel() {
    let non_hour = r#"{
        "T":"funding-fee","P":1785518221111213021,
        "d":{"s":"XBTUSDTM","fr":"0.0001","nt":1785542400000,"gl":1000}
    }"#;
    assert!(parse_funding_update(non_hour).is_none());

    let mark = include_str!("../../fixtures/kucoin/ws_pro_mark_price_xbtusdtm.json");
    assert!(parse_funding_update(mark).is_none());
}

#[test]
fn mark_parser_rejects_missing_provider_timestamp() {
    let raw = r#"{
        "T":"mark-price",
        "d":{"s":"XBTUSDTM","mp":"1","ip":"1","oi":"0"}
    }"#;
    assert!(parse_mark_index_update(raw).is_none());
}

#[test]
fn freshness_rejects_future_and_expired_cache_rows() {
    assert!(is_fresh(10_000, 15_000, 5_000));
    assert!(!is_fresh(10_000, 15_001, 5_000));
    assert!(!is_fresh(15_001, 15_000, 5_000));
    assert!(!is_fresh(0, 15_000, 5_000));
}
