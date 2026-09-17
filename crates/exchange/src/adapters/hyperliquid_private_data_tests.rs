use super::*;
use pretty_assertions::assert_eq;

#[test]
fn parse_spot_balances_filters_currency() {
    let state = SpotClearinghouseState {
        balances: vec![
            SpotBalance {
                coin: "USDC".into(),
                total: "14.625485".into(),
                hold: "1.25".into(),
            },
            SpotBalance {
                coin: "PURR".into(),
                total: "2000".into(),
                hold: "0".into(),
            },
        ],
    };
    let parsed = parse_spot_balances(state, Some("USDC")).expect("spot balances");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed["USDC"].available, 13.375485);
}

#[test]
fn parse_spot_balances_rejects_hold_above_total() {
    let state = SpotClearinghouseState {
        balances: vec![SpotBalance {
            coin: "USDC".into(),
            total: "1".into(),
            hold: "2".into(),
        }],
    };
    let error = parse_spot_balances(state, None).expect_err("hold above total must fail");
    assert!(error.to_string().contains("hold exceeds total"));
}

#[test]
fn parse_spot_balances_rejects_blank_identity_and_negative_values() {
    let blank_coin = SpotClearinghouseState {
        balances: vec![SpotBalance {
            coin: String::new(),
            total: "1".into(),
            hold: "0".into(),
        }],
    };
    assert!(parse_spot_balances(blank_coin, None).is_err());

    let negative_total = SpotClearinghouseState {
        balances: vec![SpotBalance {
            coin: "USDC".into(),
            total: "-1".into(),
            hold: "0".into(),
        }],
    };
    assert!(parse_spot_balances(negative_total, None).is_err());
}

#[test]
fn parse_perp_balance_sums_unrealized_pnl() {
    let state = clearinghouse_state(vec![
        asset_position("ETH", "0.0335", "-0.0134"),
        asset_position("BTC", "-0.01", "1.25"),
    ]);
    let parsed = parse_perp_balance(&state, Some("USDC")).expect("perp balance");
    assert_eq!(parsed["USDC"].total, 13109.482328);
    assert_eq!(parsed["USDC"].available, 13104.514502);
    assert_eq!(parsed["USDC"].frozen, 4.967826);
    assert!((parsed["USDC"].unrealized_pnl - 1.2366).abs() < 1e-9);
}

#[test]
fn hyperliquid_account_balance_parses_official_fixtures() {
    let perp: ClearinghouseState = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_clearinghouse_state_account_balance.json"
    ))
    .expect("official clearinghouseState fixture decodes");
    let spot: SpotClearinghouseState = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_spot_clearinghouse_state_account_balance.json"
    ))
    .expect("official spotClearinghouseState fixture decodes");

    let perp_balance = parse_perp_balance(&perp, Some("USDC")).expect("perp balance parses");
    let spot_balances = parse_spot_balances(spot, Some("USDC")).expect("spot balance parses");

    assert_eq!(perp_balance["USDC"].total, 13109.482328);
    assert_eq!(perp_balance["USDC"].available, 13104.514502);
    assert_eq!(perp_balance["USDC"].frozen, 4.967826);
    assert_eq!(perp_balance["USDC"].unrealized_pnl, -0.0134);
    assert_eq!(spot_balances["USDC"].total, 14.625485);
    assert_eq!(spot_balances["USDC"].available, 13.375485);
    assert_eq!(spot_balances["USDC"].frozen, 1.25);
}

#[test]
fn hyperliquid_account_summary_preserves_perp_margin_and_withdrawable_facts() {
    let state: ClearinghouseState = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_clearinghouse_state_account_balance.json"
    ))
    .expect("official clearinghouseState fixture decodes");

    let summary =
        parse_account_summary(&state, "hyperliquid", 1_700_000_000_000).expect("summary parses");

    assert_eq!(summary.equity_scope, AccountEquityScope::Perpetuals);
    assert_eq!(summary.total_equity_usd, 13109.482328);
    assert_eq!(summary.total_available_balance_usd, 13104.514502);
    assert_eq!(summary.withdrawable_balance_usd, Some(13104.514502));
    assert_eq!(summary.total_initial_margin_usd, 4.967826);
    assert_eq!(summary.total_maintenance_margin_usd, 0.1);
    assert!(summary.account_im_rate > 0.0);
    assert!(summary.account_mm_rate > 0.0);
}

#[test]
fn strict_account_fixture_rejects_defaulting() {
    let mut non_finite_balance: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_clearinghouse_state_account_balance.json"
    ))
    .unwrap();
    non_finite_balance["marginSummary"]["accountValue"] = serde_json::json!("NaN");
    let state: ClearinghouseState = serde_json::from_value(non_finite_balance).unwrap();
    assert!(parse_perp_balance(&state, Some("USDC")).is_err());

    let mut unknown_position_type: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_clearinghouse_state_account_balance.json"
    ))
    .unwrap();
    unknown_position_type["assetPositions"][0]["type"] = serde_json::json!("twoWay");
    let state: ClearinghouseState = serde_json::from_value(unknown_position_type).unwrap();
    assert!(parse_perp_balance(&state, Some("USDC")).is_err());

    let mut invalid_entry_price: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_clearinghouse_state_account_balance.json"
    ))
    .unwrap();
    invalid_entry_price["assetPositions"][0]["position"]["entryPx"] = serde_json::json!("0");
    let state: ClearinghouseState = serde_json::from_value(invalid_entry_price).unwrap();
    let mark_map = HashMap::from([("ETH".to_owned(), 2_985.0)]);
    assert!(parse_positions(state, Some("ETH"), &mark_map, "hyperliquid").is_err());

    let mut missing_timestamp: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_frontend_open_orders_dex.json"
    ))
    .unwrap();
    missing_timestamp[0]
        .as_object_mut()
        .unwrap()
        .remove("timestamp");
    assert!(serde_json::from_value::<Vec<OpenOrderItem>>(missing_timestamp).is_err());

    let mut invalid_timestamp: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_frontend_open_orders_dex.json"
    ))
    .unwrap();
    invalid_timestamp[0]["timestamp"] = serde_json::json!(0);
    let orders: Vec<OpenOrderItem> = serde_json::from_value(invalid_timestamp).unwrap();
    assert!(parse_open_orders(orders, None, "hyperliquid").is_err());

    let mut unknown_type: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_frontend_open_orders_dex.json"
    ))
    .unwrap();
    unknown_type[0]["orderType"] = serde_json::json!("iceberg");
    let orders: Vec<OpenOrderItem> = serde_json::from_value(unknown_type).unwrap();
    assert!(parse_open_orders(orders, None, "hyperliquid").is_err());

    let mut missing_reduce_only: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_frontend_open_orders_dex.json"
    ))
    .unwrap();
    missing_reduce_only[0]
        .as_object_mut()
        .unwrap()
        .remove("reduceOnly");
    assert!(serde_json::from_value::<Vec<OpenOrderItem>>(missing_reduce_only).is_err());

    let mut missing_trigger: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_frontend_open_orders_dex.json"
    ))
    .unwrap();
    missing_trigger[0]
        .as_object_mut()
        .unwrap()
        .remove("isTrigger");
    assert!(serde_json::from_value::<Vec<OpenOrderItem>>(missing_trigger).is_err());
}

#[test]
fn parse_positions_uses_mark_map_and_liquidation_price() {
    let state = clearinghouse_state(vec![
        asset_position("xyz:MU", "-2.5", "7.0"),
        asset_position("ETH", "0", "0"),
    ]);
    let mark_map = HashMap::from([("xyz:MU".to_owned(), 664.63)]);
    let parsed = parse_positions(state, Some("xyz:MU"), &mark_map, "hyperliquid:xyz")
        .expect("positions parse");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].symbol, "MU");
    assert_eq!(parsed[0].side, "short");
    assert_eq!(parsed[0].quantity, 2.5);
    assert_eq!(parsed[0].mark_price, 664.63);
    assert_eq!(parsed[0].liquidation_price, Some(2866.26936529));
}

#[test]
fn hyperliquid_account_position_parses_official_fixture() {
    let state: ClearinghouseState = serde_json::from_str(include_str!(
        "../../fixtures/hyperliquid/info_clearinghouse_state_account_balance.json"
    ))
    .expect("official clearinghouseState position fixture decodes");
    let mark_map = HashMap::from([("ETH".to_owned(), 2985.0)]);
    let parsed =
        parse_positions(state, Some("ETH"), &mark_map, "hyperliquid").expect("positions parse");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].symbol, "ETH");
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[0].quantity, 0.0335);
    assert_eq!(parsed[0].mark_price, 2985.0);
    assert_eq!(parsed[0].liquidation_price, Some(2866.26936529));
}

#[test]
fn parse_open_orders_filters_and_maps_side() {
    let mut order = open_order("Limit", None, false);
    order.side = "A".into();
    let parsed =
        parse_open_orders(vec![order], Some("BTC"), "hyperliquid:xyz").expect("open orders");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].symbol, "BTC");
    assert_eq!(parsed[0].exchange, "hyperliquid:xyz");
    assert!(matches!(parsed[0].side, OrderSide::Sell));
    assert!(matches!(parsed[0].order_type, OrderType::Limit));
    assert_eq!(parsed[0].filled_quantity, 0.0);
}

#[test]
fn classify_order_type_priority() {
    let post_only = open_order("Limit", Some("Alo"), false);
    assert!(matches!(
        parse_open_orders(vec![post_only], None, "hyperliquid").expect("post-only")[0].order_type,
        OrderType::PostOnly
    ));
    let market = open_order("Market", Some("Ioc"), false);
    assert!(matches!(
        parse_open_orders(vec![market], None, "hyperliquid").expect("market")[0].order_type,
        OrderType::Market
    ));
    let trigger_market = open_order("Stop Market", Some("Gtc"), true);
    assert!(matches!(
        parse_open_orders(vec![trigger_market], None, "hyperliquid").expect("trigger market")[0]
            .order_type,
        OrderType::Market
    ));
    let trigger_limit = open_order("Stop Limit", None, true);
    assert!(matches!(
        parse_open_orders(vec![trigger_limit], None, "hyperliquid").expect("trigger limit")[0]
            .order_type,
        OrderType::Limit
    ));
    let frontend_market = open_order("Market", Some("FrontendMarket"), false);
    assert!(matches!(
        parse_open_orders(vec![frontend_market], None, "hyperliquid").expect("frontend market")[0]
            .order_type,
        OrderType::Market
    ));
}

#[test]
fn order_status_payload_maps_status_and_target() {
    let mut order = open_order("Limit", None, false);
    order.coin = "ETH".into();
    let payload = OrderStatusPayload {
        status: "order".into(),
        order: Some(OrderStatusEntry {
            order,
            status: "filled".into(),
            status_timestamp: 1_700_000_000_001,
        }),
    };
    assert!(order_status_to_info(payload, "ETH", "hyperliquid").is_err());
    assert!(order_status_to_info(
        OrderStatusPayload {
            status: "unknownOid".into(),
            order: None,
        },
        "ETH",
        "hyperliquid",
    )
    .expect("unknown oid")
    .is_none());
}

#[test]
fn private_parsers_reject_missing_and_bad_numbers() {
    let bad_spot = SpotClearinghouseState {
        balances: vec![SpotBalance {
            coin: "USDC".into(),
            total: String::new(),
            hold: "0".into(),
        }],
    };
    assert!(parse_spot_balances(bad_spot, None).is_err());

    let mut bad_perp = clearinghouse_state(vec![]);
    bad_perp.margin_summary.account_value = "NaN".into();
    assert!(parse_perp_balance(&bad_perp, None).is_err());
}

#[test]
fn positions_reject_bad_payload_or_missing_mark_price() {
    let state = clearinghouse_state(vec![asset_position("BTC", "0.1", "0")]);
    assert!(parse_positions(state, Some("BTC"), &HashMap::new(), "hyperliquid").is_err());

    let bad_state = clearinghouse_state(vec![asset_position("BTC", "bad", "0")]);
    let mark_map = HashMap::from([("BTC".to_owned(), 100.0)]);
    assert!(parse_positions(bad_state, Some("BTC"), &mark_map, "hyperliquid").is_err());
}

#[test]
fn positions_reject_negative_margin() {
    let mut state = clearinghouse_state(vec![asset_position("BTC", "0.1", "0")]);
    state.asset_positions[0].position.margin_used = "-1".into();
    let mark_map = HashMap::from([("BTC".to_owned(), 100.0)]);

    assert!(parse_positions(state, Some("BTC"), &mark_map, "hyperliquid").is_err());
}

#[test]
fn open_order_and_order_status_fail_closed() {
    let mut bad_side = open_order("Limit", None, false);
    bad_side.side = String::new();
    assert!(parse_open_orders(vec![bad_side], None, "hyperliquid").is_err());

    let mut bad_timestamp = open_order("Limit", None, false);
    bad_timestamp.timestamp = 0;
    assert!(parse_open_orders(vec![bad_timestamp], None, "hyperliquid").is_err());

    let unknown_status = OrderStatusPayload {
        status: "order".into(),
        order: Some(OrderStatusEntry {
            order: open_order("Limit", None, false),
            status: "mystery".into(),
            status_timestamp: 1_700_000_000_001,
        }),
    };
    assert!(order_status_to_info(unknown_status, "BTC", "hyperliquid").is_err());
}

#[test]
fn open_orders_reject_missing_or_unknown_order_semantics() {
    let mut missing_type = open_order("Limit", None, false);
    missing_type.order_type = None;
    let error =
        parse_open_orders(vec![missing_type], None, "hyperliquid").expect_err("missing orderType");
    assert!(error.to_string().contains("missing orderType"));

    let unknown_type = open_order("Iceberg", None, false);
    let error =
        parse_open_orders(vec![unknown_type], None, "hyperliquid").expect_err("unknown orderType");
    assert!(error.to_string().contains("unsupported orderType"));

    let unknown_tif = open_order("Limit", Some("BadTif"), false);
    let error = parse_open_orders(vec![unknown_tif], None, "hyperliquid").expect_err("unknown tif");
    assert!(error.to_string().contains("unsupported tif"));

    let stop_without_trigger = open_order("Stop Limit", Some("Gtc"), false);
    let error = parse_open_orders(vec![stop_without_trigger], None, "hyperliquid")
        .expect_err("missing trigger evidence");
    assert!(error.to_string().contains("requires isTrigger"));
}

fn asset_position(coin: &str, szi: &str, unrealized_pnl: &str) -> AssetPositionEntry {
    AssetPositionEntry {
        position_type: "oneWay".into(),
        position: AssetPosition {
            coin: coin.into(),
            szi: szi.into(),
            entry_px: Some("2986.3".into()),
            liquidation_px: Some("2866.26936529".into()),
            margin_used: "4.967826".into(),
            unrealized_pnl: unrealized_pnl.into(),
            leverage: Some(LeverageEntry {
                margin_mode: "cross".into(),
                value: 20.0,
            }),
        },
    }
}

fn clearinghouse_state(asset_positions: Vec<AssetPositionEntry>) -> ClearinghouseState {
    let margin_summary = MarginSummary {
        account_value: "13109.482328".into(),
        total_ntl_pos: "100".into(),
        total_raw_usd: "13109.482328".into(),
        total_margin_used: "4.967826".into(),
    };
    ClearinghouseState {
        margin_summary: margin_summary.clone(),
        cross_margin_summary: margin_summary,
        cross_maintenance_margin_used: "0.1".into(),
        withdrawable: "13104.514502".into(),
        asset_positions,
    }
}

fn open_order(order_type: &str, tif: Option<&str>, is_trigger: bool) -> OpenOrderItem {
    OpenOrderItem {
        coin: "BTC".into(),
        side: "B".into(),
        oid: 100,
        limit_px: "30000".into(),
        orig_sz: "1".into(),
        sz: "1".into(),
        timestamp: 1_700_000_000_000,
        order_type: Some(order_type.into()),
        tif: tif.map(str::to_owned),
        is_trigger,
        reduce_only: false,
        cloid: None,
    }
}

fn user_fill(oid: i64, coin: &str, px: &str, sz: &str, fee: &str) -> UserFillItem {
    UserFillItem {
        coin: coin.into(),
        oid,
        px: px.into(),
        sz: sz.into(),
        fee: fee.into(),
    }
}

#[test]
fn parse_open_order_surfaces_client_order_id_and_reduce_only() {
    let mut order = open_order("Limit", None, false);
    order.cloid = Some("0x01010101010101010101010101010101".into());
    order.reduce_only = true;
    let parsed = parse_open_orders(vec![order], None, "hyperliquid").expect("open order");
    assert_eq!(
        parsed[0].client_order_id.as_deref(),
        Some("0x01010101010101010101010101010101")
    );
    assert_eq!(parsed[0].reduce_only, Some(true));
}

#[test]
fn parse_open_orders_skips_unparseable_row_without_failing_table() {
    // 部分成交挂单缺 fill 证据：跳过该行，其余行照常返回——
    // 整表可用性不再被单行的证据严格性拖垮。
    let clean = open_order("Limit", None, false);
    let mut partially_filled = open_order("Limit", None, false);
    partially_filled.sz = "0.5".into();
    partially_filled.orig_sz = "1.0".into();

    let parsed = parse_open_orders(vec![partially_filled, clean], None, "hyperliquid")
        .expect("table stays available");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].status, OrderStatus::Open);
}

#[test]
fn parse_open_orders_joins_partial_fill_economics_by_order_id() {
    let mut partially_filled = open_order("Limit", None, false);
    partially_filled.sz = "0.5".into();
    partially_filled.orig_sz = "1.0".into();
    let fills = vec![
        user_fill(100, "BTC", "30000", "0.2", "0.01"),
        user_fill(100, "BTC", "31000", "0.3", "0.02"),
    ];

    let parsed = parse_open_orders_with_fills(vec![partially_filled], &fills, None, "hyperliquid")
        .expect("partial fill evidence");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].status, OrderStatus::PartiallyFilled);
    assert_eq!(parsed[0].filled_quantity, 0.5);
    assert!((parsed[0].filled_price - 30_600.0).abs() < 1e-9);
    assert!((parsed[0].fees - 0.03).abs() < 1e-9);
}

#[test]
fn parse_open_order_collapses_blank_client_order_id() {
    let parsed = parse_open_orders(vec![open_order("Limit", None, false)], None, "hyperliquid")
        .expect("open order");
    assert_eq!(parsed[0].client_order_id, None);
    assert_eq!(parsed[0].reduce_only, Some(false));
}

// PR-EQ: official Hyperliquid `/info` type=orderStatus response-envelope
// fixtures, decoded end-to-end via OrderStatusPayload + order_status_to_info.
// The order_status tests above build OrderStatusPayload structs directly and so
// skip the official JSON envelope, the `unknownOid` not-found path and the
// top-level/inner unsupported-status fail-closed paths that finality mapping
// depends on.
#[test]
fn order_status_official_filled_envelope_requires_user_fills_evidence() {
    let fixture = include_str!("../../fixtures/hyperliquid/info_order_status_filled.json");
    let payload: OrderStatusPayload = serde_json::from_str(fixture).expect("envelope decodes");
    let error = order_status_to_info(payload, "ETH", "hyperliquid")
        .expect_err("order status cannot invent fill economics");
    assert!(error.to_string().contains("userFills evidence"));
}

#[test]
fn order_status_official_filled_envelope_joins_user_fills_evidence() {
    let fixture = include_str!("../../fixtures/hyperliquid/info_order_status_filled.json");
    let payload: OrderStatusPayload = serde_json::from_str(fixture).expect("envelope decodes");
    let fills = vec![user_fill(77, "ETH", "3001", "1", "0.45")];

    let order = order_status_to_info_with_fills(payload, &fills, "ETH", "hyperliquid")
        .expect("order status")
        .expect("order");

    assert_eq!(order.status, OrderStatus::Filled);
    assert_eq!(order.filled_quantity, 1.0);
    assert_eq!(order.filled_price, 3001.0);
    assert_eq!(order.fees, 0.45);
}

#[test]
fn order_status_official_unknown_oid_is_not_found() {
    let payload: OrderStatusPayload =
        serde_json::from_str(r#"{"status":"unknownOid"}"#).expect("envelope decodes");
    let parsed = order_status_to_info(payload, "ETH", "hyperliquid").expect("ok");
    assert!(parsed.is_none());
}

#[test]
fn order_status_official_unsupported_top_status_fails_closed() {
    let payload: OrderStatusPayload =
        serde_json::from_str(r#"{"status":"someWeirdStatus"}"#).expect("envelope decodes");
    assert!(order_status_to_info(payload, "ETH", "hyperliquid").is_err());
}

#[test]
fn order_status_official_unsupported_inner_status_fails_closed() {
    let text = r#"{"status":"order","order":{"order":{"coin":"ETH","side":"B","oid":77,"limitPx":"3000","origSz":"1","sz":"0","timestamp":1700000000000,"orderType":"Limit","tif":"Gtc","isTrigger":false,"reduceOnly":false},"status":"mystery","statusTimestamp":1700000000001}}"#;
    let payload: OrderStatusPayload = serde_json::from_str(text).expect("envelope decodes");
    assert!(order_status_to_info(payload, "ETH", "hyperliquid").is_err());
}

#[test]
fn order_status_official_envelope_filters_other_coin() {
    let fixture = include_str!("../../fixtures/hyperliquid/info_order_status_filled.json");
    let payload: OrderStatusPayload = serde_json::from_str(fixture).expect("envelope decodes");
    let parsed = order_status_to_info(payload, "BTC", "hyperliquid").expect("ok");
    assert!(parsed.is_none());
}
