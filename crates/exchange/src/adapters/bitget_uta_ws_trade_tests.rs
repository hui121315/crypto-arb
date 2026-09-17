use super::*;
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, MarginMode, OrderSide, OrderSource, OrderType, TimeInForce};

#[test]
fn login_request_matches_v3_login_schema() {
    let request = login_request(cfg());
    let arg = &request.args[0];
    assert_eq!(request.op, "login");
    assert_eq!(arg.api_key, "k");
    assert_eq!(arg.passphrase, "p");
    assert_eq!(arg.timestamp.len(), 13);
    // base64 of 32-byte HMAC-SHA256 is 44 chars including padding.
    assert_eq!(arg.sign.len(), 44);
}

#[test]
fn trade_session_uses_official_text_ping_heartbeat() {
    assert_eq!(
        bitget_trade_heartbeat(),
        WsHeartbeat::Text("ping".to_owned())
    );
    assert!(bitget_trade_heartbeat_response("pong"));
    assert!(bitget_trade_heartbeat_response(" pong\n"));
    assert!(!bitget_trade_heartbeat_response(r#"{"event":"trade"}"#));
}

#[test]
fn place_request_matches_v3_envelope_shape() {
    let request = order_place_request(
        &intent(OrderType::Limit),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Crossed,
    )
    .expect("place request serializes");
    assert_eq!(request.op, "trade");
    assert_eq!(request.id, "i1");
    // V3 lifts category + topic to the envelope; lowercase per ccxt-bridge.
    assert_eq!(request.category, "usdt-futures");
    assert_eq!(request.topic, "place-order");
    // The per-order body is a single JSON object in args[0]; category is
    // already at the top level and must not repeat here.
    let params = request.args[0].as_object().expect("args[0] is object");
    assert_place_params_match_v3_limit(params);
}

#[test]
fn spot_place_and_cancel_use_uta_spot_envelope() {
    let compiled = CompiledBitgetOrder {
        category: BitgetUtaCategory::Spot,
        native_symbol: "BTCUSDT".to_owned(),
        quantity: 0.01,
        pos_side: None,
        reduce_only: false,
    };
    let place = order_place_request(
        &intent(OrderType::Limit),
        &compiled,
        BitgetMarginMode::Crossed,
    )
    .expect("spot place request");
    let params = place.args[0].as_object().expect("spot args");
    assert_eq!(place.category, "spot");
    assert_eq!(params["symbol"], "BTCUSDT");
    assert!(!params.contains_key("marginMode"));
    assert!(!params.contains_key("posSide"));
    assert!(!params.contains_key("reduceOnly"));

    let cancel = CancelOrderRequest {
        exchange: "bitget".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };
    let cancel = order_cancel_request_for(&cancel, BitgetUtaCategory::Spot, "BTCUSDT".to_owned())
        .expect("spot cancel request");
    assert_eq!(cancel.category, "spot");
    assert_eq!(cancel.topic, TOPIC_CANCEL_ORDER);
}

fn assert_place_params_match_v3_limit(params: &serde_json::Map<String, serde_json::Value>) {
    assert!(!params.contains_key("category"));
    assert_eq!(params["symbol"], "BTCUSDT");
    assert_eq!(params["marginMode"], "crossed");
    assert!(!params.contains_key("marginCoin"));
    assert_eq!(params["side"], "buy");
    assert_eq!(params["orderType"], "limit");
    assert_eq!(params["qty"], "0.01");
    assert_eq!(params["price"], "50000");
    assert_eq!(params["timeInForce"], "ioc");
    assert!(!params.contains_key("force"));
    assert_eq!(params["clientOid"], "cid-1");
}

#[test]
fn place_request_translates_reduce_only_for_ws_schema() {
    let mut order = intent(OrderType::Market);
    order.reduce_only = true;
    let request = order_place_request(&order, "BTCUSDT".to_owned(), BitgetMarginMode::Crossed)
        .expect("place request serializes");
    let params = request.args[0].as_object().expect("args[0] is object");

    assert_eq!(params["orderType"], "market");
    assert_eq!(params["reduceOnly"], "YES");
    assert!(!params.contains_key("timeInForce"));
}

#[test]
fn cancel_request_envelope_carries_client_oid_only() {
    let request = CancelOrderRequest {
        exchange: "bitget".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };
    let ws = order_cancel_request(&request, "BTCUSDT".to_owned()).expect("cancel request");
    assert_eq!(ws.op, "trade");
    assert_eq!(ws.id, "i1");
    assert_eq!(ws.topic, "cancel-order");
    assert_eq!(ws.category, "usdt-futures");
    let params = ws.args[0].as_object().expect("args[0] is object");
    // category is in the envelope; official UTA WS cancel args are orderId/clientOid only.
    assert!(!params.contains_key("category"));
    assert!(!params.contains_key("symbol"));
    assert_eq!(params["clientOid"], "cid-1");
    assert!(!params.contains_key("orderId"));
}

#[test]
fn parses_success_response_into_ack_row() {
    let response = parse_trade_response(
        r#"{"event":"trade","id":"i1","topic":"place-order","code":"0","msg":"","args":[{"orderId":"7","clientOid":"cid-1"}]}"#,
    )
    .expect("trade response parses");
    let request = order_place_request(
        &intent(OrderType::Limit),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Crossed,
    )
    .expect("place request");
    let matcher = request_matcher(&request);
    assert!(response.matches_parts(&matcher));
    let row = response.into_result().expect("ack row parses");
    assert_eq!(row.order_id, "7");
    assert_eq!(row.client_oid, "cid-1");
}

#[test]
fn bitget_uta_ws_place_order_ack_parses_official_fixture() {
    let response = parse_trade_response(include_str!(
        "../../fixtures/bitget/uta_ws_place_order_ack.json"
    ))
    .expect("official Bitget UTA WS place-order fixture parses");
    let request = order_place_request(
        &intent(OrderType::Limit),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Crossed,
    )
    .expect("place request");
    let matcher = request_matcher(&request);

    assert!(response.matches_parts(&matcher));
    let row = response.into_result().expect("ack row parses");
    assert_eq!(row.order_id, "1217143186968068096");
    assert_eq!(row.client_oid, "cid-1");
}

#[test]
fn bitget_uta_ws_cancel_order_ack_parses_official_fixture() {
    let request = CancelOrderRequest {
        exchange: "bitget".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };
    let ws = order_cancel_request(&request, "BTCUSDT".to_owned()).expect("cancel request");
    let matcher = request_matcher(&ws);
    let response = parse_trade_response(include_str!(
        "../../fixtures/bitget/uta_ws_cancel_order_ack.json"
    ))
    .expect("official Bitget UTA WS cancel-order fixture parses");

    assert!(response.matches_parts(&matcher));
    let row = response.into_result().expect("ack row parses");
    assert_eq!(row.order_id, "111111111111111111");
    assert_eq!(row.client_oid, "cid-1");
}

#[test]
fn parses_legacy_data_response_into_ack_row() {
    let response = parse_trade_response(
        r#"{"event":"trade","id":"i1","topic":"place-order","code":"0","msg":"","data":[{"orderId":"8","clientOid":"cid-8"}]}"#,
    )
    .expect("trade response parses");
    let row = response.into_result().expect("ack row parses");

    assert_eq!(row.order_id, "8");
    assert_eq!(row.client_oid, "cid-8");
}

#[test]
fn parses_numeric_zero_code_as_success() {
    let response = parse_trade_response(
        r#"{"op":"trade","id":"i1","topic":"place-order","code":0,"args":[{"orderId":"9","clientOid":"cid-9"}]}"#,
    )
    .expect("numeric-code response parses");
    let row = response.into_result().expect("ack row parses");
    assert_eq!(row.order_id, "9");
}

#[test]
fn surfaces_api_error_on_non_success_code() {
    let response = parse_trade_response(
        r#"{"event":"trade","id":"i1","topic":"place-order","code":"43012","msg":"Insufficient balance"}"#,
    )
    .expect("error response parses");
    let err = response.into_result().unwrap_err();
    match err {
        ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "43012");
            assert_eq!(message, "Insufficient balance");
        }
        other => panic!("expected API error, got {other:?}"),
    }
}

#[test]
fn matcher_accepts_official_error_event_without_topic() {
    let request = order_place_request(
        &intent(OrderType::Limit),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Crossed,
    )
    .expect("place request");
    let matcher = request_matcher(&request);
    let response = parse_trade_response(
        r#"{"event":"error","id":"i1","code":"40010","msg":"Request timed out"}"#,
    )
    .expect("official error response parses");

    assert!(response.matches_parts(&matcher));
    assert!(matches!(
        response.into_result(),
        Err(ExchangeError::Api { code, .. }) if code == "40010"
    ));
}

#[test]
fn matcher_rejects_topic_or_id_mismatch() {
    let request = order_place_request(
        &intent(OrderType::Limit),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Crossed,
    )
    .expect("place request");
    let matcher = request_matcher(&request);
    let foreign_topic = parse_trade_response(
        r#"{"event":"trade","id":"i1","topic":"cancel-order","code":"0","data":[{}]}"#,
    )
    .expect("foreign topic parses");
    assert!(!foreign_topic.matches_parts(&matcher));
    let foreign_id = parse_trade_response(
        r#"{"event":"trade","id":"other","topic":"place-order","code":"0","data":[{}]}"#,
    )
    .expect("foreign id parses");
    assert!(!foreign_id.matches_parts(&matcher));
}

#[test]
fn long_request_ids_are_hashed_to_under_forty_chars() {
    let long = "a".repeat(64);
    let hashed = checked_request_id(&long).expect("hash succeeds");
    assert!(hashed.starts_with("bg-"));
    assert!(hashed.len() <= MAX_REQUEST_ID_LEN);
}

#[test]
fn blank_request_id_is_rejected() {
    let err = checked_request_id("   ").unwrap_err();
    match err {
        ExchangeError::Api { code, .. } => assert_eq!(code, "validation"),
        other => panic!("expected validation error, got {other:?}"),
    }
}

fn cfg() -> WsTradeConfig<'static> {
    WsTradeConfig {
        url: "wss://example.test",
        api_key: "k",
        api_secret: "s",
        passphrase: "p",
        timeout_secs: 1,
    }
}

fn intent(order_type: OrderType) -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "bitget".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
