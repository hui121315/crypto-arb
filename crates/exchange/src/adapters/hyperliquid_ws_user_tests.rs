use super::*;
use pretty_assertions::assert_eq;
use serde_json::Value;
use shared_types::{OrderSide, OrderStatus};

const USER: &str = "0x0000000000000000000000000000000000000000";

#[test]
fn subscribe_payloads_match_official_user_subscriptions() {
    let orders: Value =
        serde_json::from_str(&subscribe_order_updates_payload(USER).expect("orders"))
            .expect("json");
    let fills: Value =
        serde_json::from_str(&subscribe_user_fills_payload(USER, true).expect("fills"))
            .expect("json");
    let spot: Value =
        serde_json::from_str(&subscribe_spot_state_payload(USER, Some(true)).expect("spot"))
            .expect("json");
    let open_orders: Value = serde_json::from_str(
        &subscribe_open_orders_payload(USER, Some("xyz")).expect("open orders"),
    )
    .expect("json");

    assert_eq!(orders["method"], "subscribe");
    assert_eq!(orders["subscription"]["type"], "orderUpdates");
    assert_eq!(orders["subscription"]["user"], USER);
    assert_eq!(fills["subscription"]["aggregateByTime"], true);
    assert_eq!(spot["subscription"]["isPortfolioMargin"], true);
    assert_eq!(open_orders["subscription"]["type"], "openOrders");
    assert_eq!(open_orders["subscription"]["dex"], "xyz");
    assert!(subscribe_order_updates_payload("bad").is_err());
}

#[test]
fn parses_dex_scoped_open_orders_as_a_complete_snapshot() {
    let event = parse_user_event(
        r#"{
            "channel":"openOrders",
            "data":{
                "dex":"xyz",
                "user":"0x0000000000000000000000000000000000000000",
                "orders":[{
                    "coin":"xyz:BTC",
                    "side":"A",
                    "limitPx":"50000",
                    "sz":"0.4",
                    "oid":701,
                    "timestamp":1700000000000,
                    "origSz":"1",
                    "reduceOnly":true
                }]
            }
        }"#,
    )
    .expect("open orders parse")
    .expect("known open orders");
    let HyperliquidUserWsEvent::OpenOrders(snapshot) = event else {
        panic!("expected open orders snapshot");
    };

    assert_eq!(snapshot.venue, "hyperliquid:xyz");
    assert_eq!(snapshot.orders.len(), 1);
    assert_eq!(snapshot.orders[0].exchange, "hyperliquid:xyz");
    assert_eq!(snapshot.orders[0].status, OrderStatus::PartiallyFilled);
    assert_eq!(snapshot.orders[0].reduce_only, Some(true));
}

#[test]
fn parses_order_fills_funding_and_user_events() {
    assert_order_event(order_event());
    assert_fill_event(fill_event());
    assert_user_events_fills();
    assert_funding_event(funding_event());
    assert_liquidation_event(liquidation_event());
    assert_non_user_cancel_event(non_user_cancel_event());
}

fn order_event() -> HyperliquidUserWsEvent {
    parse_user_event(
        r#"{
            "channel":"orderUpdates",
            "data":[{
                "order":{
                    "coin":"xyz:BTC",
                    "side":"B",
                    "limitPx":"50000",
                    "sz":"0.02",
                    "oid":123,
                    "timestamp":1700000000000,
                    "origSz":"0.05",
                    "cloid":"0x01010101010101010101010101010101"
                },
                "status":"open",
                "statusTimestamp":1700000000010
            }]
        }"#,
    )
    .expect("order parse")
    .expect("known order")
}

fn fill_event() -> HyperliquidUserWsEvent {
    parse_user_event(
        r#"{
            "channel":"userFills",
            "data":{
                "isSnapshot":false,
                "user":"0x0",
                "fills":[{
                    "coin":"BTC",
                    "px":"50010",
                    "sz":"0.01",
                    "side":"B",
                    "time":1700000000100,
                    "closedPnl":"1.2",
                    "hash":"0xabc",
                    "tid":456,
                    "oid":123,
                    "crossed":true,
                    "fee":"0.01",
                    "feeToken":"USDC"
                }]
            }
        }"#,
    )
    .expect("fill parse")
    .expect("known fill")
}

fn assert_user_events_fills() {
    let event = parse_user_event(
        r#"{
            "channel":"user",
            "data":{
                "fills":[{
                    "coin":"BTC",
                    "px":"50010",
                    "sz":"0.01",
                    "side":"B",
                    "time":1700000000100,
                    "closedPnl":"1.2",
                    "hash":"0xabc",
                    "tid":456,
                    "oid":123,
                    "crossed":true,
                    "fee":"0.01",
                    "feeToken":"USDC"
                }]
            }
        }"#,
    )
    .expect("userEvents fill parse");

    assert_fill_event(event.expect("userEvents fills map"));
}

fn funding_event() -> HyperliquidUserWsEvent {
    parse_user_event(
        r#"{
            "channel":"user",
            "data":{
                "funding":{
                    "time":1700000000200,
                    "coin":"BTC",
                    "usdc":"-0.1",
                    "szi":"0.5",
                    "fundingRate":"0.0001"
                }
            }
        }"#,
    )
    .expect("funding parse")
    .expect("known funding")
}

fn liquidation_event() -> HyperliquidUserWsEvent {
    parse_user_event(
        r#"{
            "channel":"user",
            "data":{
                "liquidation":{
                    "lid":7,
                    "liquidator":"0x111",
                    "liquidated_user":"0x222",
                    "liquidated_ntl_pos":"12.5",
                    "liquidated_account_value":"3.4"
                }
            }
        }"#,
    )
    .expect("liq parse")
    .expect("known liq")
}

fn non_user_cancel_event() -> HyperliquidUserWsEvent {
    parse_user_event(r#"{"channel":"user","data":{"nonUserCancel":[{"coin":"BTC","oid":123}]}}"#)
        .expect("cancel parse")
        .expect("known cancel")
}

fn assert_order_event(event: HyperliquidUserWsEvent) {
    let HyperliquidUserWsEvent::Order(rows) = event else {
        panic!("expected order");
    };
    assert_eq!(
        rows[0].client_order_id,
        "0x01010101010101010101010101010101"
    );
    assert_eq!(
        rows[0].order.client_order_id.as_deref(),
        Some("0x01010101010101010101010101010101")
    );
    assert_eq!(rows[0].order.reduce_only, None);
    assert_eq!(rows[0].live_state, LiveOrderState::PartiallyFilled);
    assert_eq!(rows[0].order.status, OrderStatus::PartiallyFilled);
    assert_eq!(rows[0].order.exchange, "hyperliquid:xyz");
    assert_eq!(rows[0].order.symbol, "BTC");
    assert_eq!(rows[0].order.side, OrderSide::Buy);
    assert!((rows[0].order.filled_quantity - 0.03).abs() < 1e-12);
}

fn assert_fill_event(event: HyperliquidUserWsEvent) {
    let HyperliquidUserWsEvent::Fill(rows) = event else {
        panic!("expected fill");
    };
    assert_eq!(rows[0].coin, "BTC");
    assert_eq!(rows[0].venue, "hyperliquid");
    assert_eq!(rows[0].trade_id, Some(456));
    assert_eq!(rows[0].price, 50010.0);
    assert!(rows[0].crossed);
}

fn assert_funding_event(event: HyperliquidUserWsEvent) {
    let HyperliquidUserWsEvent::Funding(rows) = event else {
        panic!("expected funding");
    };
    assert_eq!(rows[0].usdc, -0.1);
    assert_eq!(rows[0].venue, "hyperliquid");
    assert_eq!(rows[0].funding_rate, 0.0001);
}

fn assert_liquidation_event(event: HyperliquidUserWsEvent) {
    let HyperliquidUserWsEvent::Liquidation(row) = event else {
        panic!("expected liquidation");
    };
    assert_eq!(row.id, 7);
    assert_eq!(row.notional_position, 12.5);
}

fn assert_non_user_cancel_event(event: HyperliquidUserWsEvent) {
    let HyperliquidUserWsEvent::NonUserCancel(rows) = event else {
        panic!("expected non user cancel");
    };
    assert_eq!(rows[0].order_id, "123");
    assert_eq!(rows[0].venue, "hyperliquid");
}

#[test]
fn ws_finality_fixture_maps_partial_and_terminal_states() {
    let event = parse_user_event(include_str!(
        "../../fixtures/hyperliquid/ws_order_updates_finality.json"
    ))
    .expect("fixture parses")
    .expect("known channel");
    let HyperliquidUserWsEvent::Order(rows) = event else {
        panic!("expected order updates");
    };

    assert_eq!(rows[0].order.status, OrderStatus::PartiallyFilled);
    assert_eq!(rows[0].live_state, LiveOrderState::PartiallyFilled);
    assert_eq!(
        rows[0].client_order_id,
        "0x01010101010101010101010101010101"
    );
    assert_eq!(rows[0].order.order_id, "701");
    assert_eq!(rows[1].order.status, OrderStatus::Canceled);
    assert_eq!(rows[1].live_state, LiveOrderState::Cancelled);
    assert_eq!(rows[2].order.status, OrderStatus::Rejected);
    assert_eq!(rows[2].live_state, LiveOrderState::Rejected);
    assert_eq!(rows[3].order.status, OrderStatus::Filled);
    assert_eq!(rows[3].live_state, LiveOrderState::Filled);
}

#[test]
fn dex_scoped_fill_and_funding_fixtures_preserve_reported_values() {
    let fill = parse_user_event(include_str!(
        "../../fixtures/hyperliquid/ws_user_events_fill_xyz.json"
    ))
    .expect("fill fixture parses")
    .expect("fill event");
    let HyperliquidUserWsEvent::Fill(rows) = fill else {
        panic!("expected userEvents fill");
    };
    assert_eq!(rows[0].venue, "hyperliquid:xyz");
    assert_eq!(rows[0].coin, "BTC");
    assert_eq!(rows[0].fee, -0.001);
    assert_eq!(rows[0].fee_token, "USDC");

    let funding = parse_user_event(include_str!(
        "../../fixtures/hyperliquid/ws_user_fundings_xyz.json"
    ))
    .expect("funding fixture parses")
    .expect("funding event");
    let HyperliquidUserWsEvent::Funding(rows) = funding else {
        panic!("expected funding");
    };
    assert_eq!(rows[0].venue, "hyperliquid:xyz");
    assert_eq!(rows[0].coin, "BTC");
    assert_eq!(rows[0].usdc, -0.12);
}

#[test]
fn parses_clearinghouse_spot_and_all_dexs_state() {
    assert_clearinghouse_event(clearinghouse_event());
    assert_spot_state_event(spot_state_event());
    assert_all_dexs_event(all_dexs_event());
}

fn clearinghouse_event() -> HyperliquidUserWsEvent {
    parse_user_event(
        r#"{
            "channel":"clearinghouseState",
            "data":{
                "dex":"xyz",
                "user":"0x0",
                "clearinghouseState":{
                    "marginSummary":{"accountValue":"100","totalMarginUsed":"20"},
                    "withdrawable":"80",
                    "assetPositions":[{
                        "position":{
                            "coin":"xyz:BTC",
                            "szi":"-0.2",
                            "entryPx":"50000",
                            "liquidationPx":"60000",
                            "marginUsed":"10",
                            "unrealizedPnl":"-2",
                            "leverage":{"value":5}
                        }
                    }]
                }
            }
        }"#,
    )
    .expect("ch parse")
    .expect("known ch")
}

fn spot_state_event() -> HyperliquidUserWsEvent {
    parse_user_event(
        r#"{
            "channel":"spotState",
            "data":{
                "user":"0x0",
                "spotState":{
                    "balances":[{"coin":"USDC","token":0,"hold":"0.2","total":"1.2","entryNtl":"1.0"}]
                }
            }
        }"#,
    )
    .expect("spot parse")
    .expect("known spot")
}

fn all_dexs_event() -> HyperliquidUserWsEvent {
    parse_user_event(
        r#"{
            "channel":"allDexsClearinghouseState",
            "data":{
                "user":"0x0",
                "clearinghouseStates":[
                    ["xyz",{
                        "marginSummary":{"accountValue":"9","totalMarginUsed":"1"},
                        "withdrawable":"8",
                        "assetPositions":[]
                    }]
                ]
            }
        }"#,
    )
    .expect("all dex parse")
    .expect("known all dex")
}

fn assert_clearinghouse_event(event: HyperliquidUserWsEvent) {
    let HyperliquidUserWsEvent::Clearinghouse(row) = event else {
        panic!("expected clearinghouse");
    };
    assert_eq!(row.dex, Some("xyz".to_owned()));
    assert_eq!(row.withdrawable, 80.0);
    assert_eq!(row.positions[0].side, "short");
    assert_eq!(row.positions[0].liquidation_price, Some(60_000.0));
}

fn assert_spot_state_event(event: HyperliquidUserWsEvent) {
    let HyperliquidUserWsEvent::SpotState(rows) = event else {
        panic!("expected spot state");
    };
    assert_eq!(rows[0].coin, "USDC");
    assert_eq!(rows[0].total - rows[0].hold, 1.0);
}

fn assert_all_dexs_event(event: HyperliquidUserWsEvent) {
    let HyperliquidUserWsEvent::AllDexsClearinghouse(rows) = event else {
        panic!("expected all dex");
    };
    assert_eq!(rows[0].dex, "xyz");
    assert_eq!(rows[0].state.account_value, 9.0);
}

#[test]
fn ignores_non_user_channels() {
    assert!(
        parse_user_event(r#"{"channel":"subscriptionResponse","data":{}}"#)
            .expect("valid json")
            .is_none()
    );
}

#[test]
fn order_updates_fail_closed_on_missing_or_unknown_fields() {
    let missing_status_time = r#"{
        "channel":"orderUpdates",
        "data":[{
            "order":{
                "coin":"BTC",
                "side":"B",
                "limitPx":"50000",
                "sz":"0.01",
                "oid":123,
                "timestamp":1700000000000,
                "origSz":"0.01"
            },
            "status":"open"
        }]
    }"#;
    assert!(parse_user_event(missing_status_time).is_err());

    let unknown_side = r#"{
        "channel":"orderUpdates",
        "data":[{
            "order":{
                "coin":"BTC",
                "side":"?",
                "limitPx":"50000",
                "sz":"0.01",
                "oid":123,
                "timestamp":1700000000000,
                "origSz":"0.01"
            },
            "status":"open",
            "statusTimestamp":1700000000010
        }]
    }"#;
    assert!(parse_user_event(unknown_side).is_err());

    let unknown_status = r#"{
        "channel":"orderUpdates",
        "data":[{
            "order":{
                "coin":"BTC",
                "side":"B",
                "limitPx":"50000",
                "sz":"0.01",
                "oid":123,
                "timestamp":1700000000000,
                "origSz":"0.01"
            },
            "status":"mystery",
            "statusTimestamp":1700000000010
        }]
    }"#;
    assert!(parse_user_event(unknown_status).is_err());

    let missing_fee_token = r#"{
        "channel":"userFills",
        "data":{"fills":[{
            "coin":"BTC","px":"50000","sz":"0.01","side":"B",
            "time":1700000000010,"closedPnl":"0","hash":"0xabc","tid":9,
            "oid":123,"crossed":true,"fee":"0.01"
        }]}
    }"#;
    assert!(parse_user_event(missing_fee_token).is_err());

    let missing_funding_amount = r#"{
        "channel":"userFundings",
        "data":{"fundings":[{
            "time":1700000000010,"coin":"BTC","szi":"1","fundingRate":"0.0001"
        }]}
    }"#;
    assert!(parse_user_event(missing_funding_amount).is_err());
}

#[test]
fn account_state_fail_closed_on_missing_or_bad_numbers() {
    let missing_margin = r#"{
        "channel":"clearinghouseState",
        "data":{
            "dex":"xyz",
            "user":"0x0",
            "clearinghouseState":{
                "withdrawable":"80",
                "assetPositions":[]
            }
        }
    }"#;
    assert!(parse_user_event(missing_margin).is_err());

    let null_withdrawable = r#"{
        "channel":"clearinghouseState",
        "data":{
            "dex":"xyz",
            "user":"0x0",
            "clearinghouseState":{
                "marginSummary":{"accountValue":"100","totalMarginUsed":"20"},
                "withdrawable":null,
                "assetPositions":[]
            }
        }
    }"#;
    assert!(parse_user_event(null_withdrawable).is_err());

    let missing_entry_price = r#"{
        "channel":"clearinghouseState",
        "data":{
            "dex":"xyz",
            "user":"0x0",
            "clearinghouseState":{
                "marginSummary":{"accountValue":"100","totalMarginUsed":"20"},
                "withdrawable":"80",
                "assetPositions":[{
                    "position":{
                        "coin":"xyz:BTC",
                        "szi":"-0.2",
                        "liquidationPx":"60000",
                        "marginUsed":"10",
                        "unrealizedPnl":"-2"
                    }
                }]
            }
        }
    }"#;
    assert!(parse_user_event(missing_entry_price).is_err());

    let bad_spot_total = r#"{
        "channel":"spotState",
        "data":{
            "user":"0x0",
            "spotState":{
                "balances":[{"coin":"USDC","token":0,"hold":"0.2","total":"","entryNtl":"1.0"}]
            }
        }
    }"#;
    assert!(parse_user_event(bad_spot_total).is_err());
}
