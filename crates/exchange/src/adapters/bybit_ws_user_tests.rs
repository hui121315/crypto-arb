use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;
use shared_types::{LiveOrderState, OrderSide, OrderStatus, OrderType};

#[test]
fn auth_payload_matches_bybit_private_ws_schema() {
    let value: Value = serde_json::from_str(
        &auth_payload(BybitUserWsConfig {
            api_key: "key",
            api_secret: "secret",
        })
        .expect("auth payload"),
    )
    .expect("json");

    assert_eq!(value["op"], "auth");
    assert_eq!(value["args"][0], "key");
    assert!(value["args"][1].is_i64());
    assert_eq!(value["args"][2].as_str().unwrap_or_default().len(), 64);
    assert_eq!(private_ws_url(false), BYBIT_PRIVATE_WS_URL);
    assert_eq!(private_ws_url(true), BYBIT_TESTNET_PRIVATE_WS_URL);
}

#[test]
fn subscribe_payload_uses_bybit_private_topics() {
    let value: Value =
        serde_json::from_str(&subscribe_private_payload("u1").expect("subscribe")).expect("json");

    assert_eq!(value["req_id"], "u1");
    assert_eq!(value["op"], "subscribe");
    assert_eq!(value["args"][0], "order");
    assert_eq!(value["args"][1], "execution");
    assert_eq!(value["args"][2], "position");
    assert_eq!(value["args"][3], "wallet");
    assert!(subscribe_payload("u1", &[]).is_err());
}

#[test]
fn private_ws_control_fixtures_require_auth_and_subscription_ack() {
    for (fixture, expected_channel) in [
        (
            include_str!("../../fixtures/bybit/ws_user_auth_success.json"),
            "auth",
        ),
        (
            include_str!("../../fixtures/bybit/ws_user_subscribe_success.json"),
            "subscribe",
        ),
    ] {
        assert!(matches!(
            parse_user_control(fixture).expect("control fixture parses"),
            Some(BybitUserControl::Acknowledged { channel, .. }) if channel == expected_channel
        ));
    }
}

#[test]
fn private_ws_control_rejections_preserve_auth_scope() {
    assert!(matches!(
        parse_user_control(include_str!(
            "../../fixtures/bybit/ws_user_auth_failure.json"
        ))
        .expect("auth failure parses"),
        Some(BybitUserControl::Rejected {
            authentication_failed: true,
            error,
            ..
        }) if error.contains("invalid")
    ));
    assert!(matches!(
        parse_user_control(include_str!(
            "../../fixtures/bybit/ws_user_subscribe_failure.json"
        ))
        .expect("subscription failure parses"),
        Some(BybitUserControl::Rejected {
            authentication_failed: false,
            request_id: Some(request_id),
            ..
        }) if request_id == "private"
    ));
}

#[test]
fn parses_order_event_to_order_delta() {
    let row = post_only_order_update();
    assert_post_only_order_update(&row);
}

fn post_only_order_update() -> BybitOrderUpdate {
    let event = parse_user_event(
        r#"{
            "topic":"order",
            "id":"1",
            "creationTime":1672364262474,
            "data":[{
                "category":"linear",
                "orderId":"7",
                "orderLinkId":"cid-1",
                "symbol":"BTCUSDT",
                "side":"Buy",
                "orderType":"Limit",
                "timeInForce":"PostOnly",
                "orderStatus":"PartiallyFilled",
                "qty":"0.01",
                "price":"50000",
                "avgPrice":"50010",
                "cumExecQty":"0.003",
                "cumExecFee":"0.12",
                "positionIdx":1,
                "cancelType":"UNKNOWN",
                "rejectReason":"EC_NoError",
                "leavesQty":"0.007",
                "reduceOnly":false,
                "createdTime":"1672364262444"
            }]
        }"#,
    )
    .expect("event")
    .expect("known topic");

    let BybitUserEvent::Order(rows) = event else {
        panic!("expected order event");
    };
    rows.into_iter()
        .next()
        .expect("order stream should contain one row")
}

fn assert_post_only_order_update(row: &BybitOrderUpdate) {
    assert_eq!(row.client_order_id, "cid-1");
    assert_eq!(row.live_state, LiveOrderState::PartiallyFilled);
    assert_post_only_order_info(row);
    assert_post_only_finality(row);
}

fn assert_post_only_order_info(row: &BybitOrderUpdate) {
    assert_eq!(row.order.order_id, "7");
    assert_eq!(row.order.symbol, "BTC");
    assert_eq!(row.order.side, OrderSide::Buy);
    assert_eq!(row.order.order_type, OrderType::PostOnly);
    assert_eq!(row.order.status, OrderStatus::PartiallyFilled);
    assert_eq!(row.order.filled_quantity, 0.003);
    assert_eq!(row.order.fees, 0.12);
    assert_eq!(row.order.client_order_id.as_deref(), Some("cid-1"));
    assert_eq!(row.order.reduce_only, Some(false));
}

fn assert_post_only_finality(row: &BybitOrderUpdate) {
    assert_eq!(row.finality.position_idx, Some(1));
    assert_eq!(row.finality.cancel_type.as_deref(), Some("UNKNOWN"));
    assert_eq!(row.finality.reject_reason.as_deref(), Some("EC_NoError"));
    assert_eq!(row.finality.leaves_quantity, Some(0.007));
    assert_eq!(row.finality.reduce_only, Some(false));
    assert_eq!(row.finality.time_in_force, "PostOnly");
}

#[test]
fn parses_execution_event_to_fill_delta_source() {
    let rows = execution_rows();

    assert_eq!(rows.len(), 2);
    assert_first_execution_row(&rows[0]);
    assert_second_execution_row(&rows[1]);
}

#[test]
fn parses_official_execution_fixture_with_usdc_fee_identity() {
    let event = parse_user_event(include_str!(
        "../../fixtures/bybit/ws_user_execution_fill.json"
    ))
    .expect("execution fixture parses")
    .expect("execution topic");
    let BybitUserEvent::Execution(rows) = event else {
        panic!("expected execution event");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].symbol, "BTCPERP");
    assert_eq!(rows[0].fee, Some(26.3725275));
    assert_eq!(rows[0].fee_currency.as_deref(), Some("USDC"));
    assert_eq!(rows[0].exec_id, "bybit-exec-fill-1");
}

#[test]
fn parses_official_order_fixture_to_terminal_delta() {
    let event = parse_user_event(include_str!(
        "../../fixtures/bybit/ws_user_order_filled.json"
    ))
    .expect("order fixture parses")
    .expect("order topic");
    let BybitUserEvent::Order(rows) = event else {
        panic!("expected order event");
    };
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].order.symbol, "BTCPERP");
    assert_eq!(rows[0].order.status, OrderStatus::Filled);
    assert_eq!(rows[0].live_state, LiveOrderState::Filled);
    assert_eq!(rows[0].finality.leaves_quantity, Some(0.0));
}

fn execution_rows() -> Vec<BybitExecutionUpdate> {
    let event = parse_user_event(
        r#"{
            "topic":"execution.linear",
            "id":"386825804_BTCUSDT_140612148849382",
            "creationTime":1746270400355,
            "data":[{
                "category":"linear",
                "symbol":"BTCUSDT",
                "side":"Sell",
                "orderId":"9aac161b-8ed6-450d-9cab-c5cc67c21784",
                "orderLinkId":"cid-1",
                "execId":"0ab1bdf7-4219-438b-b30a-32ec863018f7",
                "execPrice":"95900.1",
                "execQty":"0.5",
                "execFee":"26.3725275",
                "feeRate":"0.00055",
                "execTime":"1746270400353",
                "isMaker":false,
                "extraFees":[{
                    "feeCoin":"USDT",
                    "feeType":"GST",
                    "subFeeType":"IND_GST",
                    "feeRate":"0.0000675",
                    "fee":"0.006403779"
                }],
                "feeCurrency":"USDT",
                "seq":140612148849382
            },{
                "category":"linear",
                "symbol":"BTCUSDT",
                "side":"Sell",
                "orderId":"9aac161b-8ed6-450d-9cab-c5cc67c21784",
                "orderLinkId":"cid-1",
                "execId":"91dfbbef-a58f-4377-9fdc-4f0eaa88b98d3",
                "execPrice":"95901.2",
                "execQty":"0.1",
                "execFee":"",
                "feeRate":"",
                "execTime":"1746270400354",
                "isMaker":true,
                "extraFees":"",
                "feeCurrency":"",
                "seq":140612148849382
            }]
        }"#,
    )
    .expect("execution")
    .expect("known topic");

    let BybitUserEvent::Execution(rows) = event else {
        panic!("expected execution event");
    };
    rows
}

fn assert_first_execution_row(row: &BybitExecutionUpdate) {
    assert_first_execution_identity(row);
    assert_first_execution_economics(row);
    assert_first_execution_extra_fee(row);
    assert_first_execution_meta(row);
}

fn assert_first_execution_identity(row: &BybitExecutionUpdate) {
    assert_eq!(row.order_id, "9aac161b-8ed6-450d-9cab-c5cc67c21784");
    assert_eq!(row.client_order_id, "cid-1");
    assert_eq!(row.exec_id, "0ab1bdf7-4219-438b-b30a-32ec863018f7");
    assert_eq!(row.symbol, "BTC");
    assert_eq!(row.category, "linear");
    assert_eq!(row.side, "Sell");
}

fn assert_first_execution_economics(row: &BybitExecutionUpdate) {
    assert_eq!(row.price, 95_900.1);
    assert_eq!(row.size, 0.5);
    assert_eq!(row.fee, Some(26.3725275));
    assert_eq!(row.fee_currency.as_deref(), Some("USDT"));
    assert_eq!(row.fee_rate, Some(0.00055));
}

fn assert_first_execution_extra_fee(row: &BybitExecutionUpdate) {
    assert_eq!(row.extra_fees.len(), 1);
    assert_eq!(row.extra_fees[0].fee_coin, "USDT");
    assert_eq!(row.extra_fees[0].fee_type, "GST");
    assert_eq!(row.extra_fees[0].sub_fee_type, "IND_GST");
    assert_eq!(row.extra_fees[0].fee_rate, Some(0.0000675));
    assert_eq!(row.extra_fees[0].fee, Some(0.006403779));
}

fn assert_first_execution_meta(row: &BybitExecutionUpdate) {
    assert_eq!(row.trade_time_ms, 1_746_270_400_353);
    assert_eq!(row.is_maker, Some(false));
    assert_eq!(row.seq, Some(140_612_148_849_382));
}

fn assert_second_execution_row(row: &BybitExecutionUpdate) {
    assert_eq!(row.exec_id, "91dfbbef-a58f-4377-9fdc-4f0eaa88b98d3");
    assert_eq!(row.fee, None);
    assert_eq!(row.fee_currency, None);
    assert_eq!(row.fee_rate, None);
    assert_eq!(row.extra_fees.len(), 0);
    assert_eq!(row.is_maker, Some(true));
}

#[test]
fn parses_market_order_with_official_blank_optional_fields() {
    let event = parse_user_event(
        r#"{
            "topic":"order.linear",
            "data":[{
                "orderId":"8",
                "orderLinkId":"",
                "symbol":"ETHUSDT",
                "side":"Sell",
                "orderType":"Market",
                "timeInForce":"IOC",
                "orderStatus":"New",
                "qty":"0.2",
                "price":"",
                "avgPrice":"",
                "cumExecQty":"0",
                "cumExecFee":"",
                "createdTime":"1672364262444"
            }]
        }"#,
    )
    .expect("event")
    .expect("known topic");

    let BybitUserEvent::Order(rows) = event else {
        panic!("expected order event");
    };
    let row = &rows[0];
    assert_eq!(row.live_state, LiveOrderState::Accepted);
    assert_eq!(row.order.symbol, "ETH");
    assert_eq!(row.order.side, OrderSide::Sell);
    assert_eq!(row.order.order_type, OrderType::Market);
    assert_eq!(row.order.status, OrderStatus::Open);
    assert_eq!(row.order.price, 0.0);
    assert_eq!(row.order.filled_price, 0.0);
    assert_eq!(row.order.fees, 0.0);
    assert_eq!(row.order.client_order_id, None);
}

#[test]
fn parses_position_and_wallet_events_without_available_balance_guessing() {
    let position = parse_user_event(include_str!(
        "../../fixtures/bybit/ws_user_position_snapshot.json"
    ))
    .expect("position")
    .expect("known topic");

    let BybitUserEvent::Position(update) = position else {
        panic!("expected position event");
    };
    assert_eq!(update.update_type, "snapshot");
    assert_eq!(update.positions[0].symbol, "BTCPERP");
    assert_eq!(update.positions[0].side, "long");
    assert_eq!(update.positions[0].liquidation_price, Some(64_000.0));

    let wallet = parse_user_event(include_str!(
        "../../fixtures/bybit/ws_user_wallet_snapshot.json"
    ))
    .expect("wallet")
    .expect("known topic");

    let BybitUserEvent::Wallet(update) = wallet else {
        panic!("expected wallet event");
    };
    assert_eq!(update.update_type, "snapshot");
    assert_eq!(update.observed_at_ms, 1_783_913_200_100);
    assert_eq!(update.accounts[0].account_type, "UNIFIED");
    assert_eq!(update.accounts[0].total_equity, 10_262.913_350_23);
    assert_eq!(update.accounts[0].total_available_balance, 9_556.605_655_5);
    assert_eq!(update.accounts[0].total_initial_margin, 127.857_316_14);
    assert_eq!(update.accounts[0].total_maintenance_margin, 54.328_462_87);
    assert_eq!(update.accounts[0].coins[0].coin, "USDT");
    assert_eq!(update.accounts[0].coins[0].available_to_withdraw, None);
    assert_eq!(update.accounts[0].coins[0].unrealized_pnl, 578.450_378_59);
}

#[test]
fn parses_empty_position_side_and_blank_portfolio_margin_fields() {
    let event = parse_user_event(
        r#"{
            "topic":"position.linear",
            "type":"snapshot",
            "data":[{
                "category":"linear",
                "symbol":"BTCUSDT",
                "side":"",
                "size":"0",
                "entryPrice":"0",
                "markPrice":"50000",
                "unrealisedPnl":"0",
                "leverage":"",
                "liqPrice":"",
                "updatedTime":"1697682317038"
            }]
        }"#,
    )
    .expect("position")
    .expect("known topic");

    let BybitUserEvent::Position(update) = event else {
        panic!("expected position event");
    };
    assert_eq!(update.positions[0].side, "");
    assert_eq!(update.positions[0].size, 0.0);
    assert_eq!(update.positions[0].leverage, 1.0);
    assert_eq!(update.positions[0].liquidation_price, None);
}

#[test]
fn ignores_unknown_events() {
    assert!(parse_user_event(r#"{"topic":"greek","data":[]}"#)
        .expect("valid json")
        .is_none());
}

#[test]
fn rejects_known_topic_without_data_array() {
    assert_parse_error(r#"{"topic":"order"}"#, "missing data array");
}

#[test]
fn rejects_unknown_order_enums_without_defaulting() {
    assert_parse_error(
        r#"{
            "topic":"order",
            "data":[{
                "orderId":"7",
                "symbol":"BTCUSDT",
                "side":"Hold",
                "orderType":"Limit",
                "timeInForce":"GTC",
                "orderStatus":"New",
                "qty":"0.01",
                "price":"50000",
                "cumExecQty":"0",
                "createdTime":"1672364262444"
            }]
        }"#,
        "order.side",
    );
    assert_parse_error(
        r#"{
            "topic":"order",
            "data":[{
                "orderId":"7",
                "symbol":"BTCUSDT",
                "side":"Buy",
                "orderType":"Bbo",
                "timeInForce":"GTC",
                "orderStatus":"New",
                "qty":"0.01",
                "price":"50000",
                "cumExecQty":"0",
                "createdTime":"1672364262444"
            }]
        }"#,
        "order.orderType",
    );
    assert_parse_error(
        r#"{
            "topic":"order",
            "data":[{
                "orderId":"7",
                "symbol":"BTCUSDT",
                "side":"Buy",
                "orderType":"Limit",
                "timeInForce":"GTC",
                "orderStatus":"Lost",
                "qty":"0.01",
                "price":"50000",
                "cumExecQty":"0",
                "createdTime":"1672364262444"
            }]
        }"#,
        "order.orderStatus",
    );
}

#[test]
fn rejects_required_numeric_and_timestamp_fields_without_zero_fallback() {
    assert_parse_error(
        r#"{
            "topic":"order",
            "data":[{
                "orderId":"7",
                "symbol":"BTCUSDT",
                "side":"Buy",
                "orderType":"Limit",
                "timeInForce":"GTC",
                "orderStatus":"New",
                "qty":"",
                "price":"50000",
                "cumExecQty":"0",
                "createdTime":"1672364262444"
            }]
        }"#,
        "order.qty",
    );
    assert_parse_error(
        r#"{
            "topic":"order",
            "data":[{
                "orderId":"7",
                "symbol":"BTCUSDT",
                "side":"Buy",
                "orderType":"Limit",
                "timeInForce":"GTC",
                "orderStatus":"New",
                "qty":"0.01",
                "price":"50000",
                "cumExecQty":"0",
                "createdTime":"0"
            }]
        }"#,
        "order.createdTime",
    );
    assert_parse_error(
        r#"{
                "topic":"execution",
                "data":[{
                "category":"linear",
                "orderId":"7",
                "symbol":"BTCUSDT",
                "side":"Buy",
                "execId":"e1",
                "execPrice":"0",
                "execQty":"0.01",
                "execTime":"1746270400353"
            }]
        }"#,
        "execution.execPrice",
    );
    assert_parse_error(
        r#"{
            "topic":"execution",
            "data":[{
                "category":"linear",
                "orderId":"7",
                "symbol":"BTCUSDT",
                "side":"Buy",
                "execId":"e1",
                "execPrice":"10",
                "execQty":"0.01",
                "execTime":"1746270400353",
                "extraFees":[{"feeCoin":"","feeType":"GST","subFeeType":"IND_GST"}]
            }]
        }"#,
        "execution.extraFees.feeCoin",
    );
    assert_parse_error(
        r#"{
            "topic":"order",
            "data":[{
                "orderId":"7",
                "symbol":"BTCUSDT",
                "side":"Buy",
                "orderType":"Limit",
                "timeInForce":"GTC",
                "orderStatus":"New",
                "qty":"0.01",
                "price":"50000",
                "cumExecQty":"0",
                "leavesQty":"bad",
                "createdTime":"1672364262444"
            }]
        }"#,
        "order.leavesQty",
    );
}

#[test]
fn rejects_nonzero_position_with_empty_or_unknown_side() {
    assert_parse_error(
        r#"{
            "topic":"position",
            "data":[{
                "category":"linear",
                "symbol":"BTCUSDT",
                "side":"",
                "size":"0.01",
                "entryPrice":"50000",
                "markPrice":"50010",
                "unrealisedPnl":"1",
                "leverage":"3",
                "liqPrice":"",
                "updatedTime":"1697682317038"
            }]
        }"#,
        "position.side",
    );
}

#[test]
fn rejects_wallet_missing_coin_or_bad_numeric_field() {
    assert_parse_error(
        r#"{"topic":"wallet","creationTime":1700034722104,"data":[{"accountType":"UNIFIED","accountIMRate":"0","accountMMRate":"0","totalEquity":"1","totalAvailableBalance":"1","totalInitialMargin":"0","totalMaintenanceMargin":"0"}]}"#,
        "wallet.coin",
    );
    assert_parse_error(
        r#"{
            "topic":"wallet",
            "creationTime":1700034722104,
            "data":[{
                "accountType":"UNIFIED",
                "accountIMRate":"0",
                "accountMMRate":"0",
                "totalEquity":"10.5",
                "totalAvailableBalance":"950",
                "totalInitialMargin":"0",
                "totalMaintenanceMargin":"0",
                "coin":[{
                    "coin":"USDT",
                    "equity":"bad",
                    "usdValue":"10.5",
                    "walletBalance":"10",
                    "availableToWithdraw":"",
                    "locked":"0.2",
                    "unrealisedPnl":"0.5"
                }]
            }]
        }"#,
        "wallet.coin.equity",
    );
    assert_parse_error(
        r#"{
            "topic":"wallet",
            "creationTime":1700034722104,
            "data":[{
                "accountType":"UNIFIED",
                "accountIMRate":"0",
                "accountMMRate":"0",
                "totalEquity":"10.5",
                "totalAvailableBalance":"bad",
                "totalInitialMargin":"0",
                "totalMaintenanceMargin":"0",
                "coin":[{
                    "coin":"USDT",
                    "equity":"10.5",
                    "usdValue":"10.5",
                    "walletBalance":"10",
                    "availableToWithdraw":"",
                    "locked":"0.2",
                    "unrealisedPnl":"0.5"
                }]
            }]
        }"#,
        "wallet.totalAvailableBalance",
    );
}

fn assert_parse_error(payload: &str, needle: &str) {
    let error = parse_user_event(payload).expect_err("payload should fail closed");
    assert!(
        error.to_string().contains(needle),
        "error {error:?} should contain {needle}"
    );
}
