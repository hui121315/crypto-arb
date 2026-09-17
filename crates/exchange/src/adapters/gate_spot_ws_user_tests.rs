use super::*;

#[test]
fn subscribes_to_all_spot_orders_with_auth() {
    let payload = subscribe_orders_payload_at("key", "secret", 1_780_000_000).expect("payload");
    let value: Value = serde_json::from_str(&payload).expect("json");
    assert_eq!(value["channel"], GATE_SPOT_ORDER_CHANNEL);
    assert_eq!(value["payload"][0], "!all");
    assert_eq!(value["auth"]["KEY"], "key");
    assert_eq!(value["auth"]["SIGN"].as_str().map(str::len), Some(128));
}

#[test]
fn parses_filled_spot_order_in_base_units() {
    let text = r#"{
        "time_ms":1780000000100,"channel":"spot.orders","event":"update",
        "result":[{
            "id":"399123456","text":"t-cid-spot-1","create_time":"1780000000",
            "create_time_ms":"1780000000000","update_time_ms":"1780000000123",
            "currency_pair":"SOL_USDT","type":"limit",
            "side":"sell","amount":"0.4","price":"150","time_in_force":"gtc",
            "left":"0","avg_deal_price":"150.1","fee":"0.06","event":"finish",
            "finish_as":"filled"
        }]
    }"#;
    let GateSpotUserMessage::Orders(rows) = parse_message(text).expect("event") else {
        panic!("expected orders");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].live_state, LiveOrderState::Filled);
    assert_eq!(rows[0].order.symbol, "SOL");
    assert_eq!(rows[0].order.filled_quantity, 0.4);
    assert_eq!(rows[0].received_at_ms, 1_780_000_000_123);
}

#[test]
fn parses_subscription_ack() {
    let text = r#"{"id":1,"channel":"spot.orders","event":"subscribe","error":null,"result":{"status":"success"}}"#;
    assert!(matches!(
        parse_message(text).expect("ack"),
        GateSpotUserMessage::Control(GateSpotUserControl::Acknowledged { .. })
    ));
}
