use super::*;
use crate::adapters::okx_instruments::{
    sizing_from_instrument, OkxInstrumentRow, OkxInstrumentRule, OkxOrderSizing,
};
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, OrderSide, OrderSource, OrderType, TimeInForce};

#[test]
fn login_request_matches_okx_ws_schema() {
    let request = login_request(cfg());
    let arg = &request.args[0];

    assert_eq!(request.op, "login");
    assert_eq!(arg.api_key, "k");
    assert_eq!(arg.passphrase, "p");
    // 修复 P1 9.4：timestamp 必须是 `seconds.milliseconds` 浮点字符串。
    assert!(!arg.timestamp.is_empty(), "timestamp empty");
    let parts: Vec<&str> = arg.timestamp.split('.').collect();
    assert_eq!(
        parts.len(),
        2,
        "expected `secs.ms` format, got `{}`",
        arg.timestamp
    );
    assert!(
        parts[0].parse::<u64>().is_ok(),
        "seconds not numeric: {}",
        parts[0]
    );
    assert_eq!(
        parts[1].len(),
        3,
        "expected 3-digit millis, got `{}`",
        parts[1]
    );
    assert!(
        parts[1].parse::<u32>().is_ok(),
        "millis not numeric: {}",
        parts[1]
    );
    assert_eq!(arg.sign.len(), 44);
}

#[test]
fn trade_session_uses_official_text_ping_heartbeat() {
    assert_eq!(okx_trade_heartbeat(), WsHeartbeat::Text("ping".to_owned()));
    assert!(okx_trade_heartbeat_response("pong"));
    assert!(!okx_trade_heartbeat_response(
        r#"{"event":"login","code":"0"}"#
    ));
}

#[test]
fn order_request_matches_okx_ws_schema() {
    let request = order_request(
        "i1",
        OP_ORDER,
        match place_order_ws_arg(
            &intent(),
            123_456,
            OkxTdMode::Cross,
            OkxPositionMode::Net,
            sizing(&intent()),
        ) {
            Ok(arg) => arg,
            Err(error) => panic!("okx order arg: {error}"),
        },
    );
    let arg = &request.args[0];

    assert_eq!(request.id, "i1");
    assert_eq!(request.op, "order");
    assert_eq!(arg["instIdCode"], 123_456);
    assert!(arg.get("instId").is_none());
    assert_eq!(arg["tdMode"], "cross");
    assert_eq!(arg["side"], "buy");
    assert_eq!(arg["ordType"], "limit");
    assert_eq!(arg["sz"], "1");
    assert_eq!(arg["px"], "50000");
    assert_eq!(arg["clOrdId"], "cid1");
    assert_eq!(arg["posSide"], "net");
}

#[test]
fn spot_order_and_cancel_use_cash_product_identity() {
    let order = intent();
    let place = order_request(
        "i1",
        OP_ORDER,
        place_spot_order_ws_arg(
            &order,
            "BTC-USDT".to_owned(),
            "0.01".to_owned(),
            Some("50000".to_owned()),
        )
        .expect("spot order arg"),
    );
    let arg = &place.args[0];
    assert_eq!(arg["instId"], "BTC-USDT");
    assert_eq!(arg["tdMode"], "cash");
    assert_eq!(arg["sz"], "0.01");
    assert!(arg.get("instIdCode").is_none());
    assert!(arg.get("posSide").is_none());
    assert!(arg.get("reduceOnly").is_none());

    let cancel = CancelOrderRequest {
        exchange: "okx".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid1".into(),
    };
    let cancel = order_request(
        "i1",
        OP_CANCEL_ORDER,
        cancel_spot_order_ws_arg(&cancel, "BTC-USDT".to_owned()).expect("spot cancel arg"),
    );
    assert_eq!(cancel.args[0]["instId"], "BTC-USDT");
    assert!(cancel.args[0].get("instIdCode").is_none());
}

#[test]
fn generated_request_ids_match_okx_transport_contract() {
    let first = next_request_id();
    let second = next_request_id();

    assert_eq!(first.len(), 32);
    assert!(first.bytes().all(|byte| byte.is_ascii_alphanumeric()));
    assert_ne!(first, second);
}

#[test]
fn market_order_request_uses_market_without_px() {
    let mut intent = intent();
    intent.order_type = OrderType::Market;
    let request = order_request(
        "i1",
        OP_ORDER,
        match place_order_ws_arg(
            &intent,
            123_456,
            OkxTdMode::Cross,
            OkxPositionMode::Net,
            sizing(&intent),
        ) {
            Ok(arg) => arg,
            Err(error) => panic!("okx market order arg: {error}"),
        },
    );
    let arg = &request.args[0];

    assert_eq!(arg["ordType"], "market");
    assert!(arg.get("px").is_none());
}

#[test]
fn order_request_preserves_ioc_and_fok_ord_type() {
    let mut ioc = intent();
    ioc.time_in_force = TimeInForce::Ioc;
    let ioc_arg = match place_order_ws_arg(
        &ioc,
        123_456,
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        sizing(&ioc),
    ) {
        Ok(arg) => arg,
        Err(error) => panic!("okx ioc arg: {error}"),
    };
    assert_eq!(ioc_arg["ordType"], "ioc");
    assert_eq!(ioc_arg["px"], "50000");

    let mut fok = intent();
    fok.time_in_force = TimeInForce::Fok;
    let fok_arg = match place_order_ws_arg(
        &fok,
        123_456,
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        sizing(&fok),
    ) {
        Ok(arg) => arg,
        Err(error) => panic!("okx fok arg: {error}"),
    };
    assert_eq!(fok_arg["ordType"], "fok");
    assert_eq!(fok_arg["px"], "50000");
}

#[test]
fn order_request_maps_gtx_to_post_only_ord_type() {
    let mut intent = intent();
    intent.time_in_force = TimeInForce::Gtx;
    let arg = match place_order_ws_arg(
        &intent,
        123_456,
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        sizing(&intent),
    ) {
        Ok(arg) => arg,
        Err(error) => panic!("okx gtx arg: {error}"),
    };

    assert_eq!(arg["ordType"], "post_only");
    assert_eq!(arg["px"], "50000");
}

#[test]
fn order_request_uses_isolated_td_mode_when_configured() {
    // 修复 P1 2.4：td_mode 需要按运行时配置传入。
    let request = order_request(
        "i1",
        OP_ORDER,
        match place_order_ws_arg(
            &intent(),
            123_456,
            OkxTdMode::Isolated,
            OkxPositionMode::Net,
            sizing(&intent()),
        ) {
            Ok(arg) => arg,
            Err(error) => panic!("okx order arg: {error}"),
        },
    );
    assert_eq!(request.args[0]["tdMode"], "isolated");
}

#[test]
fn cancel_request_matches_okx_ws_schema() {
    let request = CancelOrderRequest {
        exchange: "okx".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid1".into(),
    };
    let op = order_request(
        "i1",
        OP_CANCEL_ORDER,
        match cancel_order_ws_arg(&request, 123_456) {
            Ok(arg) => arg,
            Err(error) => panic!("okx cancel arg: {error}"),
        },
    );

    assert_eq!(op.id, "i1");
    assert_eq!(op.op, "cancel-order");
    assert_eq!(op.args[0]["instIdCode"], 123_456);
    assert!(op.args[0].get("instId").is_none());
    assert_eq!(op.args[0]["clOrdId"], "cid1");
}

#[test]
fn parses_success_response_to_ack_item() {
    let item = match parse_op_response(
        r#"{"id":"i1","op":"order","code":"0","msg":"","data":[{"ordId":"7","clOrdId":"cid1","sCode":"0","sMsg":""}]}"#,
    )
    .and_then(WsOpResponse::into_item)
    {
        Ok(item) => item,
        Err(error) => panic!("okx ws item: {error}"),
    };
    let ack = ack_from_item("i1".into(), "fallback".into(), item);

    assert_eq!(ack.exchange_order_id.as_deref(), Some("7"));
    assert_eq!(ack.client_order_id, "fallback");
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("cid1")
    );
}

#[test]
fn okx_ws_place_order_ack_parses_official_fixture() {
    let item = ws_ack_item(include_str!(
        "../../fixtures/okx/ws_trade_place_order_ack.json"
    ));
    let ack = ack_from_item("i1".into(), "fallback".into(), item);

    assert_eq!(ack.exchange_order_id.as_deref(), Some("12345689"));
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("oktswap6")
    );
}

#[test]
fn okx_ws_cancel_order_ack_parses_official_fixture() {
    let item = ws_ack_item(include_str!(
        "../../fixtures/okx/ws_trade_cancel_order_ack.json"
    ));
    let ack = ack_from_item("i1".into(), "fallback".into(), item);

    assert_eq!(ack.exchange_order_id.as_deref(), Some("2510789768709120"));
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("oktswap6")
    );
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

fn intent() -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn sizing(intent: &OrderIntent) -> OkxOrderSizing {
    sizing_from_instrument(intent, &rule()).expect("okx test sizing")
}

fn rule() -> OkxInstrumentRule {
    OkxInstrumentRule::from_row(OkxInstrumentRow {
        inst_id: "BTC-USDT-SWAP".into(),
        inst_id_code: Some(123_456),
        contract_value: "0.01".into(),
        contract_value_currency: "BTC".into(),
        lot_size: "1".into(),
        min_size: "1".into(),
        tick_size: "0.1".into(),
        state: "live".into(),
    })
    .expect("okx test instrument")
}

fn ws_ack_item(text: &str) -> OrderAckItem {
    match parse_op_response(text).and_then(WsOpResponse::into_item) {
        Ok(item) => item,
        Err(error) => panic!("okx ws item: {error}"),
    }
}
