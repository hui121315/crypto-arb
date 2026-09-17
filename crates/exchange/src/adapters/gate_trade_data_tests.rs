use super::*;
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, MarginMode, OrderSource, TimeInForce};

#[test]
fn place_order_converts_base_qty_to_contracts() {
    assert_eq!(
        crate::adapters::gate::FUTURES_ORDERS_PATH,
        "/api/v4/futures/usdt/orders"
    );

    let mut order = intent(OrderSide::Sell, OrderType::Limit);
    order.time_in_force = TimeInForce::Gtc;
    let params = place_order_params(&order, "BTC_USDT".into(), 0.0001).expect("params serializes");

    assert_eq!(params["contract"], "BTC_USDT");
    assert_eq!(params["size"], "-100");
    assert_eq!(params["price"], "50000");
    assert_eq!(params["tif"], "gtc");
    assert_eq!(params["text"], "t-cid-1");
}

#[test]
fn limit_order_uses_requested_official_ioc_or_fok_tif() {
    let mut ioc = intent(OrderSide::Buy, OrderType::Limit);
    ioc.time_in_force = TimeInForce::Ioc;
    let ioc_params =
        place_order_params(&ioc, "BTC_USDT".into(), 0.0001).expect("ioc params serializes");

    let mut fok = intent(OrderSide::Buy, OrderType::Limit);
    fok.time_in_force = TimeInForce::Fok;
    let fok_params =
        place_order_params(&fok, "BTC_USDT".into(), 0.0001).expect("fok params serializes");

    assert_eq!(ioc_params["tif"], "ioc");
    assert_eq!(fok_params["tif"], "fok");
}

#[test]
fn limit_order_rejects_non_official_gtx_tif() {
    let mut order = intent(OrderSide::Buy, OrderType::Limit);
    order.time_in_force = TimeInForce::Gtx;
    let error = place_order_params(&order, "BTC_USDT".into(), 0.0001)
        .expect_err("gtx is not a gate futures tif");

    assert!(
        error.to_string().contains("no gtx tif"),
        "unexpected error: {error}"
    );
}

#[test]
fn post_only_uses_poc_tif() {
    let params = place_order_params(
        &intent(OrderSide::Buy, OrderType::PostOnly),
        "BTC_USDT".into(),
        0.0001,
    )
    .expect("params serializes");

    assert_eq!(params["size"], "100");
    assert_eq!(params["tif"], "poc");
}

#[test]
fn reduce_only_uses_official_optional_payload_shape() {
    let mut reducing = intent(OrderSide::Buy, OrderType::Limit);
    reducing.reduce_only = true;
    let reducing_params =
        place_order_params(&reducing, "BTC_USDT".into(), 0.0001).expect("params serializes");

    let opening = intent(OrderSide::Buy, OrderType::Limit);
    let opening_params =
        place_order_params(&opening, "BTC_USDT".into(), 0.0001).expect("params serializes");

    assert_eq!(reducing_params["reduce_only"], true);
    assert!(opening_params.get("reduce_only").is_none());
}

#[test]
fn market_order_uses_official_zero_price_ioc_shape() {
    let params = place_order_params(
        &intent(OrderSide::Buy, OrderType::Market),
        "BTC_USDT".into(),
        0.0001,
    )
    .expect("market params serialize");

    assert_eq!(params["contract"], "BTC_USDT");
    assert_eq!(params["size"], "100");
    assert_eq!(params["price"], "0");
    assert_eq!(params["tif"], "ioc");
}

#[test]
fn cancel_requires_numeric_exchange_order_id() {
    let request = CancelOrderRequest {
        exchange: "gate".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: Some("777".into()),
        client_order_id: "cid-1".into(),
    };
    let params = cancel_order_params(&request).expect("cancel params serializes");

    assert_eq!(params["order_id"], 777);
    assert!(
        params.get("contract").is_none(),
        "Gate WS futures.order_cancel req_param is order_id-only"
    );
}

#[test]
fn gate_text_keeps_existing_prefix() {
    assert_eq!(gate_text("cid-1").expect("gate text"), "t-cid-1");
    assert_eq!(gate_text("t-cid-1").expect("gate text"), "t-cid-1");
}

#[test]
fn gate_text_compacts_long_or_invalid_ids_to_official_policy() {
    let long = gate_text("hedge-0123456789abcdef-0123456789abcdef-long").expect("long gate text");
    let invalid = gate_text("cid/with spaces/中文").expect("invalid gate text");

    assert_gate_text_policy(&long);
    assert_gate_text_policy(&invalid);
    assert_ne!(long, invalid);
    assert_eq!(
        long,
        gate_text("hedge-0123456789abcdef-0123456789abcdef-long").expect("stable gate text")
    );
}

#[test]
fn ack_maps_numeric_exchange_order_id() {
    let row = GateOrderAckRow { order_id: Some(7) };
    let ack = ack_from_row(
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
        ack.identity_update.public_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("t-cid-1")
    );
}

#[test]
fn gate_place_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/gate/futures_place_order_ack.json");
    let row: GateOrderAckRow = serde_json::from_str(body).expect("gate futures order fixture");
    let ack = ack_from_row(
        "i1".into(),
        "cid-1".into(),
        "t-cid-1".into(),
        &row,
        LiveOrderState::Accepted,
        None,
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("15675394"));
    assert_eq!(
        ack.identity_update.exchange_order_id.as_deref(),
        Some("15675394")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("t-cid-1")
    );
}

fn intent(side: OrderSide, order_type: OrderType) -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "gate".into(),
        symbol: "BTC".into(),
        side,
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

fn assert_gate_text_policy(text: &str) {
    let body = text.strip_prefix("t-").expect("gate prefix");
    assert!(body.len() <= 28, "body too long: {body}");
    assert!(
        body.chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.')),
        "invalid body: {body}"
    );
}
