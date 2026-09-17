use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderSide, OrderStatus, OrderType};

#[test]
fn subscribe_payload_matches_gate_authenticated_schema() {
    let cfg = cfg();
    let value: Value =
        serde_json::from_str(&subscribe_orders_payload(cfg, "BTC_USDT").expect("payload"))
            .expect("json");

    assert_eq!(value["channel"], "futures.orders");
    assert_eq!(value["event"], "subscribe");
    assert_eq!(value["payload"][0], "20011");
    assert_eq!(value["payload"][1], "BTC_USDT");
    assert_eq!(value["auth"]["method"], "api_key");
    assert_eq!(value["auth"]["KEY"], "key");
    assert_eq!(
        value["auth"]["SIGN"].as_str().unwrap_or_default().len(),
        128
    );
    assert!(subscribe_orders_payload(GateUserWsConfig { user_id: "", ..cfg }, "BTC_USDT").is_err());
    assert_eq!(private_ws_url(false), GATE_PRIVATE_WS_URL);
    assert_eq!(private_ws_url(true), GATE_TESTNET_PRIVATE_WS_URL);
}

#[test]
fn balances_subscription_uses_only_user_id_payload() {
    let value: Value =
        serde_json::from_str(&subscribe_balances_payload(cfg()).expect("payload")).expect("json");

    assert_eq!(value["channel"], "futures.balances");
    assert_eq!(value["payload"].as_array().map(Vec::len), Some(1));
    assert_eq!(value["payload"][0], "20011");
}

#[test]
fn usertrades_subscription_supports_all_contracts_payload() {
    let value: Value =
        serde_json::from_str(&subscribe_usertrades_payload(cfg(), "!all").expect("payload"))
            .expect("json");

    assert_eq!(value["channel"], "futures.usertrades");
    assert_eq!(value["payload"][0], "20011");
    assert_eq!(value["payload"][1], "!all");
    assert_eq!(value["auth"]["method"], "api_key");
}

#[test]
fn parses_order_position_and_balance_events() {
    assert_order_event(order_event());
    assert_position_event(position_event());
    assert_balance_event(balance_event());
}

#[test]
fn parses_numeric_private_account_rows_from_official_ws_schema() {
    let position = parse_user_event(
        r#"{
            "channel":"futures.positions",
            "event":"update",
            "result":[{
                "contract":"BTC_USDT",
                "size":2,
                "entry_price":40000.36666661111,
                "mark_price":40100.5,
                "unrealised_pnl":1.25,
                "leverage":1,
                "liq_price":0.1,
                "margin":49.999890611186,
                "maintenance_rate":0.005,
                "time_ms":1628736848321
            }]
        }"#,
    )
    .expect("numeric position parse")
    .expect("known position");
    let GateUserEvent::Position(position_rows) = position else {
        panic!("expected position event");
    };
    assert_eq!(position_rows[0].updated_time_ms, 1_628_736_848_321);
    assert_eq!(position_rows[0].entry_price, 40_000.366_666_611_11);

    let balance = parse_user_event(
        r#"{
            "channel":"futures.balances",
            "event":"update",
            "result":[{
                "currency":"USDT",
                "balance":50.744629602494,
                "change":50.744629602494,
                "available":50.744629602494,
                "position_margin":0,
                "order_margin":0,
                "unrealised_pnl":0
            }]
        }"#,
    )
    .expect("numeric balance parse")
    .expect("known balance");
    let GateUserEvent::Balance(balance_rows) = balance else {
        panic!("expected balance event");
    };
    assert_eq!(balance_rows[0].available, 50.744629602494);
}

#[test]
fn parses_usertrade_event_with_fee_evidence() {
    let GateUserEvent::UserTrade(rows) = usertrade_event() else {
        panic!("expected usertrade event");
    };
    let row = &rows[0];

    assert_eq!(row.trade_id, "3335259");
    assert_eq!(row.exchange_order_id, "4872460");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.quantity, 1.0);
    assert_eq!(row.price, 40_000.4);
    assert_eq!(row.fee, 0.0009290592);
    assert_eq!(row.point_fee, 0.0);
    assert_eq!(row.occurred_at_ms, 1_628_736_848_321);
}

#[test]
fn parses_zero_price_ioc_as_market_and_positive_price_ioc_as_limit() {
    let market = order_event_with("0", "ioc");
    let limit = order_event_with("31303.180000", "ioc");

    assert_order_type(market, OrderType::Market);
    assert_order_type(limit, OrderType::Limit);
}

#[test]
fn parses_fok_as_limit_order() {
    assert_order_type(order_event_with("31303.180000", "fok"), OrderType::Limit);
}

#[test]
fn parses_numeric_order_prices_from_live_private_ws() {
    let event = parse_user_event(
        r#"{
            "channel":"futures.orders",
            "event":"update",
            "result":[{
                "id":36028834089796976,
                "contract":"BTC_USDT",
                "status":"finished",
                "finish_as":"filled",
                "size":"1",
                "left":"0",
                "price":62922,
                "fill_price":62909.4,
                "create_time_ms":1785600000000,
                "text":"t-xl-gt-o-test",
                "tif":"fok",
                "is_reduce_only":false
            }]
        }"#,
    )
    .expect("numeric live order prices parse")
    .expect("known order event");
    let GateUserEvent::Order(rows) = event else {
        panic!("expected order event");
    };

    assert_eq!(rows[0].live_state, LiveOrderState::Filled);
    assert_eq!(rows[0].order.price, 62_922.0);
    assert_eq!(rows[0].order.filled_price, 62_909.4);
}

fn order_event() -> GateUserEvent {
    order_event_with("31303.180000", "poc")
}

fn order_event_with(price: &str, tif: &str) -> GateUserEvent {
    let text = format!(
        r#"{{
            "channel":"futures.orders",
            "event":"update",
            "time":1541505434,
            "time_ms":1541505434123,
            "result":[{{
                "id":74046543,
                "contract":"BTC_USDT",
                "status":"finished",
                "finish_as":"filled",
                "size":-100,
                "left":0,
                "price":"{price}",
                "fill_price":"31300",
                "create_time_ms":1541505434123,
                "text":"t-cid-1",
                "tif":"{tif}",
                "is_reduce_only":true
            }}]
        }}"#
    );
    parse_user_event(&text)
        .expect("order parse")
        .expect("known order")
}

fn position_event() -> GateUserEvent {
    parse_user_event(
        r#"{
            "channel":"futures.positions",
            "event":"update",
            "result":[{
                "contract":"BTC_USDT",
                "size":"-2.5",
                "entry_price":"30000",
                "mark_price":"30100",
                "unrealised_pnl":"-2",
                "leverage":"5",
                "liq_price":"40000",
                "margin":"100",
                "maintenance_rate":"0.005",
                "update_time":1541505434123
            }]
        }"#,
    )
    .expect("position parse")
    .expect("known position")
}

fn balance_event() -> GateUserEvent {
    parse_user_event(
        r#"{
            "channel":"futures.balances",
            "event":"update",
            "result":[{
                "currency":"USDT",
                "balance":"9.998739899488",
                "change":"-0.000002074115",
                "available":"8.5",
                "position_margin":"1.0",
                "order_margin":"0.4",
                "unrealised_pnl":"0.1"
            }]
        }"#,
    )
    .expect("balance parse")
    .expect("known balance")
}

fn usertrade_event() -> GateUserEvent {
    parse_user_event(
        r#"{
            "channel":"futures.usertrades",
            "event":"update",
            "time":1543205083,
            "time_ms":1543205083123,
            "result":[{
                "id":"3335259",
                "create_time":1628736848,
                "create_time_ms":1628736848321,
                "contract":"BTC_USDT",
                "order_id":"4872460",
                "size":"1",
                "price":"40000.4",
                "role":"maker",
                "text":"api",
                "fee":0.0009290592,
                "point_fee":0
            }]
        }"#,
    )
    .expect("usertrade parse")
    .expect("known usertrade")
}

fn assert_order_event(event: GateUserEvent) {
    let GateUserEvent::Order(rows) = event else {
        panic!("expected order event");
    };
    let row = &rows[0];
    assert_eq!(row.client_order_id, "t-cid-1");
    assert_eq!(row.order.client_order_id.as_deref(), Some("t-cid-1"));
    assert_eq!(row.order.reduce_only, Some(true));
    assert_eq!(row.live_state, LiveOrderState::Filled);
    assert_eq!(row.order.side, OrderSide::Sell);
    assert_eq!(row.order.order_type, OrderType::PostOnly);
    assert_eq!(row.order.status, OrderStatus::Filled);
    assert_eq!(row.order.filled_quantity, 100.0);
}

fn assert_order_type(event: GateUserEvent, expected: OrderType) {
    let GateUserEvent::Order(rows) = event else {
        panic!("expected order event");
    };
    assert_eq!(rows[0].order.order_type, expected);
}

fn assert_position_event(event: GateUserEvent) {
    let GateUserEvent::Position(rows) = event else {
        panic!("expected position event");
    };
    assert_eq!(rows[0].symbol, "BTC");
    assert_eq!(rows[0].side, "short");
    assert_eq!(rows[0].size, 2.5);
    assert_eq!(rows[0].liquidation_price, Some(40_000.0));
    assert_eq!(rows[0].maintenance_margin_ratio, 0.005);
}

fn assert_balance_event(event: GateUserEvent) {
    let GateUserEvent::Balance(rows) = event else {
        panic!("expected balance event");
    };
    assert_eq!(rows[0].currency, "USDT");
    assert_eq!(rows[0].available, 8.5);
    assert_eq!(rows[0].position_margin + rows[0].order_margin, 1.4);
}

#[test]
fn ignores_non_update_or_unknown_channel() {
    assert!(
        parse_user_event(r#"{"channel":"futures.orders","event":"subscribe","result":{}}"#)
            .expect("valid json")
            .is_none()
    );
    assert!(
        parse_user_event(r#"{"channel":"futures.trades","event":"update","result":[]}"#)
            .expect("valid json")
            .is_none()
    );
}

fn cfg() -> GateUserWsConfig<'static> {
    GateUserWsConfig {
        api_key: "key",
        api_secret: "secret",
        user_id: "20011",
        time_offset_secs: 0,
    }
}

// PR-ER: Gate futures private WS order parsing now fails closed on unrecognized
// enums, matching the strict REST read path (gate_private_data order type/status
// parsers). Previously an unknown order status mapped to Pending (-> Accepted via
// live_state) and an unknown tif defaulted to Limit.
fn gate_order_text(status: &str, finish_as: &str, tif: &str, price: &str) -> String {
    gate_order_text_with_left(status, finish_as, tif, price, "100")
}

fn gate_order_text_with_left(
    status: &str,
    finish_as: &str,
    tif: &str,
    price: &str,
    left: &str,
) -> String {
    format!(
        r#"{{"channel":"futures.orders","event":"update","time_ms":1541505434123,"result":[{{"id":74046543,"contract":"BTC_USDT","status":"{status}","finish_as":"{finish_as}","size":-100,"left":{left},"price":"{price}","fill_price":"31300","create_time_ms":1541505434123,"text":"t-cid-1","tif":"{tif}","is_reduce_only":true}}]}}"#
    )
}

#[test]
fn ws_order_update_rejects_unknown_status() {
    let error = parse_user_event(&gate_order_text("weird", "filled", "gtc", "100"))
        .expect_err("unknown status must fail closed");
    assert!(
        error.to_string().contains("status"),
        "error should name the field: {error}"
    );
}

#[test]
fn ws_order_update_rejects_unknown_tif() {
    let error = parse_user_event(&gate_order_text("open", "", "weird", "100"))
        .expect_err("unknown tif must fail closed");
    assert!(
        error.to_string().contains("tif"),
        "error should name the field: {error}"
    );
}

#[test]
fn ws_order_update_accepts_known_open_gtc() {
    let event = parse_user_event(&gate_order_text("open", "", "gtc", "100"))
        .expect("known enums parse")
        .expect("event present");
    let GateUserEvent::Order(rows) = event else {
        panic!("expected order event");
    };
    assert_eq!(rows[0].order.status, OrderStatus::Open);
    assert_eq!(rows[0].order.order_type, OrderType::Limit);
}

#[test]
fn ws_order_update_maps_open_left_to_partially_filled() {
    let event = parse_user_event(&gate_order_text_with_left("open", "", "gtc", "100", "40"))
        .expect("partial order parses")
        .expect("event present");
    let GateUserEvent::Order(rows) = event else {
        panic!("expected order event");
    };
    assert_eq!(rows[0].order.status, OrderStatus::PartiallyFilled);
    assert_eq!(rows[0].live_state, LiveOrderState::PartiallyFilled);
    assert_eq!(rows[0].order.filled_quantity, 60.0);
}

#[test]
fn ws_position_update_rejects_negative_maintenance_rate() {
    let error = parse_user_event(
        r#"{
            "channel":"futures.positions",
            "event":"update",
            "result":[{
                "contract":"BTC_USDT",
                "size":"1",
                "entry_price":"30000",
                "mark_price":"30100",
                "unrealised_pnl":"1",
                "leverage":"5",
                "liq_price":"20000",
                "margin":"100",
                "maintenance_rate":"-0.01",
                "update_time":1541505434123
            }]
        }"#,
    )
    .expect_err("negative maintenance rate must fail closed");

    assert!(error.to_string().contains("maintenance_rate"));
}
