use super::*;

#[test]
fn builds_signed_subscription() {
    let payload =
        subscription_payload_at("key", "secret", "sub-1", 1_780_000_000_000).expect("payload");
    let value: Value = serde_json::from_str(&payload).expect("json");
    assert_eq!(value["method"], "userDataStream.subscribe.signature");
    assert_eq!(value["params"]["apiKey"], "key");
    assert_eq!(value["params"]["timestamp"], 1_780_000_000_000_i64);
    assert_eq!(
        value["params"]["signature"].as_str().map(str::len),
        Some(64)
    );
}

#[test]
fn parses_wrapped_spot_fill_terminal() {
    let text = r#"{
        "subscriptionId":0,
        "event":{
            "e":"executionReport","E":1780000000100,"T":1780000000090,
            "s":"SOLUSDT","c":"cid-spot-1","S":"BUY","o":"MARKET","f":"GTC",
            "q":"0.1","p":"0","x":"TRADE","X":"FILLED","r":"NONE",
            "i":12345,"l":"0.1","z":"0.1","L":"150","Z":"15",
            "n":"0.0001","N":"SOL","t":99,"O":1780000000000
        }
    }"#;
    let BinanceSpotUserMessage::Event(BinanceUserEvent::Order(update)) =
        parse_message(text).expect("event")
    else {
        panic!("expected order event");
    };
    assert_eq!(update.client_order_id, "cid-spot-1");
    assert_eq!(update.live_state, LiveOrderState::Filled);
    assert_eq!(update.order.symbol, "SOL");
    assert_eq!(update.order.filled_quantity, 0.1);
    assert_eq!(update.order.filled_price, 150.0);
    assert_eq!(update.trade_id, Some(99));
}

#[test]
fn parses_subscription_rejection() {
    let text = r#"{"id":"sub-1","status":401,"error":{"code":-2015,"msg":"bad key"}}"#;
    let BinanceSpotUserMessage::Control(BinanceSpotUserControl::Rejected {
        authentication_failed,
        ..
    }) = parse_message(text).expect("control")
    else {
        panic!("expected rejection");
    };
    assert!(authentication_failed);
}
