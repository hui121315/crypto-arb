use super::*;
use crate::error::ExchangeError;
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, MarginMode, OrderSource, TimeInForce};

#[test]
fn limit_order_uses_gtc_and_price() {
    assert_eq!(crate::adapters::bybit::PLACE_ORDER_PATH, "/v5/order/create");

    let mut order = intent(OrderType::Limit);
    order.time_in_force = TimeInForce::Gtc;
    let value = place_order_arg(&order, "BTCUSDT".into(), 0).expect("limit arg serializes");

    assert_eq!(value["category"], "linear");
    assert_eq!(value["symbol"], "BTCUSDT");
    assert_eq!(value["side"], "Buy");
    assert_eq!(value["orderType"], "Limit");
    assert_eq!(value["qty"], "0.01");
    assert_eq!(value["price"], "50000");
    assert_eq!(value["timeInForce"], "GTC");
    assert_eq!(value["positionIdx"], 0);
    assert_eq!(value["orderLinkId"], "cid-1");
}

#[test]
fn pre_check_order_reuses_official_create_order_shape() {
    let mut order = intent(OrderType::Limit);
    order.time_in_force = TimeInForce::Gtc;
    let value = pre_check_order_arg(&order, "BTCUSDT".into(), 0).expect("pre-check arg serializes");

    assert_eq!(value["category"], "linear");
    assert_eq!(value["symbol"], "BTCUSDT");
    assert_eq!(value["side"], "Buy");
    assert_eq!(value["orderType"], "Limit");
    assert_eq!(value["qty"], "0.01");
    assert_eq!(value["price"], "50000");
    assert_eq!(value["timeInForce"], "GTC");
    assert_eq!(value["positionIdx"], 0);
    assert_eq!(value["orderLinkId"], "cid-1");
}

#[test]
fn market_order_sends_official_percent_slippage() {
    let mut order = intent(OrderType::Market);
    order.slippage_tolerance_bps = Some(5.0);
    let value = place_order_arg(&order, "BTCUSDT".into(), 0).expect("market arg serializes");

    assert_eq!(value["orderType"], "Market");
    assert_eq!(value.get("price"), None);
    assert_eq!(value.get("timeInForce"), None);
    assert_eq!(value["slippageToleranceType"], "Percent");
    assert_eq!(value["slippageTolerance"], "0.05");
}

#[test]
fn market_order_requires_slippage_tolerance_evidence() {
    let error = place_order_arg(&intent(OrderType::Market), "BTCUSDT".into(), 0)
        .expect_err("market needs slippage evidence");

    assert!(api_message(error).contains("slippageTolerance"));
}

#[test]
fn market_order_rejects_fractional_bps() {
    let mut order = intent(OrderType::Market);
    order.slippage_tolerance_bps = Some(1.5);
    let error = place_order_arg(&order, "BTCUSDT".into(), 0).expect_err("fractional bps");

    assert!(api_message(error).contains("whole bps"));
}

#[test]
fn post_only_order_uses_limit_post_only() {
    let value = place_order_arg(&intent(OrderType::PostOnly), "BTCUSDT".into(), 0)
        .expect("post-only arg serializes");

    assert_eq!(value["orderType"], "Limit");
    assert_eq!(value["timeInForce"], "PostOnly");
}

#[test]
fn limit_ioc_and_fok_preserve_time_in_force() {
    let mut ioc = intent(OrderType::Limit);
    ioc.time_in_force = TimeInForce::Ioc;
    let ioc_value = place_order_arg(&ioc, "BTCUSDT".into(), 0).expect("ioc arg serializes");
    assert_eq!(ioc_value["timeInForce"], "IOC");

    let mut fok = intent(OrderType::Limit);
    fok.time_in_force = TimeInForce::Fok;
    let fok_value = place_order_arg(&fok, "BTCUSDT".into(), 0).expect("fok arg serializes");
    assert_eq!(fok_value["timeInForce"], "FOK");
}

#[test]
fn limit_gtx_uses_bybit_post_only_time_in_force() {
    let mut order = intent(OrderType::Limit);
    order.time_in_force = TimeInForce::Gtx;
    let value = place_order_arg(&order, "BTCUSDT".into(), 0).expect("gtx arg serializes");

    assert_eq!(value["orderType"], "Limit");
    assert_eq!(value["timeInForce"], "PostOnly");
}

#[test]
fn reduce_only_true_is_serialized() {
    let mut order = intent(OrderType::Limit);
    order.reduce_only = true;
    let value = place_order_arg(&order, "BTCUSDT".into(), 0).expect("arg serializes");

    assert_eq!(value["reduceOnly"], true);
}

#[test]
fn hedge_position_idx_is_serialized() {
    let buy =
        place_order_arg(&intent(OrderType::Limit), "BTCUSDT".into(), 1).expect("buy hedge arg");
    let sell =
        place_order_arg(&intent(OrderType::Limit), "BTCUSDT".into(), 2).expect("sell hedge arg");

    assert_eq!(buy["positionIdx"], 1);
    assert_eq!(sell["positionIdx"], 2);
}

#[test]
fn order_link_id_accepts_official_36_char_charset() {
    let mut order = intent(OrderType::Limit);
    let client_id = format!("{}Z", "Aa0-_".repeat(7));
    order.client_order_id = client_id.clone();
    let value = place_order_arg(&order, "BTCUSDT".into(), 0).expect("arg serializes");

    assert_eq!(client_id.len(), 36);
    assert_eq!(value["orderLinkId"], client_id);
}

#[test]
fn order_link_id_derives_over_36_chars() {
    let mut order = intent(OrderType::Limit);
    let raw = "a".repeat(37);
    order.client_order_id = raw.clone();
    let value = place_order_arg(&order, "BTCUSDT".into(), 0).expect("arg serializes");

    assert_derived_order_link_id(&raw, &value);
    assert_eq!(
        value["orderLinkId"],
        bybit_order_link_id(&raw).expect("stable venue id")
    );
}

#[test]
fn order_link_id_derives_unsupported_chars() {
    let mut order = intent(OrderType::Limit);
    let raw = "cid:1";
    order.client_order_id = raw.into();
    let value = place_order_arg(&order, "BTCUSDT".into(), 0).expect("arg serializes");

    assert_derived_order_link_id(raw, &value);
}

#[test]
fn order_link_id_rejects_empty_public_id() {
    let mut order = intent(OrderType::Limit);
    order.client_order_id = "  ".into();
    let error = place_order_arg(&order, "BTCUSDT".into(), 0).expect_err("empty id");

    assert!(api_message(error).contains("orderLinkId cannot be empty"));
}

#[test]
fn cancel_arg_keeps_order_id_and_link_id() {
    let request = CancelOrderRequest {
        exchange: "bybit".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: Some("oid-1".into()),
        client_order_id: "cid-1".into(),
    };
    let value = cancel_order_arg(&request, "BTCUSDT".into()).expect("cancel arg serializes");

    assert_eq!(value["category"], "linear");
    assert_eq!(value["symbol"], "BTCUSDT");
    assert_eq!(value["orderId"], "oid-1");
    assert_eq!(value["orderLinkId"], "cid-1");
}

#[test]
fn cancel_arg_derives_bad_order_link_id() {
    let request = CancelOrderRequest {
        exchange: "bybit".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: Some("oid-1".into()),
        client_order_id: "cid/1".into(),
    };
    let value = cancel_order_arg(&request, "BTCUSDT".into()).expect("cancel arg serializes");

    assert_derived_order_link_id("cid/1", &value);
}

#[test]
fn number_param_strips_trailing_zeros() {
    assert_eq!(number_param(1.2300).expect("number"), "1.23");
    assert_eq!(number_param(1.0).expect("number"), "1");
}

#[test]
fn ack_reuses_fallback_client_id_when_missing() {
    let row = OrderAckRow {
        order_id: "oid-1".into(),
        order_link_id: String::new(),
    };
    let ack = ack_from_row(
        "i1".into(),
        "public-cid".into(),
        "venue-cid".into(),
        row,
        LiveOrderState::Accepted,
        None,
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("oid-1"));
    assert_eq!(ack.client_order_id, "public-cid");
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("public-cid")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("venue-cid")
    );
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

fn assert_derived_order_link_id(raw: &str, value: &serde_json::Value) {
    let id = value["orderLinkId"].as_str().expect("string orderLinkId");
    assert_ne!(id, raw);
    assert_eq!(id.len(), 27);
    assert!(id.starts_with("bb-"));
    assert!(id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'));
}

fn intent(order_type: OrderType) -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Testnet,
        exchange: "bybit".into(),
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

#[test]
fn bybit_place_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/bybit/order_create_ack.json");
    let row = serde_json::from_str::<
        crate::adapters::bybit_response::BybitObjectResponse<OrderAckRow>,
    >(body)
    .expect("official bybit order-create envelope parses")
    .into_result("place order")
    .expect("retCode 0 yields result row");
    let ack = ack_from_row(
        "internal-bybit-1".to_owned(),
        "public-cid".to_owned(),
        "venue-cid".to_owned(),
        row,
        LiveOrderState::Accepted,
        None,
    );
    assert_eq!(
        ack.exchange_order_id.as_deref(),
        Some("1521264544649423872")
    );
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[test]
fn bybit_cancel_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/bybit/order_cancel_ack.json");
    let row = serde_json::from_str::<
        crate::adapters::bybit_response::BybitObjectResponse<OrderAckRow>,
    >(body)
    .expect("official bybit order-cancel envelope parses")
    .into_result("cancel order")
    .expect("retCode 0 yields result row");
    let ack = ack_from_row(
        "internal-bybit-cancel-1".to_owned(),
        "public-cid".to_owned(),
        "venue-cid".to_owned(),
        row,
        LiveOrderState::CancelRequested,
        Some("bybit cancel request accepted; final state requires order stream or query".into()),
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("1779020"));
    assert_eq!(ack.client_order_id, "public-cid");
    assert_eq!(ack.state, LiveOrderState::CancelRequested);
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("public-cid")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("bybit-cli-7")
    );
    assert_eq!(
        ack.identity_update.exchange_order_id.as_deref(),
        Some("1779020")
    );
}
