use super::*;
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, OrderSource, TimeInForce};

#[test]
fn order_place_request_matches_binance_ws_schema() {
    let cfg = cfg();
    let mut order = intent();
    order.time_in_force = TimeInForce::Gtc;
    let request = order_place_request(cfg, &order, "BTCUSDT", "BOTH").expect("limit price ok");

    assert_eq!(request.method, "order.place");
    assert_eq!(request.params["apiKey"], "k");
    assert_eq!(request.params["symbol"], "BTCUSDT");
    assert_eq!(request.params["side"], "BUY");
    assert_eq!(request.params["positionSide"], "BOTH");
    assert_eq!(request.params["type"], "LIMIT");
    assert_eq!(request.params["quantity"], "0.01");
    assert_eq!(request.params["price"], "50000");
    assert_eq!(request.params["timeInForce"], "GTC");
    assert_eq!(request.params["newClientOrderId"], "cid-1");
    assert_eq!(request.params["newOrderRespType"], "ACK");
    assert_eq!(request.params["recvWindow"], 5000);
    assert!(request.params["timestamp"].is_u64());
    assert!(request.params.contains_key("signature"));
}

#[test]
fn spot_order_requests_use_spot_schema_without_futures_fields() {
    let cfg = cfg();
    let place = spot_order_place_request(cfg.signing_values(), &intent(), "BTCUSDT")
        .expect("spot place request");

    assert_eq!(place.method, METHOD_ORDER_PLACE);
    assert_eq!(place.params["symbol"], "BTCUSDT");
    assert_eq!(place.params["quantity"], "0.01");
    assert_eq!(place.params["newOrderRespType"], "ACK");
    assert!(!place.params.contains_key("positionSide"));
    assert!(!place.params.contains_key("reduceOnly"));

    let cancel = CancelOrderRequest {
        exchange: "binance".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };
    let cancel = order_cancel_request_with_signing(cfg.signing_values(), &cancel, "BTCUSDT");
    assert_eq!(cancel.method, METHOD_ORDER_CANCEL);
    assert_eq!(cancel.params["origClientOrderId"], "cid-1");

    let query = signed_read_request(
        cfg.signing_values(),
        "query-1",
        METHOD_ORDER_STATUS,
        BTreeMap::from([
            ("origClientOrderId", "cid-1".to_owned()),
            ("symbol", "BTCUSDT".to_owned()),
        ]),
    );
    assert_eq!(query.method, METHOD_ORDER_STATUS);
    assert_eq!(query.params["symbol"], "BTCUSDT");
}

#[test]
fn order_place_request_validates_official_client_order_id_rule() {
    let cfg = cfg();
    let mut order = intent();
    order.client_order_id = "ABC.def:/xyz_09-1".into();
    let request = order_place_request(cfg, &order, "BTCUSDT", "BOTH").expect("official chars pass");
    assert_eq!(request.params["newClientOrderId"], "ABC.def:/xyz_09-1");

    order.client_order_id = String::new();
    assert!(matches!(
        order_place_request(cfg, &order, "BTCUSDT", "BOTH"),
        Err(ExchangeError::Api { .. })
    ));

    order.client_order_id = "a".repeat(37);
    assert!(matches!(
        order_place_request(cfg, &order, "BTCUSDT", "BOTH"),
        Err(ExchangeError::Api { .. })
    ));

    order.client_order_id = "bad#id".into();
    assert!(matches!(
        order_place_request(cfg, &order, "BTCUSDT", "BOTH"),
        Err(ExchangeError::Api { .. })
    ));
}

#[test]
fn limit_ioc_and_fok_preserve_time_in_force() {
    let cfg = cfg();
    let mut ioc = intent();
    ioc.time_in_force = TimeInForce::Ioc;
    let ioc_request = order_place_request(cfg, &ioc, "BTCUSDT", "BOTH").expect("ioc ok");
    assert_eq!(ioc_request.params["timeInForce"], "IOC");

    let mut fok = intent();
    fok.time_in_force = TimeInForce::Fok;
    let fok_request = order_place_request(cfg, &fok, "BTCUSDT", "BOTH").expect("fok ok");
    assert_eq!(fok_request.params["timeInForce"], "FOK");
}

#[test]
fn limit_gtx_uses_official_gtx_time_in_force() {
    let cfg = cfg();
    let mut order = intent();
    order.time_in_force = TimeInForce::Gtx;
    let request = order_place_request(cfg, &order, "BTCUSDT", "BOTH").expect("gtx ok");

    assert_eq!(request.params["type"], "LIMIT");
    assert_eq!(request.params["timeInForce"], "GTX");
}

/// 修复 P1 9.3：`LIMIT` / `PostOnly` 必须携正价格，否则早期 validation 错误。
#[test]
fn limit_without_price_is_rejected_early() {
    let cfg = cfg();
    let mut bad = intent();
    bad.price = None;
    let err = order_place_request(cfg, &bad, "BTCUSDT", "BOTH").unwrap_err();
    assert!(matches!(err, ExchangeError::Api { code, .. } if code == "validation"));

    bad.price = Some(0.0);
    let err = order_place_request(cfg, &bad, "BTCUSDT", "BOTH").unwrap_err();
    assert!(matches!(err, ExchangeError::Api { code, .. } if code == "validation"));

    bad.price = Some(-1.0);
    let err = order_place_request(cfg, &bad, "BTCUSDT", "BOTH").unwrap_err();
    assert!(matches!(err, ExchangeError::Api { code, .. } if code == "validation"));

    bad.price = Some(f64::NAN);
    let err = order_place_request(cfg, &bad, "BTCUSDT", "BOTH").unwrap_err();
    assert!(matches!(err, ExchangeError::Api { code, .. } if code == "validation"));

    // PostOnly 同样校验
    bad.order_type = OrderType::PostOnly;
    bad.price = None;
    assert!(order_place_request(cfg, &bad, "BTCUSDT", "BOTH").is_err());

    // Market 单不校验 price
    let mut mkt = intent();
    mkt.order_type = OrderType::Market;
    mkt.price = None;
    assert!(order_place_request(cfg, &mkt, "BTCUSDT", "BOTH").is_ok());
}

#[test]
fn order_place_request_serializes_verified_position_side() {
    let cfg = cfg();
    let buy = intent();
    let buy_request = order_place_request(cfg, &buy, "BTCUSDT", "LONG").expect("hedge buy side");
    assert_eq!(buy_request.params["positionSide"], "LONG");

    let mut sell = intent();
    sell.side = OrderSide::Sell;
    let sell_request =
        order_place_request(cfg, &sell, "BTCUSDT", "SHORT").expect("hedge sell side");
    assert_eq!(sell_request.params["positionSide"], "SHORT");
}

#[test]
fn order_place_request_maps_hedge_short_close_without_reduce_only() {
    let cfg = cfg();
    let mut order = intent();
    order.reduce_only = true;
    let request =
        order_place_request(cfg, &order, "BTCUSDT", "SHORT").expect("verified hedge short close");

    assert_eq!(request.params["positionSide"], "SHORT");
    assert!(!request.params.contains_key("reduceOnly"));
}

#[test]
fn order_cancel_request_matches_binance_ws_schema() {
    let cfg = cfg();
    let request = CancelOrderRequest {
        exchange: "binance".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };

    let ws = order_cancel_request(cfg, &request, "BTCUSDT");

    assert_eq!(ws.method, "order.cancel");
    assert_eq!(ws.params["apiKey"], "k");
    assert_eq!(ws.params["symbol"], "BTCUSDT");
    assert_eq!(ws.params["origClientOrderId"], "cid-1");
    assert_eq!(ws.params["recvWindow"], 5000);
    assert!(ws.params["timestamp"].is_u64());
    assert!(ws.params.contains_key("signature"));
}

#[test]
fn private_read_requests_match_current_binance_ws_methods() {
    let cfg = cfg();
    let account = signed_read_request(
        cfg.signing_values(),
        "account-1",
        METHOD_ACCOUNT_STATUS_V2,
        BTreeMap::new(),
    );
    assert_eq!(account.method, "v2/account.status");
    assert_eq!(account.params["apiKey"], "k");
    assert_eq!(account.params["recvWindow"], 5000);
    assert!(account.params["timestamp"].is_u64());
    assert!(account.params.contains_key("signature"));

    let position = signed_read_request(
        cfg.signing_values(),
        "position-1",
        METHOD_ACCOUNT_POSITION_V2,
        BTreeMap::from([("symbol", "BTCUSDT".to_owned())]),
    );
    assert_eq!(position.method, "v2/account.position");
    assert_eq!(position.params["symbol"], "BTCUSDT");

    let order = signed_read_request(
        cfg.signing_values(),
        "order-1",
        METHOD_ORDER_STATUS,
        BTreeMap::from([
            ("origClientOrderId", "cid-1".to_owned()),
            ("symbol", "BTCUSDT".to_owned()),
        ]),
    );
    assert_eq!(order.method, "order.status");
    assert_eq!(order.params["origClientOrderId"], "cid-1");
    assert_eq!(order.params["symbol"], "BTCUSDT");
}

#[test]
fn user_stream_control_requests_match_current_binance_ws_methods() {
    for method in [
        METHOD_USER_STREAM_START,
        METHOD_USER_STREAM_PING,
        METHOD_USER_STREAM_STOP,
    ] {
        let request = api_key_request("stream-1", method, "k");
        assert_eq!(request.method, method);
        assert_eq!(request.params["apiKey"], "k");
        assert!(!request.params.contains_key("signature"));
        assert!(!request.params.contains_key("timestamp"));
    }
}

/// 修复 P1 9.2：小数字不应被序列化为科学计数法（Binance 会拒绝）。
#[test]
fn number_param_avoids_scientific_notation() {
    // 极小值：之前 `to_string()` 会输出 "0.00000001" 或 "1e-8"
    assert_eq!(number_param(0.000_000_01), "0.00000001");
    assert_eq!(number_param(1e-10), "0.0000000001");
    // 整数/普通值仍保持期望格式
    assert_eq!(number_param(50_000.0), "50000");
    assert_eq!(number_param(50_000.5), "50000.5");
    // 零边界
    assert_eq!(number_param(0.0), "0");
    // -0.0 经 12 位 trim 为 "-0"，与 binance.rs 同实现一致（Binance 接受）
    let neg_zero = number_param(-0.0);
    assert!(neg_zero == "0" || neg_zero == "-0");
    // 负值
    assert_eq!(number_param(-1.5), "-1.5");
}

#[test]
fn parses_success_response_to_ack() {
    let response =
        match parse_response(r#"{"id":"i1","status":200,"result":{"orderId":7,"status":"NEW"}}"#)
            .and_then(WsResponse::into_result)
        {
            Ok(result) => result,
            Err(error) => panic!("ws success result: {error}"),
        };

    let ack = ack_from_result("i1".into(), "cid-1".into(), &response);

    assert_eq!(ack.exchange_order_id.as_deref(), Some("7"));
    assert_eq!(ack.client_order_id, "cid-1");
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[test]
fn binance_ws_place_order_ack_parses_official_fixture() {
    let response = parse_response(include_str!(
        "../../fixtures/binance/ws_order_place_success.json"
    ))
    .and_then(WsResponse::into_result)
    .expect("official Binance WS place-order fixture parses");
    let ack = ack_from_result("i1".into(), "x-crossline-cid-1".into(), &response);

    assert_eq!(ack.exchange_order_id.as_deref(), Some("283194212"));
    assert_eq!(ack.client_order_id, "x-crossline-cid-1");
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[test]
fn binance_ws_cancel_order_ack_parses_official_fixture() {
    let response = parse_response(include_str!(
        "../../fixtures/binance/ws_order_cancel_success.json"
    ))
    .and_then(WsResponse::into_result)
    .expect("official Binance WS cancel-order fixture parses");
    let ack = ack_from_result("i1".into(), "myOrder1".into(), &response);

    assert_eq!(ack.exchange_order_id.as_deref(), Some("283194212"));
    assert_eq!(ack.client_order_id, "myOrder1");
    assert_eq!(ack.state, LiveOrderState::Cancelled);
}

#[test]
fn binance_ws_account_status_v2_parses_official_fixture() {
    let response: TypedWsResponse<AccountInfoV3> = parse_typed_response(include_str!(
        "../../fixtures/binance/ws_account_status_v2.json"
    ))
    .expect("official Binance WS account fixture envelope parses");
    let item = response
        .into_result(METHOD_ACCOUNT_STATUS_V2)
        .expect("official Binance WS account fixture result parses");
    let read = parse_account_info(item, Some("USDT"), 1_700_000_000_000, ACCOUNT_STATUS_SOURCE)
        .expect("account projection parses");

    assert_eq!(read.summary.total_equity_usd, 126.72469206);
    assert_eq!(read.summary.source, ACCOUNT_STATUS_SOURCE);
    assert_eq!(read.balances.len(), 1);
}

#[test]
fn binance_ws_balance_v2_parses_official_fixture() {
    let response: TypedWsResponse<Vec<BalanceItem>> = parse_typed_response(include_str!(
        "../../fixtures/binance/ws_account_balance_v2.json"
    ))
    .expect("official Binance WS balance fixture envelope parses");
    let items = response
        .into_result(METHOD_ACCOUNT_BALANCE_V2)
        .expect("official Binance WS balance fixture result parses");
    let balances = parse_balances(items, Some("USDT")).expect("balance projection parses");

    let usdt = balances.get("USDT").expect("USDT balance");
    assert_eq!(usdt.available, 23.72469206);
    assert_eq!(usdt.total, 122607.35137903);
}

#[test]
fn binance_ws_position_v2_parses_official_fixture() {
    let response: TypedWsResponse<Vec<PositionItem>> =
        parse_typed_response(include_str!("../../fixtures/binance/ws_position_v2.json"))
            .expect("official Binance WS position fixture envelope parses");
    let items = response
        .into_result(METHOD_ACCOUNT_POSITION_V2)
        .expect("official Binance WS position fixture result parses");
    let parsed =
        parse_positions_with_mode(items, Some("ADAUSDT")).expect("position projection parses");

    assert_eq!(parsed.mode.map(|mode| mode.as_str()), Some("one_way"));
    assert_eq!(parsed.rows.len(), 1);
    assert_eq!(parsed.rows[0].mark_price, 0.41047590);
    assert_eq!(parsed.rows[0].margin, 0.61571385);
}

#[test]
fn binance_ws_order_status_parses_supported_official_schema() {
    let response: TypedWsResponse<OpenOrderItem> = parse_typed_response(include_str!(
        "../../fixtures/binance/ws_order_status_filled.json"
    ))
    .expect("official Binance WS order-status envelope parses");
    let item = response
        .into_result(METHOD_ORDER_STATUS)
        .expect("official Binance WS order-status result parses");
    let order = parse_open_order(&item).expect("order projection parses");

    assert_eq!(order.status, shared_types::OrderStatus::Filled);
    assert_eq!(order.filled_quantity, 0.01);
    assert_eq!(order.filled_price, 50_010.0);
    assert_eq!(order.reduce_only, Some(true));
}

#[test]
fn binance_ws_user_stream_control_parses_official_fixtures() {
    let start: TypedWsResponse<UserDataStreamResult> = parse_typed_response(include_str!(
        "../../fixtures/binance/ws_user_data_stream_start.json"
    ))
    .expect("official Binance WS user-stream start envelope parses");
    let listen_key = start
        .into_result(METHOD_USER_STREAM_START)
        .expect("official Binance WS user-stream start result parses")
        .listen_key
        .expect("listenKey");
    assert!(listen_key.starts_with("xs0m"));

    let stop: TypedWsResponse<Value> = parse_typed_response(include_str!(
        "../../fixtures/binance/ws_user_data_stream_stop.json"
    ))
    .expect("official Binance WS user-stream stop envelope parses");
    assert_eq!(
        stop.into_result(METHOD_USER_STREAM_STOP)
            .expect("official Binance WS user-stream stop result parses"),
        serde_json::json!({})
    );
}

fn cfg() -> WsTradeConfig<'static> {
    WsTradeConfig {
        url: "wss://example.test",
        api_key: "k",
        api_secret: "s",
        timeout_secs: 1,
        time_offset_ms: 0,
    }
}

fn intent() -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "binance".into(),
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
