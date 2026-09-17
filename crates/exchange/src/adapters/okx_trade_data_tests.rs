use super::*;
use crate::adapters::okx_instruments::{
    number_param, sizing_from_instrument, OkxInstrumentRow, OkxInstrumentRule, OkxOrderSizing,
};
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, MarginMode, OrderSource, TimeInForce};

#[test]
fn ack_reject_maps_to_order_ack() {
    let ack = ack_from_item(
        "i1".into(),
        "c1".into(),
        OrderAckItem {
            ord_id: String::new(),
            cl_ord_id: "c1".into(),
            s_code: "51008".into(),
            s_msg: "insufficient balance".into(),
        },
    );
    assert_eq!(ack.state, LiveOrderState::Rejected);
    assert_eq!(ack.client_order_id, "c1");
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("c1")
    );
    assert_eq!(ack.message.as_deref(), Some("51008 insufficient balance"));
}

#[test]
fn number_param_avoids_scientific_notation() {
    assert_eq!(number_param(0.000_000_01), "0.00000001");
    assert_eq!(number_param(0.000_000_001), "0.000000001");
    assert!(!number_param(1e-10).contains('e'));
}

#[test]
fn number_param_handles_zero_and_normal() {
    assert_eq!(number_param(0.0), "0");
    assert_eq!(number_param(1.0), "1");
    assert_eq!(number_param(0.5), "0.5");
    assert_eq!(number_param(30_000.123), "30000.123");
}

#[test]
fn number_param_strips_trailing_zeros() {
    assert_eq!(number_param(1.230_000_000), "1.23");
    assert_eq!(number_param(100.0), "100");
}

#[test]
fn td_mode_string_mapping() {
    assert_eq!(OkxTdMode::Cross.as_str(), "cross");
    assert_eq!(OkxTdMode::Isolated.as_str(), "isolated");
    assert_eq!(OkxTdMode::Cash.as_str(), "cash");
    assert_eq!(OkxTdMode::SpotIsolated.as_str(), "spot_isolated");
}

#[test]
fn place_order_arg_uses_configured_td_mode() {
    assert_eq!(
        crate::adapters::okx_live::PLACE_ORDER_PATH,
        "/api/v5/trade/order"
    );

    let intent = test_intent(OrderType::Limit);
    let cross = place_order_arg(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .expect("cross body");
    assert_eq!(cross["tdMode"], "cross");

    let isolated = place_order_arg(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Isolated,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .expect("isolated body");
    assert_eq!(isolated["tdMode"], "isolated");
}

#[test]
fn pre_check_order_body_reuses_create_order_shape() {
    let intent = test_intent(OrderType::Limit);
    let body = pre_check_order_body_json(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .expect("pre-check body");
    let value: serde_json::Value = serde_json::from_str(&body).expect("json");

    assert_eq!(value["instId"], "BTC-USDT-SWAP");
    assert_eq!(value["tdMode"], "cross");
    assert_eq!(value["ordType"], "limit");
    assert_eq!(value["clOrdId"], intent.client_order_id);
}

#[test]
fn market_order_uses_ord_type_market_without_px() {
    let mut intent = test_intent(OrderType::Market);
    intent.price = Some(50_000.0);

    let body = place_order_arg(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .expect("market body");

    assert_eq!(body["ordType"], "market");
    assert!(body.get("px").is_none());
}

#[test]
fn futures_payload_omits_unscoped_optional_currency_and_tag_fields() {
    let intent = test_intent(OrderType::Market);
    let rest = place_order_arg(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .expect("rest market body");
    let ws = place_order_ws_arg(
        &intent,
        123_456,
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .expect("ws market body");

    for body in [&rest, &ws] {
        assert!(body.get("tgtCcy").is_none());
        assert!(body.get("tag").is_none());
        assert!(body.get("ccy").is_none());
    }
}

#[test]
fn limit_ioc_and_fok_use_official_ord_type_with_px() {
    let mut ioc = test_intent(OrderType::Limit);
    ioc.time_in_force = TimeInForce::Ioc;
    let ioc_body = place_order_arg(
        &ioc,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&ioc),
    )
    .expect("ioc body");
    assert_eq!(ioc_body["ordType"], "ioc");
    assert_eq!(ioc_body["px"], "50000");

    let mut fok = test_intent(OrderType::Limit);
    fok.time_in_force = TimeInForce::Fok;
    let fok_body = place_order_arg(
        &fok,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&fok),
    )
    .expect("fok body");
    assert_eq!(fok_body["ordType"], "fok");
    assert_eq!(fok_body["px"], "50000");
}

#[test]
fn limit_gtx_uses_official_post_only_ord_type() {
    let mut intent = test_intent(OrderType::Limit);
    intent.time_in_force = TimeInForce::Gtx;
    let body = place_order_arg(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .expect("gtx body");

    assert_eq!(body["ordType"], "post_only");
    assert_eq!(body["px"], "50000");
}

#[test]
fn net_mode_sends_net_pos_side_and_reduce_only() {
    let mut intent = test_intent(OrderType::Limit);
    intent.reduce_only = true;

    let body = place_order_arg(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .expect("net reduce body");

    assert_eq!(body["posSide"], "net");
    assert_eq!(body["reduceOnly"], true);
}

#[test]
fn long_short_mode_maps_open_and_close_pos_side() {
    let open_long = place_order_arg(
        &test_intent(OrderType::Limit),
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::LongShort,
        test_sizing(&test_intent(OrderType::Limit)),
    )
    .expect("open long body");
    assert_eq!(open_long["posSide"], "long");
    assert!(open_long.get("reduceOnly").is_none());

    let mut close_short = test_intent(OrderType::Limit);
    close_short.reduce_only = true;
    let close_short_body = place_order_arg(
        &close_short,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::LongShort,
        test_sizing(&close_short),
    )
    .expect("close short body");
    assert_eq!(close_short_body["posSide"], "short");
    assert!(close_short_body.get("reduceOnly").is_none());
}

#[test]
fn place_order_arg_validates_official_cl_ord_id_rule() {
    let mut intent = test_intent(OrderType::Limit);
    intent.client_order_id = "bad-id".into();
    assert!(place_order_arg(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .is_err());

    intent.client_order_id = "A".repeat(33);
    assert!(place_order_arg(
        &intent,
        "BTC-USDT-SWAP".into(),
        OkxTdMode::Cross,
        OkxPositionMode::Net,
        test_sizing(&intent),
    )
    .is_err());
}

#[test]
fn place_order_arg_validates_quantity_and_limit_price() {
    let mut missing_price = test_intent(OrderType::Limit);
    missing_price.price = None;
    assert!(test_sizing_result(&missing_price).is_err());

    let mut bad_sizing = test_intent(OrderType::Market);
    bad_sizing.quantity = 0.0;
    assert!(test_sizing_result(&bad_sizing).is_err());
}

#[test]
fn cancel_order_arg_uses_client_order_id() {
    let request = CancelOrderRequest {
        exchange: "okx".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: Some("123".into()),
        client_order_id: "cid".into(),
    };
    let body = cancel_order_arg(&request, "BTC-USDT-SWAP".into()).expect("cancel body");
    assert_eq!(body["instId"], "BTC-USDT-SWAP");
    assert_eq!(body["clOrdId"], "cid");
}

fn test_intent(order_type: OrderType) -> OrderIntent {
    OrderIntent {
        id: "i1".into(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn test_sizing(intent: &OrderIntent) -> OkxOrderSizing {
    test_sizing_result(intent).expect("okx test sizing")
}

fn test_sizing_result(intent: &OrderIntent) -> ExchangeResult<OkxOrderSizing> {
    sizing_from_instrument(intent, &test_rule())
}

fn test_rule() -> OkxInstrumentRule {
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
