use super::*;
use pretty_assertions::assert_eq;
use shared_types::{LiveOrderState, OrderSide, OrderStatus, OrderType};

#[test]
fn parses_account_update_without_inventing_shared_balance_fields() {
    let event = parse_user_event(
        r#"{
            "e":"ACCOUNT_UPDATE",
            "E":1564745798939,
            "T":1564745798938,
            "a":{
                "m":"ORDER",
                "B":[{"a":"USDT","wb":"122624.12345678","cw":"100.12345678","bc":"50.12345678"}],
                "P":[{"s":"BTCUSDT","pa":"0.001","ep":"50000","cr":"1.23","up":"0.45","mt":"cross","iw":"0","ps":"LONG"}]
            }
        }"#,
    )
    .expect("account event parses")
    .expect("account event");

    let BinanceUserEvent::Account(update) = event else {
        panic!("expected account event");
    };
    assert_eq!(update.event_time_ms, 1_564_745_798_939);
    assert_eq!(update.transaction_time_ms, 1_564_745_798_938);
    assert_eq!(update.reason, "ORDER");
    assert_eq!(update.balances[0].asset, "USDT");
    assert_eq!(update.balances[0].wallet_balance, 122_624.123_456_78);
    assert_eq!(update.positions[0].symbol, "BTC");
    assert_eq!(update.positions[0].side, "LONG");
    assert_eq!(update.positions[0].quantity, 0.001);
}

#[test]
fn parses_order_trade_update_to_order_delta() -> Result<(), String> {
    let event = parse_user_event(include_str!(
        "../../fixtures/binance/usdm_order_trade_update_partial_fill.json"
    ))
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "missing order event".to_owned())?;
    let BinanceUserEvent::Order(update) = event else {
        return Err("expected order event".to_owned());
    };

    assert_order_update(&update);
    Ok(())
}

#[test]
fn parses_order_trade_update_filled_fixture_to_terminal_delta() -> Result<(), String> {
    let update = parsed_filled_order_update()?;

    assert_filled_update_envelope(&update);
    assert_filled_order_info(&update.order);
    Ok(())
}

fn parsed_filled_order_update() -> Result<BinanceOrderTradeUpdate, String> {
    let event = parse_user_event(include_str!(
        "../../fixtures/binance/usdm_order_trade_update_filled.json"
    ))
    .map_err(|error| error.to_string())?
    .ok_or_else(|| "missing order event".to_owned())?;
    let BinanceUserEvent::Order(update) = event else {
        return Err("expected order event".to_owned());
    };

    Ok(*update)
}

fn assert_filled_update_envelope(update: &BinanceOrderTradeUpdate) {
    assert_eq!(update.client_order_id, "cid-1");
    assert_eq!(update.execution_type, "TRADE");
    assert_eq!(update.order_status, "FILLED");
    assert_eq!(update.trade_id, Some(1_234_568));
    assert_eq!(update.last_filled_quantity, 0.007);
    assert_eq!(update.last_filled_price, 50_008.0);
    assert_eq!(update.commission_asset.as_deref(), Some("USDT"));
    assert_eq!(update.live_state, LiveOrderState::Filled);
    assert!(update.is_terminal());
}

fn assert_filled_order_info(order: &OrderInfo) {
    assert_eq!(order.order_id, "8886774");
    assert_eq!(order.status, OrderStatus::Filled);
    assert_eq!(order.quantity, 0.01);
    assert_eq!(order.filled_quantity, 0.01);
    assert_eq!(order.filled_price, 50_008.0);
    assert_eq!(order.fees, 0.28);
    assert_eq!(order.client_order_id.as_deref(), Some("cid-1"));
}

fn assert_order_update(update: &BinanceOrderTradeUpdate) {
    assert_eq!(update.event_time_ms, 1_568_879_465_651);
    assert_eq!(update.transaction_time_ms, 1_568_879_465_650);
    assert_eq!(update.trade_time_ms, 1_568_879_465_652);
    assert_eq!(update.client_order_id, "cid-1");
    assert_eq!(update.execution_type, "TRADE");
    assert_eq!(update.order_status, "PARTIALLY_FILLED");
    assert_eq!(update.reject_reason.as_deref(), Some("NONE"));
    assert_eq!(update.trade_id, Some(1_234_567));
    assert_eq!(update.last_filled_quantity, 0.003);
    assert_eq!(update.last_filled_price, 50_010.0);
    assert_eq!(update.commission_asset.as_deref(), Some("USDT"));
    assert_eq!(update.live_state, LiveOrderState::PartiallyFilled);
    assert!(!update.is_terminal());
    assert_order_info(&update.order);
}

fn assert_order_info(order: &OrderInfo) {
    assert_eq!(order.order_id, "8886774");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.side, OrderSide::Buy);
    assert_eq!(order.order_type, OrderType::PostOnly);
    assert_eq!(order.status, OrderStatus::PartiallyFilled);
    assert_eq!(order.quantity, 0.01);
    assert_eq!(order.price, 50_000.0);
    assert_eq!(order.filled_quantity, 0.003);
    assert_eq!(order.filled_price, 50_010.0);
    assert_eq!(order.fees, 0.12);
    assert_eq!(order.client_order_id.as_deref(), Some("cid-1"));
    assert_eq!(order.reduce_only, Some(true));
}

#[test]
fn ignores_non_user_stream_events() {
    let event = parse_user_event(r#"{"e":"listenKeyExpired","E":1}"#)
        .expect("valid json")
        .is_none();
    assert!(event);
}

#[test]
fn rejects_unknown_order_enums_without_defaulting() {
    assert_parse_error(
        &order_event(OrderEventFixture {
            side: "HOLD",
            ..OrderEventFixture::default()
        }),
        "order side",
    );
    assert_parse_error(
        &order_event(OrderEventFixture {
            order_type: "STOP_MARKET",
            ..OrderEventFixture::default()
        }),
        "order type",
    );
    assert_parse_error(
        &order_event(OrderEventFixture {
            order_status: "MYSTERY",
            ..OrderEventFixture::default()
        }),
        "order status",
    );
    assert_parse_error(
        &order_event(OrderEventFixture {
            execution_type: "UNKNOWN",
            ..OrderEventFixture::default()
        }),
        "execution type",
    );
}

#[test]
fn rejects_missing_or_invalid_required_order_fields() {
    assert_parse_error(
        &order_event(OrderEventFixture {
            event_time_ms: 0,
            ..OrderEventFixture::default()
        }),
        "timestamp field E",
    );
    assert_parse_error(
        &order_event(OrderEventFixture {
            transaction_time_ms: 0,
            ..OrderEventFixture::default()
        }),
        "timestamp field T",
    );
    assert_parse_error(
        &order_event(OrderEventFixture {
            order_id: 0,
            ..OrderEventFixture::default()
        }),
        "order id",
    );
    assert_parse_error(
        &order_event(OrderEventFixture {
            accumulated_filled_quantity: "NaN",
            ..OrderEventFixture::default()
        }),
        "non-finite",
    );
    assert_parse_error(
        &order_event(OrderEventFixture {
            trade_id: 0,
            ..OrderEventFixture::default()
        }),
        "incoherent TRADE fill",
    );
    assert_parse_error(
        &order_event(OrderEventFixture {
            last_filled_quantity: "0.004",
            accumulated_filled_quantity: "0.003",
            ..OrderEventFixture::default()
        }),
        "incoherent TRADE fill",
    );
}

#[test]
fn order_update_allows_missing_reject_reason() {
    let mut payload: serde_json::Value =
        serde_json::from_str(&order_event(OrderEventFixture::default()))
            .expect("valid fixture json");
    payload["o"]
        .as_object_mut()
        .expect("order object")
        .remove("r");

    let event = parse_user_event(&payload.to_string())
        .expect("missing optional reject reason parses")
        .expect("order event");
    let BinanceUserEvent::Order(update) = event else {
        panic!("expected order event");
    };
    assert_eq!(update.reject_reason, None);
}

#[test]
fn terminal_expiry_preserves_reject_reason_and_finality() {
    let event = parse_user_event(&order_event(OrderEventFixture {
        execution_type: "EXPIRED",
        order_status: "EXPIRED_IN_MATCH",
        trade_id: 0,
        last_filled_quantity: "0",
        accumulated_filled_quantity: "0",
        reject_reason: "8",
        ..OrderEventFixture::default()
    }))
    .expect("expiry parses")
    .expect("order event");
    let BinanceUserEvent::Order(update) = event else {
        panic!("expected order event");
    };

    assert_eq!(update.order_status, "EXPIRED_IN_MATCH");
    assert_eq!(update.reject_reason.as_deref(), Some("8"));
    assert_eq!(update.live_state, LiveOrderState::Failed);
    assert!(update.is_terminal());
    assert_eq!(update.trade_id, None);
}

#[test]
fn rejects_bad_account_update_fields_without_zero_fallback() {
    assert_parse_error(
        &account_event(AccountEventFixture {
            wallet_balance: "bad",
            ..AccountEventFixture::default()
        }),
        "a.B.wb",
    );
    assert_parse_error(
        &account_event(AccountEventFixture {
            position_side: "SIDEWAYS",
            ..AccountEventFixture::default()
        }),
        "position side",
    );
    assert_parse_error(
        &account_event(AccountEventFixture {
            position_amount: "",
            ..AccountEventFixture::default()
        }),
        "a.P.pa",
    );
}

#[test]
fn user_stream_url_uses_private_ws_base() {
    let url = user_stream_ws_url(" listen-key-1 ").expect("url");
    let parsed = url::Url::parse(&url).expect("valid websocket url");
    let query = parsed
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();

    assert_eq!(parsed.scheme(), "wss");
    assert_eq!(parsed.host_str(), Some("fstream.binance.com"));
    assert_eq!(parsed.path(), "/private/ws");
    assert_eq!(
        query.get("listenKey").map(|value| value.as_ref()),
        Some("listen-key-1")
    );
    assert_eq!(
        query.get("events").map(|value| value.as_ref()),
        Some("ORDER_TRADE_UPDATE/ACCOUNT_UPDATE")
    );

    let custom = user_stream_ws_url_with_base("wss://example.test/private/", "k").expect("url");
    let custom = url::Url::parse(&custom).expect("custom websocket url");
    assert_eq!(custom.path(), "/private/ws");
    assert_eq!(
        custom
            .query_pairs()
            .find(|(name, _)| name == "listenKey")
            .map(|(_, value)| value.into_owned()),
        Some("k".to_owned())
    );
    assert!(user_stream_ws_url(" ").is_err());
    assert_eq!(USER_STREAM_KEEPALIVE_INTERVAL_SECS, 1_800);
    assert_eq!(USER_STREAM_CONNECTION_MAX_SECS, 82_800);
}

#[derive(Clone, Copy)]
struct OrderEventFixture<'a> {
    event_time_ms: i64,
    transaction_time_ms: i64,
    side: &'a str,
    order_type: &'a str,
    execution_type: &'a str,
    order_status: &'a str,
    order_id: i64,
    trade_id: i64,
    last_filled_quantity: &'a str,
    accumulated_filled_quantity: &'a str,
    reject_reason: &'a str,
}

impl Default for OrderEventFixture<'_> {
    fn default() -> Self {
        Self {
            event_time_ms: 1_568_879_465_651,
            transaction_time_ms: 1_568_879_465_650,
            side: "BUY",
            order_type: "LIMIT",
            execution_type: "TRADE",
            order_status: "PARTIALLY_FILLED",
            order_id: 8_886_774,
            trade_id: 1_234_567,
            last_filled_quantity: "0.003",
            accumulated_filled_quantity: "0.003",
            reject_reason: "NONE",
        }
    }
}

fn order_event(fixture: OrderEventFixture<'_>) -> String {
    format!(
        r#"{{
        "e":"ORDER_TRADE_UPDATE",
        "E":{},
        "T":{},
        "o":{{
            "s":"BTCUSDT",
            "c":"cid-1",
            "S":"{}",
            "o":"{}",
            "f":"GTX",
            "q":"0.01",
            "p":"50000",
            "ap":"50010",
            "x":"{}",
            "X":"{}",
            "i":{},
            "l":"{}",
            "z":"{}",
            "L":"50010",
            "N":"USDT",
            "n":"0.12",
            "r":"{}",
            "t":{},
            "T":1568879465652
        }}
    }}"#,
        fixture.event_time_ms,
        fixture.transaction_time_ms,
        fixture.side,
        fixture.order_type,
        fixture.execution_type,
        fixture.order_status,
        fixture.order_id,
        fixture.last_filled_quantity,
        fixture.accumulated_filled_quantity,
        fixture.reject_reason,
        fixture.trade_id
    )
}

#[derive(Clone, Copy)]
struct AccountEventFixture<'a> {
    wallet_balance: &'a str,
    position_amount: &'a str,
    position_side: &'a str,
}

impl Default for AccountEventFixture<'_> {
    fn default() -> Self {
        Self {
            wallet_balance: "122624.12345678",
            position_amount: "0.001",
            position_side: "LONG",
        }
    }
}

fn account_event(fixture: AccountEventFixture<'_>) -> String {
    format!(
        r#"{{
        "e":"ACCOUNT_UPDATE",
        "E":1564745798939,
        "T":1564745798938,
        "a":{{
            "m":"ORDER",
            "B":[{{"a":"USDT","wb":"{}","cw":"100.12345678","bc":"50.12345678"}}],
            "P":[{{"s":"BTCUSDT","pa":"{}","ep":"50000","cr":"1.23","up":"0.45","mt":"cross","iw":"0","ps":"{}"}}]
        }}
    }}"#,
        fixture.wallet_balance, fixture.position_amount, fixture.position_side
    )
}

fn assert_parse_error(payload: &str, needle: &str) {
    let error = parse_user_event(payload).expect_err("payload should fail closed");
    assert!(
        error.to_string().contains(needle),
        "error {error:?} should contain {needle}"
    );
}
