use super::*;

#[test]
fn subscription_targets_official_swap_instruments_channel() {
    let payload: serde_json::Value =
        serde_json::from_str(&subscription_payload()).expect("subscription json");

    assert_eq!(payload["op"], "subscribe");
    assert_eq!(payload["args"][0]["channel"], "instruments");
    assert_eq!(payload["args"][0]["instType"], "SWAP");
}

#[test]
fn official_swap_update_parses_executable_rule() {
    let fixture = include_str!("../../fixtures/okx/ws_instruments_swap_update.json");
    let rules = parse_update(fixture);

    assert_eq!(rules.len(), 1);
    assert_eq!(rules[0].inst_id, "BTC-USDT-SWAP");
    assert_eq!(rules[0].ws_inst_id_code().expect("instIdCode"), 123_456);
}

#[test]
fn parser_rejects_non_swap_or_ack_frames() {
    let spot = include_str!("../../fixtures/okx/ws_instruments_swap_update.json")
        .replace("\"SWAP\"", "\"SPOT\"");

    assert!(parse_update(&spot).is_empty());
    assert!(parse_update(
        r#"{"event":"subscribe","arg":{"channel":"instruments","instType":"SWAP"}}"#
    )
    .is_empty());
}
