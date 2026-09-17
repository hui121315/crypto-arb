use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn subscription_uses_classic_level50_topic() {
    let value: Value = serde_json::from_str(&command_payload("subscribe", "BTC-USDT")).unwrap();
    assert_eq!(value["type"], "subscribe");
    assert_eq!(value["topic"], "/spotMarket/level2Depth50:BTC-USDT");
    assert_eq!(value["response"], true);
}

#[test]
fn classic_level50_snapshot_parses_official_shape() {
    let raw = r#"{
      "topic":"/spotMarket/level2Depth50:BTC-USDT",
      "type":"message","subject":"level2",
      "data":{"asks":[["95964.3","0.08168874"]],
      "bids":[["95964.2","1.35483359"]],"timestamp":1733124805073}
    }"#;
    let (symbol, book) = parse_snapshot(raw).expect("classic snapshot");
    assert_eq!(symbol, "BTC-USDT");
    assert_eq!(book.symbol, "BTC/USDT");
    assert_eq!(book.bids, vec![[95_964.2, 1.354_833_59]]);
    assert_eq!(book.asks, vec![[95_964.3, 0.081_688_74]]);
    assert_eq!(book.timestamp, 1_733_124_805_073);
}

#[test]
fn parser_rejects_beta_pro_shape_and_one_sided_book() {
    assert!(parse_snapshot(r#"{"T":"obu.SPOT","d":{}}"#).is_none());
    assert!(parse_snapshot(
        r#"{"topic":"/spotMarket/level2Depth50:BTC-USDT","data":{"bids":[["1","1"]],"asks":[]}}"#
    )
    .is_none());
}
