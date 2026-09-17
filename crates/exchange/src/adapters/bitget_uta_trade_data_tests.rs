use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;
use shared_types::{ExecutionMode, MarginMode, OrderSource, TimeInForce};

#[test]
fn place_order_body_uses_time_in_force_and_v3_category() {
    assert_eq!(
        crate::adapters::bitget::PLACE_ORDER_PATH,
        "/api/v3/trade/place-order"
    );

    let json = place_order_body_json(
        &intent(OrderType::Limit),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Crossed,
    )
    .expect("rest body serializes");
    let value: Value = serde_json::from_str(&json).expect("json parses");
    // V3 keys: category (replaces V2 productType), qty (replaces V2 size),
    // and timeInForce (replaces V2 force).
    assert_eq!(value["category"], "USDT-FUTURES");
    assert!(value.get("productType").is_none());
    assert_eq!(value["qty"], "0.01");
    assert!(value.get("size").is_none());
    assert_eq!(value["symbol"], "BTCUSDT");
    assert!(value.get("marginCoin").is_none());
    assert_eq!(value["marginMode"], "crossed");
    assert_eq!(value["side"], "buy");
    assert_eq!(value["orderType"], "limit");
    assert_eq!(value["price"], "50000");
    assert_eq!(value["timeInForce"], "ioc");
    assert!(value.get("force").is_none());
    assert_eq!(value["clientOid"], "cid-1");
}

#[test]
fn limit_time_in_force_variants_preserve_time_in_force() {
    for (time_in_force, expected) in [(TimeInForce::Gtc, "gtc"), (TimeInForce::Fok, "fok")] {
        let mut order = intent(OrderType::Limit);
        order.time_in_force = time_in_force;
        let json = place_order_body_json(&order, "BTCUSDT".to_owned(), BitgetMarginMode::Crossed)
            .expect("rest body serializes");
        let value: Value = serde_json::from_str(&json).expect("json parses");

        assert_eq!(value["orderType"], "limit");
        assert_eq!(value["timeInForce"], expected);
        assert!(value.get("force").is_none());
    }
}

#[test]
fn limit_gtx_uses_post_only_time_in_force() {
    let mut order = intent(OrderType::Limit);
    order.time_in_force = TimeInForce::Gtx;
    let json = place_order_body_json(&order, "BTCUSDT".to_owned(), BitgetMarginMode::Crossed)
        .expect("rest body serializes");
    let value: Value = serde_json::from_str(&json).expect("json parses");

    assert_eq!(value["orderType"], "limit");
    assert_eq!(value["timeInForce"], "post_only");
}

#[test]
fn market_order_omits_price_and_time_in_force() {
    let json = place_order_body_json(
        &intent(OrderType::Market),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Crossed,
    )
    .expect("market body");
    let value: Value = serde_json::from_str(&json).expect("json parses");
    assert_eq!(value["orderType"], "market");
    assert!(value.get("price").is_none());
    assert!(value.get("timeInForce").is_none());
    assert!(value.get("force").is_none());
}

#[test]
fn post_only_uses_post_only_time_in_force() {
    let json = place_order_body_json(
        &intent(OrderType::PostOnly),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Crossed,
    )
    .expect("post-only body");
    let value: Value = serde_json::from_str(&json).expect("json parses");
    assert_eq!(value["orderType"], "limit");
    assert_eq!(value["timeInForce"], "post_only");
}

#[test]
fn isolated_margin_mode_is_serialized() {
    let json = place_order_body_json(
        &intent(OrderType::Limit),
        "BTCUSDT".to_owned(),
        BitgetMarginMode::Isolated,
    )
    .expect("isolated body");
    let value: Value = serde_json::from_str(&json).expect("json parses");
    assert_eq!(value["marginMode"], "isolated");
}

#[test]
fn reduce_only_intent_serializes_yes() {
    let mut order = intent(OrderType::Market);
    order.reduce_only = true;
    let json = place_order_body_json(&order, "BTCUSDT".to_owned(), BitgetMarginMode::Crossed)
        .expect("reduce-only body");
    let value: Value = serde_json::from_str(&json).expect("json parses");
    assert_eq!(value["reduceOnly"], "yes");
}

#[test]
fn rejects_client_oid_outside_official_policy() {
    let mut order = intent(OrderType::Limit);
    order.client_order_id = "client id too long and has spaces".into();
    let result = place_order_body_json(&order, "BTCUSDT".to_owned(), BitgetMarginMode::Crossed);

    assert!(matches!(
        result,
        Err(ExchangeError::Api { code, .. }) if code == "validation"
    ));
}

#[test]
fn cancel_body_carries_category_and_optional_order_id() {
    let request = CancelOrderRequest {
        exchange: "bitget".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: Some("oid-1".into()),
        client_order_id: "cid-1".into(),
    };
    let json =
        cancel_order_body_json(&request, "BTCUSDT".to_owned()).expect("cancel body serializes");
    let value: Value = serde_json::from_str(&json).expect("json parses");
    assert_eq!(value["category"], "USDT-FUTURES");
    assert_eq!(value["symbol"], "BTCUSDT");
    assert_eq!(value["orderId"], "oid-1");
    assert_eq!(value["clientOid"], "cid-1");
}

#[test]
fn cancel_body_omits_order_id_when_client_oid_only() {
    let request = CancelOrderRequest {
        exchange: "bitget".into(),
        symbol: "BTC".into(),
        internal_order_id: "i1".into(),
        exchange_order_id: None,
        client_order_id: "cid-1".into(),
    };
    let json = cancel_order_body_json(&request, "BTCUSDT".to_owned()).expect("cancel body");
    let value: Value = serde_json::from_str(&json).expect("json parses");
    assert!(value.get("orderId").is_none());
    assert_eq!(value["clientOid"], "cid-1");
}

#[test]
fn ack_reuses_fallback_client_id_when_missing() {
    let row = UtaOrderAckRow {
        order_id: "oid-1".into(),
        client_oid: String::new(),
    };
    let ack = ack_from_row(
        "i1".into(),
        "fallback".into(),
        row,
        LiveOrderState::Accepted,
        None,
    );
    assert_eq!(ack.exchange_order_id.as_deref(), Some("oid-1"));
    assert_eq!(ack.client_order_id, "fallback");
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("fallback")
    );
    assert!(matches!(ack.state, LiveOrderState::Accepted));
}

#[test]
fn place_order_invalid_quantity_returns_error() {
    let mut order = intent(OrderType::Market);
    order.quantity = 0.0;
    let result = place_order_body_json(&order, "BTCUSDT".to_owned(), BitgetMarginMode::Crossed);
    assert!(matches!(
        result,
        Err(ExchangeError::Api { code, .. }) if code == "validation"
    ));
}

#[test]
fn compiled_usdc_hedge_order_preserves_native_category_symbol_and_pos_side() {
    let order = intent(OrderType::Limit);
    let compiled = CompiledBitgetOrder {
        category: BitgetUtaCategory::UsdcFutures,
        native_symbol: "BTCPERP".to_owned(),
        quantity: order.quantity,
        pos_side: Some("long"),
        reduce_only: false,
    };
    let json = place_order_body_json(&order, &compiled, BitgetMarginMode::Crossed)
        .expect("USDC hedge body");
    let body: serde_json::Value = serde_json::from_str(&json).expect("body json");
    assert_eq!(body["category"], "USDC-FUTURES");
    assert_eq!(body["symbol"], "BTCPERP");
    assert_eq!(body["posSide"], "long");
    assert!(body.get("reduceOnly").is_none());
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

#[test]
fn bitget_place_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/bitget/uta_place_order_ack.json");
    let row = serde_json::from_str::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaOrderAckRow>,
    >(body)
    .expect("official bitget place-order envelope parses")
    .into_result("place order")
    .expect("code 00000 yields data row");
    let ack = ack_from_row(
        "internal-bitget-1".to_owned(),
        "public-cid".to_owned(),
        row,
        LiveOrderState::Accepted,
        None,
    );
    assert_eq!(
        ack.exchange_order_id.as_deref(),
        Some("1217143186968068096")
    );
    assert_eq!(ack.state, LiveOrderState::Accepted);
}

#[test]
fn bitget_cancel_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/bitget/uta_cancel_order_ack.json");
    let row = serde_json::from_str::<
        crate::adapters::bitget_response::BitgetObjectResponse<UtaOrderAckRow>,
    >(body)
    .expect("official bitget cancel-order envelope parses")
    .into_result("cancel order")
    .expect("code 00000 yields data row");
    let ack = ack_from_row(
        "internal-bitget-cancel-1".to_owned(),
        "public-cid".to_owned(),
        row,
        LiveOrderState::CancelRequested,
        Some("bitget cancel request accepted; final state requires order stream or query".into()),
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("111111111111111111"));
    assert_eq!(ack.client_order_id, "public-cid");
    assert_eq!(ack.state, LiveOrderState::CancelRequested);
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("public-cid")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("bitget-cli-7")
    );
    assert_eq!(
        ack.identity_update.exchange_order_id.as_deref(),
        Some("111111111111111111")
    );
    assert_eq!(
        ack.message.as_deref(),
        Some("bitget cancel request accepted; final state requires order stream or query")
    );
}
