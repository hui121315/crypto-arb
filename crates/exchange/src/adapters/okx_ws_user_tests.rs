use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;

#[test]
fn login_payload_matches_okx_private_ws_schema() {
    let value: Value = serde_json::from_str(&login_payload(OkxUserWsConfig {
        api_key: "key",
        api_secret: "secret",
        passphrase: "pass",
    }))
    .expect("json");

    assert_eq!(value["op"], "login");
    assert_eq!(value["args"][0]["apiKey"], "key");
    assert_eq!(value["args"][0]["passphrase"], "pass");
    assert!(value["args"][0]["timestamp"]
        .as_str()
        .is_some_and(|s| s.contains('.')));
    assert_eq!(
        value["args"][0]["sign"].as_str().unwrap_or_default().len(),
        44
    );
}

#[test]
fn account_subscription_matches_okx_schema() {
    let value: Value =
        serde_json::from_str(&subscribe_account_payload("1512", Some("usdt"))).expect("json");

    assert_eq!(value["id"], "1512");
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0]["channel"], "account");
    assert_eq!(value["args"][0]["ccy"], "USDT");
}

#[test]
fn positions_and_orders_use_any_inst_type() {
    let positions: Value = serde_json::from_str(&subscribe_positions_payload("p1")).expect("json");
    let orders: Value = serde_json::from_str(&subscribe_orders_payload("o1")).expect("json");

    assert_eq!(positions["args"][0]["channel"], "positions");
    assert_eq!(positions["args"][0]["instType"], "ANY");
    assert_eq!(orders["args"][0]["channel"], "orders");
    assert_eq!(orders["args"][0]["instType"], "ANY");
    assert_eq!(OKX_PRIVATE_WS_URL, "wss://ws.okx.com:8443/ws/v5/private");
    assert_eq!(
        OKX_DEMO_PRIVATE_WS_URL,
        "wss://wspap.okx.com:8443/ws/v5/private"
    );
}

#[test]
fn private_ws_control_fixtures_require_login_and_three_subscription_acks() {
    for (body, expected_channel) in [
        (
            include_str!("../../fixtures/okx/ws_user_login_success.json"),
            "login",
        ),
        (
            include_str!("../../fixtures/okx/ws_user_subscribe_account_success.json"),
            "account",
        ),
        (
            include_str!("../../fixtures/okx/ws_user_subscribe_positions_success.json"),
            "positions",
        ),
        (
            include_str!("../../fixtures/okx/ws_user_subscribe_orders_success.json"),
            "orders",
        ),
    ] {
        assert!(matches!(
            parse_user_control(body).expect("control fixture parses"),
            Some(OkxUserControl::Acknowledged { channel, .. }) if channel == expected_channel
        ));
    }
}

#[test]
fn private_ws_control_rejections_preserve_auth_and_subscription_failures() {
    assert!(matches!(
        parse_user_control(include_str!(
            "../../fixtures/okx/ws_user_login_failure.json"
        ))
        .expect("login failure parses"),
        Some(OkxUserControl::Rejected {
            authentication_failed: true,
            error,
            ..
        }) if error.contains("60009")
    ));
    assert!(matches!(
        parse_user_control(include_str!(
            "../../fixtures/okx/ws_user_subscribe_error.json"
        ))
        .expect("subscription failure parses"),
        Some(OkxUserControl::Rejected {
            channel,
            authentication_failed: false,
            error,
            ..
        }) if channel == "account" && error.contains("60012")
    ));
}

#[test]
fn parses_account_snapshot_details() {
    let event = parse_user_event(include_str!(
        "../../fixtures/okx/ws_user_account_snapshot.json"
    ))
    .expect("parse")
    .expect("event");

    let OkxUserEvent::Account(update) = event else {
        panic!("account event");
    };
    assert_eq!(update.event_type, "snapshot");
    assert!(update.last_page);
    let summary = update.summary.expect("account summary");
    assert_eq!(summary.total_equity_usd, 5000.25);
    assert_eq!(summary.total_available_balance_usd, 4500.50);
    assert_eq!(summary.total_initial_margin_usd, 100.25);
    assert_eq!(summary.total_maintenance_margin_usd, 50.125);
    assert_eq!(summary.updated_time_ms, 1_705_564_213_903);
    assert_eq!(update.balances.len(), 1);
    assert_eq!(update.balances[0].currency, "USDT");
    assert_eq!(update.balances[0].total, 4750.42);
    assert_eq!(update.balances[0].available, 4734.37);
    assert_eq!(update.balances[0].frozen, 158.57);
    assert_eq!(update.balances[0].unrealized_pnl, -1.25);
}

#[test]
fn parses_position_snapshot_rows() {
    let event = parse_user_event(include_str!(
        "../../fixtures/okx/ws_user_positions_snapshot.json"
    ))
    .expect("parse")
    .expect("event");

    let OkxUserEvent::Position(update) = event else {
        panic!("position event");
    };
    assert_eq!(update.positions.len(), 1);
    let row = &update.positions[0];
    assert_eq!(row.symbol, "ETH");
    assert_eq!(row.inst_id, "ETH-USDT-SWAP");
    assert_eq!(row.side, "long");
    assert_eq!(row.quantity, 1.0);
    assert_eq!(row.entry_price, 2566.31);
    assert_eq!(row.mark_price, 2353.849);
    assert_eq!(row.liquidation_price, Some(2352.84));
}

#[test]
fn parses_order_update_rows() {
    let event = parse_user_event(include_str!(
        "../../fixtures/okx/ws_user_orders_partial_fill.json"
    ))
    .expect("parse")
    .expect("event");

    let OkxUserEvent::Order(rows) = event else {
        panic!("order event");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].client_order_id, "okx-client-fill");
    assert_eq!(rows[0].order.order_id, "680800019749904384");
    assert_eq!(rows[0].order.symbol, "BTC");
    assert_eq!(
        rows[0].order.status,
        shared_types::OrderStatus::PartiallyFilled
    );
    assert_eq!(rows[0].order.fees, 0.004);
    assert_eq!(rows[0].updated_time_ms, 1708587373362);
    let fill = rows[0].fill.as_ref().expect("tradeId produces fill");
    assert_eq!(fill.trade_id, "751159184");
    assert_eq!(fill.fill_price, 51_858.0);
    assert_eq!(fill.fill_size, 0.2);
    assert_eq!(fill.fill_fee, Some(0.004));
    assert_eq!(fill.fill_fee_currency.as_deref(), Some("USDT"));
    assert_eq!(fill.fill_time_ms, 1_708_587_373_361);
}

// PR-EK: OKX private WS order/position parsing now reuses the strict shared REST
// parsers, so unrecognized OKX enums fail closed instead of being silently
// coerced (e.g. an unknown order state used to map to Pending -> Accepted).
fn order_event(state: &str, side: &str, ord_type: &str) -> String {
    format!(
        r#"{{"arg":{{"channel":"orders","instType":"ANY"}},"data":[{{"instType":"SWAP","instId":"BTC-USDT-SWAP","ordId":"1","clOrdId":"c","px":"100","sz":"1","ordType":"{ord_type}","side":"{side}","accFillSz":"0","avgPx":"0","state":"{state}","cTime":"1708587373000","uTime":"1708587373362"}}]}}"#
    )
}

#[test]
fn ws_order_update_rejects_unknown_state() {
    let error = parse_user_event(&order_event("frozen", "buy", "limit"))
        .expect_err("unknown order state must fail closed");
    assert!(
        error.to_string().contains("state"),
        "error should name the field: {error}"
    );
}

#[test]
fn ws_order_update_rejects_unknown_side_and_type() {
    assert!(
        parse_user_event(&order_event("live", "long", "limit")).is_err(),
        "unknown side must fail closed"
    );
    assert!(
        parse_user_event(&order_event("live", "buy", "twap")).is_err(),
        "unknown ordType must fail closed"
    );
}

#[test]
fn ws_order_update_accepts_known_enums() {
    let event = parse_user_event(&order_event("partially_filled", "sell", "post_only"))
        .expect("known enums parse")
        .expect("event present");
    let OkxUserEvent::Order(rows) = event else {
        panic!("order event");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].order.status,
        shared_types::OrderStatus::PartiallyFilled
    );
}

#[test]
fn ws_position_update_rejects_unknown_pos_side() {
    let body = r#"{"arg":{"channel":"positions","instType":"ANY"},"data":[{"instType":"SWAP","instId":"ETH-USDT-SWAP","posSide":"sideways","pos":"1","avgPx":"100","markPx":"100","upl":"0","lever":"10","liqPx":"0","margin":"1","imr":"1","mgnRatio":"0.1","uTime":"1619507761462"}]}"#;
    let error = parse_user_event(body).expect_err("unknown posSide must fail closed");
    assert!(
        error.to_string().contains("posSide"),
        "error should name the field: {error}"
    );
}
