use super::*;
use pretty_assertions::assert_eq;
use shared_types::{ExecutionMode, MarginMode, OrderSource, TimeInForce};

#[test]
fn limit_order_matches_official_body_shape() {
    assert_eq!(crate::adapters::kucoin::PLACE_ORDER_PATH, "/api/v1/orders");

    let body = place_order_body_json(
        &intent(OrderSide::Buy, OrderType::Limit),
        "XBTUSDTM".into(),
        0.001,
        "BOTH",
    )
    .expect("body serializes");
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(value["clientOid"], "cid-1");
    assert_eq!(value["symbol"], "XBTUSDTM");
    assert_eq!(value["marginMode"], "ISOLATED");
    assert_eq!(value["positionSide"], "BOTH");
    assert_eq!(value["leverage"], 3);
    assert_eq!(value["side"], "buy");
    assert_eq!(value["type"], "limit");
    assert_eq!(value["size"], 10);
    assert_eq!(value["price"], "50000");
    assert_eq!(value["timeInForce"], "GTC");
}

#[test]
fn limit_ioc_and_fok_preserve_time_in_force() {
    for (time_in_force, expected) in [(TimeInForce::Ioc, "IOC"), (TimeInForce::Fok, "FOK")] {
        let mut order = intent(OrderSide::Buy, OrderType::Limit);
        order.time_in_force = time_in_force;
        let body = place_order_body_json(&order, "XBTUSDTM".into(), 0.001, "BOTH")
            .expect("body serializes");
        let value: serde_json::Value = serde_json::from_str(&body).unwrap();

        assert_eq!(value["type"], "limit");
        assert_eq!(value["price"], "50000");
        assert_eq!(value["timeInForce"], expected);
        assert!(value.get("postOnly").is_none());
    }
}

#[test]
fn limit_gtx_uses_post_only_flag() {
    let mut order = intent(OrderSide::Buy, OrderType::Limit);
    order.time_in_force = TimeInForce::Gtx;
    let body =
        place_order_body_json(&order, "XBTUSDTM".into(), 0.001, "BOTH").expect("body serializes");
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(value["type"], "limit");
    assert_eq!(value["timeInForce"], "GTC");
    assert_eq!(value["postOnly"], true);
}

#[test]
fn market_order_omits_price_and_tif() {
    let mut order = intent(OrderSide::Sell, OrderType::Market);
    order.margin_mode = MarginMode::Cross;
    let body =
        place_order_body_json(&order, "ETHUSDTM".into(), 0.01, "BOTH").expect("body serializes");
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(value["type"], "market");
    assert_eq!(value["size"], 1);
    assert!(value.get("price").is_none());
    assert!(value.get("timeInForce").is_none());
    assert_eq!(value["marginMode"], "CROSS");
}

#[test]
fn position_side_values_follow_official_wire_names() {
    assert_eq!(kucoin_position_side("BOTH").unwrap(), "BOTH");
    assert_eq!(kucoin_position_side("LONG").unwrap(), "LONG");
    assert_eq!(kucoin_position_side("SHORT").unwrap(), "SHORT");
}

#[test]
fn unknown_position_side_is_rejected() {
    let err = place_order_body_json(
        &intent(OrderSide::Buy, OrderType::Limit),
        "XBTUSDTM".into(),
        0.001,
        "UNKNOWN",
    )
    .unwrap_err();

    match err {
        ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "validation");
            assert!(message.contains("positionSide"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn non_integer_leverage_is_rejected() {
    let mut order = intent(OrderSide::Buy, OrderType::Limit);
    order.leverage = 3.5;
    let err = place_order_body_json(&order, "XBTUSDTM".into(), 0.001, "BOTH").unwrap_err();

    match err {
        ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "validation");
            assert!(message.contains("positive integer"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn post_only_sets_post_only_flag() {
    let body = place_order_body_json(
        &intent(OrderSide::Buy, OrderType::PostOnly),
        "XBTUSDTM".into(),
        0.001,
        "BOTH",
    )
    .expect("body serializes");
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(value["type"], "limit");
    assert_eq!(value["timeInForce"], "GTC");
    assert_eq!(value["postOnly"], true);
}

#[test]
fn reduce_only_true_is_serialized() {
    let mut order = intent(OrderSide::Buy, OrderType::Limit);
    order.reduce_only = true;
    let body =
        place_order_body_json(&order, "XBTUSDTM".into(), 0.001, "BOTH").expect("body serializes");
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(value["reduceOnly"], true);
}

#[test]
fn client_oid_is_trimmed_without_unverified_charset_policy() {
    let mut order = intent(OrderSide::Buy, OrderType::Limit);
    order.client_order_id = " client/with space ".into();
    let body =
        place_order_body_json(&order, "XBTUSDTM".into(), 0.001, "BOTH").expect("body serializes");
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();

    assert_eq!(value["clientOid"], "client/with space");
}

#[test]
fn empty_client_oid_is_rejected() {
    let mut order = intent(OrderSide::Buy, OrderType::Limit);
    order.client_order_id = "   ".into();
    let err = place_order_body_json(&order, "XBTUSDTM".into(), 0.001, "BOTH").unwrap_err();

    match err {
        ExchangeError::Api { code, message, .. } => {
            assert_eq!(code, "validation");
            assert!(message.contains("non-empty"));
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn fractional_contract_count_is_rejected() {
    let err = place_order_body_json(
        &intent(OrderSide::Buy, OrderType::Limit),
        "XBTUSDTM".into(),
        0.003,
        "BOTH",
    )
    .unwrap_err();

    match err {
        ExchangeError::Api { code, .. } => assert_eq!(code, "validation"),
        other => panic!("unexpected error: {other:?}"),
    }
}

#[test]
fn get_order_path_uses_official_by_client_oid_query() {
    assert_eq!(
        get_order_by_client_oid_path("cid/a b").expect("path"),
        "/api/v1/orders/byClientOid?clientOid=cid%2Fa+b"
    );
}

#[test]
fn get_order_path_uses_official_order_id_route() {
    assert_eq!(
        get_order_by_order_id_path(" 234125150956625920 ").expect("path"),
        "/api/v1/orders/234125150956625920"
    );
    assert!(get_order_by_order_id_path("cid-1").is_err());
}

#[test]
fn order_ack_uses_client_oid_from_response_when_present() {
    let ack = ack_from_order_row(
        "int-1".into(),
        "fallback".into(),
        KucoinOrderAckRow {
            order_id: "oid-1".into(),
            client_oid: "cid-1".into(),
        },
        LiveOrderState::Accepted,
        None,
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("oid-1"));
    assert_eq!(ack.client_order_id, "fallback");
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("fallback")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(
        ack.identity_update.exchange_order_id.as_deref(),
        Some("oid-1")
    );
}

#[test]
fn ambiguous_place_query_ack_preserves_identity_finality_and_fee() {
    let ack = ack_from_order_query(
        "int-1".into(),
        "public-cid".into(),
        OrderInfo {
            order_id: "oid-1".into(),
            symbol: "BTC".into(),
            exchange: "kucoin".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            status: OrderStatus::Filled,
            quantity: 2.0,
            price: 30_000.0,
            filled_quantity: 2.0,
            filled_price: 29_999.5,
            fees: 0.12,
            created_at: chrono::DateTime::from_timestamp_millis(1).expect("timestamp"),
            execution_style: None,
            venue_time_in_force: None,
            client_order_id: Some("venue-cid".into()),
            reduce_only: Some(false),
        },
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("oid-1"));
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("public-cid")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("venue-cid")
    );
    assert_eq!(ack.state, LiveOrderState::Filled);
    assert_eq!(ack.filled_quantity, Some(2.0));
    assert_eq!(ack.filled_price, Some(29_999.5));
    assert_eq!(ack.filled_fee, Some(0.12));
    assert!(ack
        .message
        .is_some_and(|message| message.contains("byClientOid")));
}

#[test]
fn cancel_ack_prefers_cancelled_order_id() {
    let ack = ack_from_cancel_row(
        "int-1".into(),
        "cid-1".into(),
        Some("fallback".into()),
        KucoinCancelRow {
            cancelled_order_ids: vec!["oid-1".into()],
            client_oid: None,
        },
    );

    assert_eq!(ack.exchange_order_id.as_deref(), Some("oid-1"));
    assert_eq!(ack.client_order_id, "cid-1");
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("cid-1")
    );
    assert_eq!(
        ack.identity_update.exchange_order_id.as_deref(),
        Some("oid-1")
    );
    assert_eq!(ack.state, LiveOrderState::CancelRequested);
}

#[test]
fn kucoin_cancel_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/kucoin/cancel_order_by_client_oid_ack.json");
    let row = serde_json::from_str::<
        crate::adapters::kucoin_response::KucoinResponse<KucoinCancelRow>,
    >(body)
    .expect("official kucoin cancel-order envelope parses")
    .into_data("cancel order")
    .expect("code 200000 yields data row");
    let ack = ack_from_cancel_row(
        "internal-kucoin-cancel-1".to_owned(),
        "public-cid".to_owned(),
        None,
        row,
    );
    assert!(ack.exchange_order_id.is_none());
    assert_eq!(ack.client_order_id, "public-cid");
    assert_eq!(
        ack.identity_update.public_client_order_id.as_deref(),
        Some("public-cid")
    );
    assert_eq!(
        ack.identity_update.venue_client_order_id.as_deref(),
        Some("client-1")
    );
    assert_eq!(ack.state, LiveOrderState::CancelRequested);
    assert!(ack
        .message
        .as_deref()
        .is_some_and(|message| message.contains("final state requires")));
}

fn intent(side: OrderSide, order_type: OrderType) -> OrderIntent {
    OrderIntent {
        id: "int-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "kucoin".into(),
        symbol: "BTC".into(),
        side,
        order_type,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Gtc,
        post_only: false,
        margin_mode: MarginMode::Isolated,
        leverage: 3.0,
        client_order_id: "cid-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

#[test]
fn kucoin_place_order_ack_parses_official_fixture() {
    let body = include_str!("../../fixtures/kucoin/place_order_ack.json");
    let row = serde_json::from_str::<
        crate::adapters::kucoin_response::KucoinResponse<KucoinOrderAckRow>,
    >(body)
    .expect("official kucoin place-order envelope parses")
    .into_data("place order")
    .expect("code 200000 yields data row");
    let ack = ack_from_order_row(
        "internal-kucoin-1".to_owned(),
        "public-cid".to_owned(),
        row,
        LiveOrderState::Accepted,
        None,
    );
    assert_eq!(
        ack.exchange_order_id.as_deref(),
        Some("5bd6e9286d99522a52e458de")
    );
    assert_eq!(ack.state, LiveOrderState::Accepted);
}
