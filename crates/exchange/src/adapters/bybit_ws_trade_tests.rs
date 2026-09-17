use super::*;
use crate::error::ExchangeError;
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, OrderSide, OrderSource, OrderType, TimeInForce};

#[test]
fn auth_request_matches_bybit_ws_schema() {
    let request = auth_request(cfg());

    assert_eq!(request.op, "auth");
    assert_eq!(request.args[0], "k");
    assert_eq!(request.args[1].len(), 13);
    assert_eq!(request.args[2].len(), 64);
}

#[test]
fn trade_session_uses_official_json_ping_heartbeat() {
    assert_eq!(
        bybit_trade_heartbeat(),
        WsHeartbeat::Text(r#"{"op":"ping"}"#.to_owned())
    );
    assert!(bybit_trade_heartbeat_response(
        r#"{"success":true,"ret_msg":"pong","op":"pong"}"#
    ));
    assert!(bybit_trade_heartbeat_response(
        r#"{"success":true,"ret_msg":"pong","op":"ping"}"#
    ));
    assert!(!bybit_trade_heartbeat_response(
        r#"{"op":"order.create","retCode":0}"#
    ));
}

#[test]
fn ws_auth_signature_matches_documented_prehash() {
    let signature = sign::ws_auth_sign(b"secret", "1700000000000");

    assert_eq!(
        signature,
        "9baf584ddf7a063dffe910d97ce4eac0cf7064058356de8b8d92f028e5ad936f"
    );
}

#[test]
fn order_create_request_matches_bybit_ws_schema() {
    let mut order = intent();
    order.time_in_force = TimeInForce::Gtc;
    let request = match order_create_request(cfg(), &order, "BTCUSDT".into(), 1) {
        Ok(request) => request,
        Err(error) => panic!("order create request: {error}"),
    };
    let arg = &request.args[0];

    assert_eq!(request.req_id, "i1");
    assert_eq!(request.op, "order.create");
    assert_eq!(request.header["X-BAPI-RECV-WINDOW"], "5000");
    assert_eq!(arg["category"], "linear");
    assert_eq!(arg["symbol"], "BTCUSDT");
    assert_eq!(arg["side"], "Buy");
    assert_eq!(arg["orderType"], "Limit");
    assert_eq!(arg["qty"], "0.01");
    assert_eq!(arg["price"], "50000");
    assert_eq!(arg["timeInForce"], "GTC");
    assert_eq!(arg["positionIdx"], 1);
    assert_eq!(arg["orderLinkId"], "cid-1");
}

#[test]
fn spot_order_and_cancel_use_spot_category_without_derivatives_fields() {
    let order = intent();
    let place = trade_request(
        "i1",
        "5000",
        OP_ORDER_CREATE,
        place_spot_order_arg(&order, "BTCUSDT".to_owned()).expect("spot order arg"),
    )
    .expect("spot place request");
    let arg = &place.args[0];
    assert_eq!(arg["category"], "spot");
    assert_eq!(arg["marketUnit"], "baseCoin");
    assert_eq!(arg["qty"], "0.01");
    assert!(arg.get("positionIdx").is_none());
    assert!(arg.get("reduceOnly").is_none());

    let cancel = CancelOrderRequest {
        exchange: "bybit".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };
    let cancel = trade_request(
        "i1",
        "5000",
        OP_ORDER_CANCEL,
        cancel_spot_order_arg(&cancel, "BTCUSDT".to_owned()).expect("spot cancel arg"),
    )
    .expect("spot cancel request");
    assert_eq!(cancel.args[0]["category"], "spot");
    assert_eq!(cancel.args[0]["symbol"], "BTCUSDT");
}

#[test]
fn order_create_preserves_ioc_and_fok_time_in_force() {
    let mut ioc = intent();
    ioc.time_in_force = TimeInForce::Ioc;
    let ioc_request = match order_create_request(cfg(), &ioc, "BTCUSDT".into(), 1) {
        Ok(request) => request,
        Err(error) => panic!("ioc create request: {error}"),
    };
    assert_eq!(ioc_request.args[0]["timeInForce"], "IOC");

    let mut fok = intent();
    fok.time_in_force = TimeInForce::Fok;
    let fok_request = match order_create_request(cfg(), &fok, "BTCUSDT".into(), 2) {
        Ok(request) => request,
        Err(error) => panic!("fok create request: {error}"),
    };
    assert_eq!(fok_request.args[0]["timeInForce"], "FOK");
}

#[test]
fn order_create_maps_gtx_to_post_only_time_in_force() {
    let mut order = intent();
    order.time_in_force = TimeInForce::Gtx;
    let request = match order_create_request(cfg(), &order, "BTCUSDT".into(), 1) {
        Ok(request) => request,
        Err(error) => panic!("gtx create request: {error}"),
    };

    assert_eq!(request.args[0]["orderType"], "Limit");
    assert_eq!(request.args[0]["timeInForce"], "PostOnly");
}

#[test]
fn order_create_market_sends_slippage_percent() {
    let mut order = intent();
    order.order_type = OrderType::Market;
    order.slippage_tolerance_bps = Some(10.0);
    let request = match order_create_request(cfg(), &order, "BTCUSDT".into(), 1) {
        Ok(request) => request,
        Err(error) => panic!("market create request: {error}"),
    };

    assert_eq!(request.args[0]["orderType"], "Market");
    assert_eq!(request.args[0].get("price"), None);
    assert_eq!(request.args[0].get("timeInForce"), None);
    assert_eq!(request.args[0]["slippageToleranceType"], "Percent");
    assert_eq!(request.args[0]["slippageTolerance"], "0.1");
}

#[test]
fn order_create_derives_overlong_req_id() {
    let mut order = intent();
    let raw = "r".repeat(37);
    order.id = raw.clone();
    let request =
        order_create_request(cfg(), &order, "BTCUSDT".into(), 1).expect("request serializes");

    assert_short_req_id(&raw, &request.req_id);
}

#[test]
fn order_create_derives_bad_order_link_id() {
    let mut order = intent();
    order.client_order_id = "cid:1".into();
    let request =
        order_create_request(cfg(), &order, "BTCUSDT".into(), 1).expect("request serializes");

    assert_short_order_link_id("cid:1", request.args[0]["orderLinkId"].as_str());
}

#[test]
fn order_create_rejects_empty_req_id() {
    let mut order = intent();
    order.id.clear();
    let error = order_create_request(cfg(), &order, "BTCUSDT".into(), 1).expect_err("empty reqId");

    assert!(api_message(error).contains("reqId cannot be empty"));
}

#[test]
fn order_cancel_request_matches_bybit_ws_schema() {
    let request = CancelOrderRequest {
        exchange: "bybit".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };
    let ws = match order_cancel_request(cfg(), &request, "BTCUSDT".into()) {
        Ok(request) => request,
        Err(error) => panic!("order cancel request: {error}"),
    };

    assert_eq!(ws.req_id, "i1");
    assert_eq!(ws.op, "order.cancel");
    assert_eq!(ws.args[0]["category"], "linear");
    assert_eq!(ws.args[0]["symbol"], "BTCUSDT");
    assert_eq!(ws.args[0]["orderLinkId"], "cid-1");
}

#[test]
fn order_cancel_derives_overlong_req_id() {
    let request = CancelOrderRequest {
        exchange: "bybit".into(),
        symbol: "BTC".into(),
        internal_order_id: "r".repeat(37),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };
    let ws = order_cancel_request(cfg(), &request, "BTCUSDT".into()).expect("cancel request");

    assert_short_req_id(&request.internal_order_id, &ws.req_id);
}

#[test]
fn parses_success_response_to_ack() {
    let row = match parse_trade_response(
        r#"{"reqId":"i1","retCode":0,"retMsg":"OK","data":{"orderId":"7","orderLinkId":"cid-1"}}"#,
    )
    .and_then(WsTradeResponse::into_result)
    {
        Ok(row) => row,
        Err(error) => panic!("ws success row: {error}"),
    };
    let ack = ack_from_result(
        "i1".into(),
        "public-cid".into(),
        "venue-cid".into(),
        row,
        LiveOrderState::Accepted,
        None,
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("7"));
    assert_eq!(ack.client_order_id, "public-cid");
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[test]
fn bybit_ws_place_order_ack_parses_official_fixture() {
    let row = parse_trade_response(include_str!(
        "../../fixtures/bybit/ws_order_create_ack.json"
    ))
    .and_then(WsTradeResponse::into_result)
    .expect("official Bybit WS order.create fixture parses");
    let ack = ack_from_result(
        "i1".into(),
        "public-cid".into(),
        "venue-cid".into(),
        row,
        LiveOrderState::Accepted,
        None,
    );

    assert_eq!(
        ack.exchange_order_id.as_deref(),
        Some("a4c1718e-fe53-4659-a118-1f6ecce04ad9")
    );
    assert_eq!(ack.client_order_id, "public-cid");
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[test]
fn bybit_ws_cancel_order_ack_parses_official_fixture() {
    let row = parse_trade_response(include_str!(
        "../../fixtures/bybit/ws_order_cancel_ack.json"
    ))
    .and_then(WsTradeResponse::into_result)
    .expect("official Bybit WS order.cancel fixture parses");
    let ack = ack_from_result(
        "i1".into(),
        "public-cid".into(),
        "venue-cid".into(),
        row,
        LiveOrderState::CancelRequested,
        Some("bybit cancel accepted; final state requires order query".to_owned()),
    );

    assert_eq!(
        ack.exchange_order_id.as_deref(),
        Some("a4c1718e-fe53-4659-a118-1f6ecce04ad9")
    );
    assert_eq!(ack.client_order_id, "public-cid");
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(ack.state, LiveOrderState::CancelRequested);
    assert!(ack
        .message
        .as_deref()
        .unwrap_or_default()
        .contains("final state requires order query"));
}

fn api_message(error: ExchangeError) -> String {
    match error {
        ExchangeError::Api {
            exchange,
            code,
            message,
        } => {
            assert_eq!(exchange, "bybit");
            assert_eq!(code, "validation");
            message
        }
        other => panic!("expected bybit validation error, got {other:?}"),
    }
}

fn assert_short_req_id(raw: &str, req_id: &str) {
    assert_ne!(req_id, raw);
    assert_eq!(req_id.len(), 27);
    assert!(req_id.starts_with("br-"));
}

fn assert_short_order_link_id(raw: &str, order_link_id: Option<&str>) {
    let id = order_link_id.expect("string orderLinkId");
    assert_ne!(id, raw);
    assert_eq!(id.len(), 27);
    assert!(id.starts_with("bb-"));
}

fn cfg() -> WsTradeConfig<'static> {
    WsTradeConfig {
        url: "wss://example.test",
        api_key: "k",
        api_secret: "s",
        recv_window: "5000",
        timeout_secs: 1,
    }
}

fn intent() -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "bybit".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}
