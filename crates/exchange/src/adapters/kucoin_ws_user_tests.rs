use super::*;
use pretty_assertions::assert_eq;
use serde_json::{json, Value};
use shared_types::{LiveOrderState, OrderSide, OrderStatus, OrderType};

#[test]
fn subscribe_payloads_match_futures_private_topics() {
    let order: Value =
        serde_json::from_str(&subscribe_orders_payload("1").expect("order sub")).expect("json");
    let positions: Value =
        serde_json::from_str(&subscribe_positions_payload("positions").expect("positions sub"))
            .expect("json");
    let position: Value =
        serde_json::from_str(&subscribe_position_payload("2", "BTC").expect("pos sub"))
            .expect("json");

    assert_eq!(order["type"], "subscribe");
    assert_eq!(order["topic"], "/contractMarket/tradeOrders");
    assert_eq!(order["privateChannel"], true);
    assert_eq!(positions["type"], "subscribe");
    assert_eq!(positions["topic"], "/contract/positionAll");
    assert_eq!(positions["privateChannel"], true);
    assert_eq!(positions["response"], true);
    assert_eq!(position["topic"], "/contract/position:XBTUSDTM");
    assert_eq!(KUCOIN_FUTURES_PRIVATE_BULLET_PATH, "/api/v1/bullet-private");
}

#[test]
fn pro_connect_url_uses_official_query_auth_fields() {
    let cfg = KucoinUserWsConfig {
        api_key: "key",
        api_secret: "secret",
        passphrase: "pass",
        time_offset_ms: 0,
    };
    let url = pro_connect_url(cfg, true);
    let default_precision_url = pro_connect_url(cfg, false);

    assert!(url.starts_with(KUCOIN_PRO_PRIVATE_WS_BASE));
    assert!(url.contains("apikey=key"));
    assert!(url.contains("sign="));
    assert!(url.contains("passphrase="));
    assert!(url.contains("timestamp="));
    assert!(url.contains("enable_ns=true"));
    assert!(!url.contains("enableNewBulletin"));
    assert!(!default_precision_url.contains("enable_ns"));
    assert_eq!(
        pro_challenge_signature("secret", r#"{"sessionId":"s"}"#).len(),
        44
    );
}

#[test]
fn pro_ws_live_submit_requires_authenticated_runtime_evidence() {
    let missing = pro_ws_live_submit_blockers(None);
    assert_eq!(missing, vec![KUCOIN_PRO_WS_RUNTIME_EVIDENCE_BLOCKER]);

    let complete = pro_ws_live_submit_blockers(Some(KucoinProWsRuntimeEvidence {
        authenticated_session: true,
        live_place_ack: true,
        live_cancel_ack: true,
        order_finality: true,
    }));
    assert!(complete.is_empty());
}

#[test]
fn pro_order_and_cancel_payloads_use_official_ops() {
    let order: Value = serde_json::from_str(
        &pro_futures_order_payload("o1", json!({"clientOid":"cid","symbol":"XBTUSDTM"}))
            .expect("order payload"),
    )
    .expect("json");
    let cancel: Value = serde_json::from_str(
        &pro_futures_cancel_payload("c1", json!({"clientOid":"cid","symbol":"XBTUSDTM"}))
            .expect("cancel payload"),
    )
    .expect("json");

    assert_eq!(order["op"], "futures.order");
    assert_eq!(cancel["op"], "futures.cancel");
}

#[test]
fn parses_order_balance_position_and_pro_ack() {
    assert_order_event(order_event());
    assert_balance_event(balance_event());
    assert_position_event(position_event());
    assert_position_event(position_all_event());
    assert_pro_ack(
        include_str!("../../fixtures/kucoin/wsapi_pro_order_ack.json"),
        "o1",
    );
    assert_pro_ack(
        include_str!("../../fixtures/kucoin/wsapi_pro_cancel_ack.json"),
        "c1",
    );
}

fn order_event() -> KucoinUserEvent {
    parse_user_event(include_str!(
        "../../fixtures/kucoin/classic_ws_trade_orders_match.json"
    ))
    .expect("order parse")
    .expect("known order")
}

fn balance_event() -> KucoinUserEvent {
    parse_user_event(
        r#"{
            "type":"message",
            "topic":"/contractAccount/wallet",
            "subject":"walletBalance.change",
            "data":{
                "currency":"USDT",
                "equity":"10.5",
                "availableBalance":"8.5",
                "holdBalance":"2",
                "crossUnPnl":"0.4",
                "isolatedUnPnl":"0.1",
                "timestamp":1593487482038
            }
        }"#,
    )
    .expect("balance parse")
    .expect("known balance")
}

fn position_event() -> KucoinUserEvent {
    parse_user_event(
        r#"{
            "type":"message",
            "topic":"/contract/position:XBTUSDTM",
            "subject":"position.change",
            "data":{
                "symbol":"XBTUSDTM",
                "currentQty":-3,
                "avgEntryPrice":"50000",
                "markPrice":"50100",
                "unrealisedPnl":"-1.2",
                "liquidationPrice":"60000",
                "leverage":"5",
                "posMargin":"100",
                "maintMarginReq":"0.004",
                "marginMode":"CROSS",
                "isOpen":true,
                "currentTimestamp":1593487482038
            }
        }"#,
    )
    .expect("position parse")
    .expect("known position")
}

fn position_all_event() -> KucoinUserEvent {
    parse_user_event(
        r#"{
            "type":"message",
            "topic":"/contract/positionAll",
            "subject":"position.change",
            "data":{
                "symbol":"XBTUSDTM",
                "currentQty":-3,
                "avgEntryPrice":"50000",
                "markPrice":"50100",
                "unrealisedPnl":"-1.2",
                "liquidationPrice":"60000",
                "leverage":"5",
                "posMargin":"100",
                "maintMarginReq":"0.004",
                "marginMode":"CROSS",
                "isOpen":true,
                "currentTimestamp":1593487482038
            }
        }"#,
    )
    .expect("position all parse")
    .expect("known position all")
}

fn assert_order_event(event: KucoinUserEvent) {
    let KucoinUserEvent::Order(row) = event else {
        panic!("expected order");
    };
    assert_order_identity(&row);
    assert_order_execution(&row);
    assert_match_fill(row.fill.as_ref().expect("match fill evidence"));
}

fn assert_order_identity(row: &KucoinOrderUpdate) {
    assert_eq!(row.client_order_id, "");
    assert_eq!(row.order.client_order_id, None);
    assert_eq!(row.order.reduce_only, None);
    assert_eq!(row.live_state, LiveOrderState::PartiallyFilled);
    assert_eq!(row.event_type, "match");
    assert!(!row.terminal);
    assert_eq!(row.event_time_ms, 1_731_916_996_762);
    assert_eq!(row.order.symbol, "BTC");
    assert_eq!(row.order.side, OrderSide::Buy);
}

fn assert_order_execution(row: &KucoinOrderUpdate) {
    assert_eq!(row.order.order_type, OrderType::Limit);
    assert_eq!(row.order.status, OrderStatus::PartiallyFilled);
    assert_eq!(row.order.filled_quantity, 1.0);
    assert_eq!(row.order.fees, 0.0);
}

fn assert_match_fill(fill: &KucoinFillUpdate) {
    assert_eq!(fill.exchange_order_id, "247899236673269761");
    assert_eq!(fill.trade_id, "1794175373644");
    assert_eq!(
        fill.venue_event_id,
        "kucoin_match:247899236673269761:1794175373644"
    );
    assert_eq!(fill.fee_type, "makerFee");
    assert_eq!(fill.liquidity, "maker");
    assert_eq!(fill.fee_amount, None);
    assert_eq!(fill.fee_currency, None);
    assert_eq!(fill.quantity, 1.0);
    assert_eq!(fill.price, 91_670.0);
}

#[test]
fn official_terminal_fixtures_require_terminal_event_type() {
    let filled = parse_order_fixture(include_str!(
        "../../fixtures/kucoin/classic_ws_trade_orders_filled.json"
    ));
    let canceled = parse_order_fixture(include_str!(
        "../../fixtures/kucoin/classic_ws_trade_orders_canceled.json"
    ));

    assert_eq!(filled.event_type, "filled");
    assert!(filled.terminal);
    assert_eq!(filled.live_state, LiveOrderState::Filled);
    assert_eq!(filled.order.status, OrderStatus::Filled);
    assert!(filled.fill.is_none());
    assert_eq!(canceled.event_type, "canceled");
    assert!(canceled.terminal);
    assert_eq!(canceled.live_state, LiveOrderState::Cancelled);
    assert_eq!(canceled.order.status, OrderStatus::Canceled);
    assert!(canceled.fill.is_none());
}

#[test]
fn duplicate_match_fixture_keeps_stable_fill_identity() {
    let body = include_str!("../../fixtures/kucoin/classic_ws_trade_orders_match.json");
    let first = parse_order_fixture(body).fill.expect("first fill");
    let duplicate = parse_order_fixture(body).fill.expect("duplicate fill");

    assert_eq!(first.venue_event_id, duplicate.venue_event_id);
    assert_eq!(first.trade_id, duplicate.trade_id);
}

fn parse_order_fixture(body: &str) -> KucoinOrderUpdate {
    let event = parse_user_event(body)
        .expect("official fixture parses")
        .expect("known order event");
    let KucoinUserEvent::Order(order) = event else {
        panic!("expected order event");
    };
    *order
}

fn assert_balance_event(event: KucoinUserEvent) {
    let KucoinUserEvent::Balance(row) = event else {
        panic!("expected balance");
    };
    assert_eq!(row.subject, "walletBalance.change");
    assert_eq!(row.currency, "USDT");
    assert_eq!(row.total, 10.5);
    assert_eq!(row.available, 8.5);
    assert_eq!(row.hold_balance, 2.0);
    assert_eq!(row.unrealized_pnl, 0.5);
}

fn assert_position_event(event: KucoinUserEvent) {
    assert_position_event_with_quantity(event, -3.0);
}

fn assert_position_event_with_quantity(event: KucoinUserEvent, expected_quantity: f64) {
    let KucoinUserEvent::Position(row) = event else {
        panic!("expected position");
    };
    let KucoinPositionDelta::Change {
        native_symbol,
        current_contracts,
        updated_time_ms,
    } = row
    else {
        panic!("expected position change");
    };
    assert_eq!(native_symbol, "XBTUSDTM");
    assert_eq!(current_contracts, expected_quantity);
    assert_eq!(updated_time_ms, 1_593_487_482_038);
}

fn assert_pro_ack(text: &str, request_id: &str) {
    let ack = parse_pro_order_ack(text).expect("ack parse").expect("ack");

    assert_eq!(ack.request_id, request_id);
    assert_eq!(ack.client_order_id, "cid-1");
    assert_eq!(ack.exchange_order_id, Some("oid-1".to_owned()));
    assert_eq!(ack.live_state, LiveOrderState::Accepted);
}

#[test]
fn ignores_unknown_or_non_message_events() {
    assert!(parse_user_event(r#"{"type":"welcome","id":"1"}"#)
        .expect("valid json")
        .is_none());
    assert!(
        parse_user_event(r#"{"type":"message","topic":"/contractMarket/ticker","data":{}}"#)
            .expect("valid json")
            .is_none()
    );
}

#[test]
fn rejects_fail_open_balance_numeric_fields() {
    let error = parse_user_event(
        r#"{
            "type":"message",
            "topic":"/contractAccount/wallet",
            "subject":"walletBalance.change",
            "data":{
                "currency":"USDT",
                "equity":"10",
                "availableBalance":null,
                "holdBalance":"0",
                "crossUnPnl":"0",
                "isolatedUnPnl":"0",
                "timestamp":1593487482038
            }
        }"#,
    )
    .expect_err("null available must fail");

    assert!(error.to_string().contains("availableBalance"));
}

#[test]
fn partial_position_change_needs_identity_quantity_and_timestamp_only() {
    let partial = r#"{
        "type":"message",
        "topic":"/contract/position:XBTUSDTM",
        "subject":"position.change",
        "data":{
            "symbol":"XBTUSDTM",
            "currentQty":"1",
            "isOpen":true,
            "currentTimestamp":1593487482038
        }
    }"#;
    let missing_symbol = r#"{
        "type":"message",
        "topic":"/contract/positionAll",
        "subject":"position.change",
        "data":{
            "currentQty":"1",
            "currentTimestamp":1593487482038
        }
    }"#;
    let zero_timestamp = r#"{
        "type":"message",
        "topic":"/contract/position:XBTUSDTM",
        "subject":"position.change",
        "data":{
            "symbol":"XBTUSDTM",
            "currentQty":"1",
            "avgEntryPrice":"50000",
            "markPrice":"50001",
            "unrealisedPnl":"0",
            "liquidationPrice":"0",
            "leverage":"2",
            "posMargin":"100",
            "maintMarginReq":"0.004",
            "marginMode":"CROSS",
            "isOpen":true,
            "currentTimestamp":0
        }
    }"#;

    let parsed = parse_user_event(partial)
        .expect("partial change parses")
        .expect("position event");
    let symbol_error = parse_user_event(missing_symbol).expect_err("missing symbol must fail");
    let time_error = parse_user_event(zero_timestamp).expect_err("zero timestamp must fail");

    assert_position_event_with_quantity(parsed, 1.0);
    assert!(symbol_error.to_string().contains("symbol"));
    assert!(time_error.to_string().contains("timestamp"));
}

#[test]
fn parses_official_position_settlement_without_snapshot_fields() {
    let event = parse_user_event(
        r#"{
            "type":"message",
            "topic":"/contract/position:XBTUSDTM",
            "subject":"position.settlement",
            "data":{
                "markPrice":67198.3,
                "qty":-2,
                "positionSide":"SHORT",
                "settleCurrency":"USDT",
                "fundingTime":1771488000000,
                "marginMode":"ISOLATED",
                "fundingFee":-0.00309113,
                "fundingRate":-0.000023,
                "ts":1771488018495030863
            }
        }"#,
    )
    .expect("settlement parses")
    .expect("position event");

    let KucoinUserEvent::Position(KucoinPositionDelta::Settlement {
        native_symbol,
        current_contracts,
        updated_time_ms,
    }) = event
    else {
        panic!("expected settlement");
    };
    assert_eq!(native_symbol.as_deref(), Some("XBTUSDTM"));
    assert_eq!(current_contracts, -2.0);
    assert_eq!(updated_time_ms, 1_771_488_018_495);
}

#[test]
fn rejects_unknown_order_side_type_or_status() {
    let unknown_side = order_event_with(
        r#""type":"match","side":"hold","status":"match","orderType":"limit","liquidity":"maker","feeType":"makerFee""#,
    );
    let unknown_type = order_event_with(
        r#""type":"match","side":"buy","status":"match","orderType":"iceberg","liquidity":"maker","feeType":"makerFee""#,
    );
    let unknown_status = order_event_with(
        r#""type":"match","side":"buy","status":"mystery","orderType":"limit","liquidity":"maker","feeType":"makerFee""#,
    );

    let side_error = parse_user_event(&unknown_side).expect_err("unknown side must fail");
    let type_error = parse_user_event(&unknown_type).expect_err("unknown type must fail");
    let status_error = parse_user_event(&unknown_status).expect_err("unknown status must fail");

    assert!(side_error.to_string().contains("side"));
    assert!(type_error.to_string().contains("orderType"));
    assert!(status_error.to_string().contains("status"));
}

#[test]
fn rejects_undocumented_reject_event_and_conflicting_fee_role() {
    let rejected = order_event_with(
        r#""type":"rejected","side":"buy","status":"rejected","orderType":"limit""#,
    );
    let wrong_fee = order_event_with(
        r#""type":"match","side":"buy","status":"match","orderType":"limit","liquidity":"maker","feeType":"takerFee""#,
    );

    let reject_error = parse_user_event(&rejected).expect_err("reject is not a Classic event");
    let fee_error = parse_user_event(&wrong_fee).expect_err("fee role mismatch must fail");

    assert!(reject_error.to_string().contains("event type"));
    assert!(fee_error.to_string().contains("conflicts"));
}

#[test]
fn rejects_match_without_trade_identity_or_positive_execution() {
    let no_trade_id = order_event_with(
        r#""type":"match","side":"buy","status":"match","orderType":"limit","liquidity":"maker","feeType":"makerFee""#,
    )
    .replace(r#""tradeId":"trade-7","#, "");
    let zero_match = order_event_with(
        r#""type":"match","side":"buy","status":"match","orderType":"limit","liquidity":"maker","feeType":"makerFee""#,
    )
    .replace(r#""matchSize":"1""#, r#""matchSize":"0""#);

    let identity_error = parse_user_event(&no_trade_id).expect_err("tradeId is required");
    let size_error = parse_user_event(&zero_match).expect_err("matchSize must be positive");

    assert!(identity_error.to_string().contains("tradeId"));
    assert!(size_error.to_string().contains("matchSize"));
}

#[test]
fn rejects_terminal_event_without_terminal_quantity_evidence() {
    let malformed = include_str!("../../fixtures/kucoin/classic_ws_trade_orders_filled.json")
        .replace(r#""filledSize": "1""#, r#""filledSize": "0.5""#);

    let error = parse_user_event(&malformed).expect_err("partial quantity cannot be terminal");

    assert!(error.to_string().contains("terminal quantity conflict"));
}

#[test]
fn rejects_non_finite_position_contract_quantity() {
    let text = r#"{
        "type":"message",
        "topic":"/contract/position:XBTUSDTM",
        "subject":"position.change",
        "data":{
            "symbol":"XBTUSDTM",
            "currentQty":"NaN",
            "currentTimestamp":1593487482038
        }
    }"#;

    let error = parse_user_event(text).expect_err("non-finite quantity must fail");

    assert!(error.to_string().contains("currentQty"));
}

#[test]
fn strict_account_fixture_rejects_defaulting() {
    let balance = serde_json::json!({
        "type": "message",
        "topic": "/contractAccount/wallet",
        "subject": "walletBalance.change",
        "data": {
            "currency": "USDT",
            "equity": "10",
            "availableBalance": "8",
            "holdBalance": "2",
            "crossUnPnl": "0",
            "isolatedUnPnl": "0",
            "timestamp": 1_593_487_482_038_i64
        }
    });
    let mut missing_equity = balance.clone();
    missing_equity["data"]
        .as_object_mut()
        .expect("balance data")
        .remove("equity");
    assert!(
        parse_user_event(&serde_json::to_string(&missing_equity).expect("balance json")).is_err()
    );

    let mut non_finite = balance.clone();
    non_finite["data"]["equity"] = serde_json::json!("NaN");
    assert!(parse_user_event(&serde_json::to_string(&non_finite).expect("balance json")).is_err());

    let mut missing_timestamp = balance;
    missing_timestamp["data"]
        .as_object_mut()
        .expect("balance data")
        .remove("timestamp");
    assert!(
        parse_user_event(&serde_json::to_string(&missing_timestamp).expect("balance json"))
            .is_err()
    );

    let mut position = serde_json::json!({
        "type": "message",
        "topic": "/contract/position:XBTUSDTM",
        "subject": "position.change",
        "data": {
            "symbol": "XBTUSDTM",
            "currentQty": 1,
            "avgEntryPrice": "50000",
            "markPrice": "50001",
            "unrealisedPnl": "0",
            "liquidationPrice": "0",
            "leverage": "2",
            "posMargin": "100",
            "maintMarginReq": "0.004",
            "marginMode": "LEGACY",
            "isOpen": true,
            "currentTimestamp": 1_593_487_482_038_i64
        }
    });
    position["data"]
        .as_object_mut()
        .expect("position data")
        .remove("currentQty");
    assert!(parse_user_event(&serde_json::to_string(&position).expect("position json")).is_err());

    let mut missing_order_type: Value = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/classic_ws_trade_orders_match.json"
    ))
    .expect("order fixture json");
    missing_order_type["data"]
        .as_object_mut()
        .expect("order data")
        .remove("orderType");
    assert!(
        parse_user_event(&serde_json::to_string(&missing_order_type).expect("order json")).is_err()
    );

    let mut invalid_timestamp: Value = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/classic_ws_trade_orders_match.json"
    ))
    .expect("order fixture json");
    invalid_timestamp["data"]["orderTime"] = serde_json::json!(0);
    assert!(
        parse_user_event(&serde_json::to_string(&invalid_timestamp).expect("order json")).is_err()
    );

    let mut missing_success: Value = serde_json::from_str(include_str!(
        "../../fixtures/kucoin/wsapi_pro_order_ack.json"
    ))
    .expect("ack fixture json");
    missing_success
        .as_object_mut()
        .expect("ack object")
        .remove("success");
    assert!(
        parse_pro_order_ack(&serde_json::to_string(&missing_success).expect("ack json")).is_err()
    );
}

fn order_event_with(fields: &str) -> String {
    format!(
        r#"{{
            "type":"message",
            "topic":"/contractMarket/tradeOrders",
            "subject":"orderChange",
            "data":{{
                "symbol":"XBTUSDTM",
                "orderId":"7",
                "clientOid":"cid-1",
                {fields},
                "size":"2",
                "price":"50000",
                "filledSize":"1",
                "remainSize":"1",
                "matchPrice":"50010",
                "matchSize":"1",
                "tradeId":"trade-7",
                "orderTime":1593487482038606180,
                "ts":1593487483038606180
            }}
        }}"#
    )
}
