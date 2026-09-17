use super::*;
use serde_json::Value;

#[test]
fn builds_private_order_v2_subscription() {
    let payload = subscribe_orders_payload("spot-orders").expect("payload");
    let value: Value = serde_json::from_str(&payload).expect("json");
    assert_eq!(value["topic"], KUCOIN_SPOT_ORDER_TOPIC);
    assert_eq!(value["privateChannel"], true);
}

#[test]
fn parses_spot_match_with_fill_identity() {
    let text = r#"{
        "topic":"/spotMarket/tradeOrdersV2","type":"message","subject":"orderChange",
        "data":{
            "clientOid":"cid-spot-1","orderId":"6720da3fa30a360007f5f832",
            "orderTime":1780000000000,"orderType":"market","originSize":"0.1",
            "side":"buy","status":"match","symbol":"SOL-USDT",
            "ts":1780000000100000000,"type":"match","filledSize":"0.1",
            "matchPrice":"150","matchSize":"0.1","tradeId":"trade-1",
            "liquidity":"taker","feeType":"takerFee"
        }
    }"#;
    let KucoinSpotUserMessage::Event(KucoinUserEvent::Order(update)) =
        parse_message(text).expect("event")
    else {
        panic!("expected order");
    };
    assert_eq!(update.order.symbol, "SOL");
    assert_eq!(update.live_state, LiveOrderState::PartiallyFilled);
    assert_eq!(update.fill.as_ref().map(|fill| fill.quantity), Some(0.1));
    assert_eq!(
        update
            .fill
            .as_ref()
            .map(|fill| fill.venue_event_id.as_str()),
        Some("kucoin_spot_trade:6720da3fa30a360007f5f832:trade-1")
    );
}

#[test]
fn parses_filled_terminal() {
    let text = r#"{
        "topic":"/spotMarket/tradeOrdersV2","type":"message","subject":"orderChange",
        "data":{
            "clientOid":"cid-spot-1","orderId":"order-1","orderTime":1780000000000,
            "orderType":"limit","originSize":"0.1","side":"sell","status":"done",
            "symbol":"SOL-USDT","ts":1780000000100000000,"type":"filled",
            "filledSize":"0.1","price":"150"
        }
    }"#;
    let KucoinSpotUserMessage::Event(KucoinUserEvent::Order(update)) =
        parse_message(text).expect("event")
    else {
        panic!("expected order");
    };
    assert!(update.terminal);
    assert_eq!(update.live_state, LiveOrderState::Filled);
}
