use super::*;
use pretty_assertions::assert_eq;
use serde_json::json;
use shared_types::{ExecutionMode, MarginMode, OrderSide, OrderSource, OrderType, TimeInForce};

#[test]
fn login_request_matches_gate_ws_schema() {
    let request = login_request(cfg());

    assert_eq!(request.channel, "futures.login");
    assert_eq!(request.event, "api");
    assert_eq!(request.payload.api_key.as_deref(), Some("k"));
    assert!(request.payload.req_param.is_none());
    let encoded = serde_json::to_value(&request).expect("serialize gate login");
    assert!(encoded["payload"].get("req_param").is_none());
    let timestamp = request.payload.timestamp.as_deref().unwrap_or_default();
    assert_eq!(
        request.payload.signature.as_deref(),
        Some(sign::ws_api_sign(b"s", EVENT_API, CHANNEL_LOGIN, "", timestamp).as_str())
    );
}

#[test]
fn place_request_matches_gate_ws_schema() {
    let request = trade_request(
        CHANNEL_ORDER_PLACE,
        "i1",
        json!({
            "contract": "BTC_USDT",
            "size": 100,
            "price": "50000",
            "tif": "gtc",
            "text": "t-cid-1"
        }),
    );

    assert_eq!(request.channel, "futures.order_place");
    assert_eq!(request.event, "api");
    assert_eq!(request.payload.req_id, "i1");
    let params = request.payload.req_param.as_ref().expect("order params");
    assert_eq!(params["contract"], "BTC_USDT");
    assert_eq!(params["size"], 100);
    assert!(request.payload.api_key.is_none());
}

#[test]
fn cancel_request_matches_gate_ws_schema() {
    let request = trade_request(
        CHANNEL_ORDER_CANCEL,
        "request-id-5",
        json!({
            "order_id": "74046514"
        }),
    );

    assert_eq!(request.channel, "futures.order_cancel");
    assert_eq!(request.event, "api");
    assert_eq!(request.payload.req_id, "request-id-5");
    let params = request.payload.req_param.as_ref().expect("cancel params");
    assert_eq!(params["order_id"], "74046514");
    assert!(
        params.get("contract").is_none(),
        "official WS cancel req_param contains order_id, not contract"
    );
    assert!(request.payload.api_key.is_none());
}

#[test]
fn order_status_request_matches_gate_ws_schema() {
    let request = trade_request(
        CHANNEL_ORDER_STATUS,
        "request-id-2",
        json!({ "order_id": "74046543" }),
    );

    assert_eq!(request.channel, "futures.order_status");
    assert_eq!(request.event, "api");
    assert_eq!(
        request.payload.req_param.as_ref().expect("status params")["order_id"],
        "74046543"
    );
}

#[test]
fn order_list_request_matches_gate_ws_schema() {
    let request = trade_request(
        CHANNEL_ORDER_LIST,
        "request-id-3",
        json!({ "contract": "BTC_USDT", "status": "open" }),
    );

    assert_eq!(request.channel, "futures.order_list");
    assert_eq!(request.event, "api");
    let params = request
        .payload
        .req_param
        .as_ref()
        .expect("order-list params");
    assert_eq!(params["contract"], "BTC_USDT");
    assert_eq!(params["status"], "open");
}

#[test]
fn generated_request_ids_fit_gate_limit_and_are_unique() {
    let first = next_request_id("order-list");
    let second = next_request_id("order-list");

    assert!(
        first.len() < 32,
        "gate requires request ids shorter than 32"
    );
    assert!(
        second.len() < 32,
        "gate requires request ids shorter than 32"
    );
    assert_ne!(first, second);
}

/// 修复 P2 9.9：ack=true + errs 是 final error，不该被当作中间 ack 跳过。
#[test]
fn ack_true_with_errs_is_final() {
    let response = parse_response(
        r#"{"request_id":"i1","ack":true,"data":{"errs":{"label":"INVALID_PARAM","message":"bad price"}}}"#,
    )
    .expect("parse");
    assert!(
        response.is_final_for("i1"),
        "ack=true+errs must be final to avoid hang"
    );
    let err = response.into_result().unwrap_err();
    assert!(matches!(err, ExchangeError::Api { .. }));
}

/// 修复 P2 9.9：ack=true + header.status="400" 也是 final error。
#[test]
fn ack_true_with_non_200_header_status_is_final() {
    let response = parse_response(
        r#"{"request_id":"i1","ack":true,"header":{"status":"400","channel":"futures.order_place"}}"#,
    )
    .expect("parse");
    assert!(response.is_final_for("i1"));
    let err = response.into_result().unwrap_err();
    match err {
        ExchangeError::Api { code, .. } => assert_eq!(code, "400"),
        other => panic!("expected Api error, got {other:?}"),
    }
}

/// 修复 P2 9.9：`req_id` 不匹配的消息（如订阅推送）不应被当作 final。
#[test]
fn unrelated_message_is_not_final() {
    let response =
        parse_response(r#"{"request_id":"other","ack":false,"data":{"result":{"id":99}}}"#)
            .expect("parse");
    assert!(!response.is_final_for("i1"));
}

/// 修复 P2 9.9：纯 ack=true 中间消息（无 errs / status="200" / 缺 header）应被跳过。
#[test]
fn intermediate_ack_message_is_not_final() {
    let response = parse_response(r#"{"request_id":"i1","ack":true,"header":{"status":"200"}}"#)
        .expect("parse");
    assert!(
        !response.is_final_for("i1"),
        "intermediate ack must not be final"
    );
}

#[test]
fn parses_success_response_to_ack_row() {
    let response =
        match parse_response(r#"{"request_id":"i1","ack":false,"data":{"result":{"id":7}}}"#) {
            Ok(response) => response,
            Err(error) => panic!("gate ws response: {error}"),
        };
    assert!(response.is_final_for("i1"));
    let row = match response.into_result() {
        Ok(row) => row,
        Err(error) => panic!("gate ws row: {error}"),
    };
    let ack = ack_from_result(
        "i1".into(),
        "cid-1".into(),
        "t-cid-1".into(),
        &row,
        LiveOrderState::Accepted,
        None,
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("7"));
    assert_eq!(ack.client_order_id, "cid-1");
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("t-cid-1")
    );
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[test]
fn gate_ws_order_place_ack_response_parses_official_fixture() {
    let response = parse_fixture(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/gate/ws_futures_order_place_success.json"
        )),
        "request-id-1",
    );
    let row = response.into_result().expect("place row");

    assert_eq!(row.order_id, Some(74046514));
}

#[test]
fn gate_ws_final_fok_response_projects_immediate_fill() {
    let response = parse_response(
        r#"{
            "request_id":"i1",
            "ack":false,
            "header":{"status":"200","channel":"futures.order_place"},
            "data":{"result":{
                "id":36028834089796976,
                "contract":"BTC_USDT",
                "status":"finished",
                "finish_as":"filled",
                "size":"1",
                "left":"0",
                "price":"62922",
                "fill_price":"62909.4",
                "create_time":1681195121.754,
                "text":"t-cid-1",
                "tif":"fok",
                "is_reduce_only":false
            }}
        }"#,
    )
    .expect("live final response parses");
    let row: OpenOrderItem = response.into_typed_result().expect("order result");
    let ack = place_ack_from_order(&intent(), &row).expect("terminal ack projects");

    assert_eq!(ack.exchange_order_id.as_deref(), Some("36028834089796976"));
    assert_eq!(ack.state, LiveOrderState::Filled);
    assert_eq!(ack.filled_quantity, Some(0.0001));
    assert_eq!(ack.filled_price, Some(62_909.4));
}

#[test]
fn gate_ws_order_cancel_ack_response_parses_official_fixture() {
    let response = parse_fixture(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/gate/ws_futures_order_cancel_success.json"
        )),
        "request-id-5",
    );
    let row = response.into_result().expect("cancel row");

    assert_eq!(row.order_id, Some(74046543));
}

#[test]
fn gate_ws_cancel_final_response_projects_cancelled() {
    let response = parse_fixture(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/gate/ws_futures_order_cancel_success.json"
        )),
        "request-id-5",
    );
    let row: OpenOrderItem = response.into_typed_result().expect("cancel order row");
    let request = CancelOrderRequest {
        exchange: "gate".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: Some("74046543".into()),
        client_order_id: "my-custom-id".into(),
    };
    let ack = cancel_ack_from_order(&request, &row).expect("cancel terminal projects");

    assert_eq!(ack.state, LiveOrderState::Cancelled);
    assert_eq!(ack.exchange_order_id.as_deref(), Some("74046543"));
}

#[test]
fn gate_ws_order_status_response_parses_official_fixture() {
    let response = parse_fixture(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/gate/ws_futures_order_status.json"
        )),
        "request-id-2",
    );
    let row: OpenOrderItem = response.into_typed_result().expect("order row");
    let order =
        crate::adapters::gate_private_data::parse_open_order(&row).expect("projected order");

    assert_eq!(order.order_id, "74046543");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.client_order_id.as_deref(), Some("t-my-custom-id"));
}

#[test]
fn gate_ws_order_list_response_parses_official_fixture() {
    let response = parse_fixture(
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/fixtures/gate/ws_futures_order_list.json"
        )),
        "request-id-3",
    );
    let rows: Vec<OpenOrderItem> = response.into_typed_result().expect("order rows");

    assert_eq!(rows.len(), 1);
    let orders =
        crate::adapters::gate_private_data::parse_open_orders(&rows).expect("projected orders");
    assert_eq!(orders[0].order_id, "74046543");
}

fn parse_fixture(text: &str, req_id: &str) -> WsResponse {
    let response = parse_response(text).expect("official fixture parses");
    assert!(response.is_final_for(req_id), "{req_id} must be final");
    response
}

fn cfg() -> WsTradeConfig<'static> {
    WsTradeConfig {
        url: "wss://example.test",
        api_key: "k",
        api_secret: "s",
        timeout_secs: 1,
        // 修复 P2 5.9：测试默认 offset=0（与历史行为一致）。
        time_offset_secs: 0,
    }
}

fn intent() -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "gate".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.0001,
        price: Some(62_922.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Fok,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
