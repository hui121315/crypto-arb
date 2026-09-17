use super::*;
use pretty_assertions::assert_eq;

#[test]
fn bybit_wallet_balance_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/bybit/wallet_balance_unified.json");
    let resp: crate::adapters::bybit_response::BybitResponse<WalletAccount> =
        serde_json::from_str(fixture).expect("official bybit wallet balance envelope decodes");
    let rows = resp
        .into_list("wallet-balance")
        .expect("retCode 0 yields rows");
    let balances = parse_balance_response(rows, Some("USDT")).expect("wallet rows parse");
    let usdt = balances.get("USDT").expect("USDT row");
    assert_eq!(usdt.total, 3.31216591);
    assert_eq!(usdt.available, 3.00326056);
    assert_eq!(usdt.frozen, 0.0);
}

#[test]
fn bybit_account_summary_parses_equity_margin_rates_and_source() {
    let fixture = include_str!("../../fixtures/bybit/wallet_balance_unified_account_metrics.json");
    let response: crate::adapters::bybit_response::BybitResponse<WalletAccount> =
        serde_json::from_str(fixture).expect("account metrics envelope decodes");
    let (rows, observed_at_ms) = response
        .into_list_with_time("wallet-balance")
        .expect("account metrics response succeeds");
    let read = parse_account_response(rows, None, observed_at_ms).expect("account metrics parse");

    assert_eq!(read.summary.venue, "bybit");
    assert_eq!(read.summary.account_type, "UNIFIED");
    assert_eq!(read.summary.total_equity_usd, 10_262.913_350_23);
    assert_eq!(read.summary.total_available_balance_usd, 9_556.605_655_5);
    assert_eq!(read.summary.total_initial_margin_usd, 127.857_316_14);
    assert_eq!(read.summary.total_maintenance_margin_usd, 54.328_462_87);
    assert_eq!(read.summary.account_im_rate, 0.021);
    assert_eq!(read.summary.account_mm_rate, 0.009);
    assert_eq!(read.summary.observed_at_ms, 1_783_913_200_000);
    assert!(read.summary.source.contains("wallet-balance"));
    assert_eq!(read.balances["USDT"].available, 9_556.605_655_5);
}

#[test]
fn parse_balance_rejects_invalid_usd_total_before_reporting_unavailable_conversion() {
    let rows = vec![wallet_account(
        "UNIFIED",
        "NaN",
        vec![balance("USDT", "1", "", "0.1", "0")],
    )];
    assert!(matches!(
        parse_balance_response(rows, Some("USDT")),
        Err(ExchangeError::Parse(message)) if message.contains("totalAvailableBalance")
    ));
}

#[test]
fn parse_balance_rejects_legacy_account_type() {
    let rows = vec![wallet_account(
        "CONTRACT",
        "88",
        vec![balance("USDT", "100", "90", "5", "1")],
    )];
    assert!(parse_balance_response(rows, Some("USDT")).is_err());
}

#[test]
fn parse_balance_rejects_invalid_coin_fields_and_account_row_count() {
    let rows = vec![wallet_account(
        "UNIFIED",
        "88",
        vec![balance("USDT", "Infinity", "", "5", "1")],
    )];
    assert!(parse_balance_response(rows, Some("USDT")).is_err());
    assert!(parse_balance_response(Vec::new(), None).is_err());

    let rows = vec![
        wallet_account("UNIFIED", "88", vec![balance("USDT", "100", "", "5", "1")]),
        wallet_account("UNIFIED", "77", vec![balance("USDC", "100", "", "5", "1")]),
    ];
    assert!(parse_balance_response(rows, None).is_err());
}

#[test]
fn parse_positions_uses_position_idx_and_pairs_hedges() {
    let rows = vec![
        position("BTCUSDT", "1", 1),
        position("BTCUSDT", "2", 2),
        position("ETHUSDT", "0", 0),
    ];
    let parsed = parse_positions(&rows).expect("positions parse");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[1].side, "short");
    assert_eq!(parsed[0].paired_with.as_deref(), Some("bybit:BTC:short"));
    assert_eq!(parsed[1].paired_with.as_deref(), Some("bybit:BTC:long"));
}

#[test]
fn bybit_positions_parse_official_fixture() {
    let fixture = include_str!("../../fixtures/bybit/position_list_linear_open.json");
    let resp: crate::adapters::bybit_response::BybitResponse<PositionRow> =
        serde_json::from_str(fixture).expect("official bybit positions envelope decodes");
    let rows = resp
        .into_list("positions")
        .expect("retCode 0 yields position rows");
    let parsed = parse_positions(&rows).expect("positions parse");

    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].symbol, "BTC");
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[1].side, "short");
    assert_eq!(parsed[0].paired_with.as_deref(), Some("bybit:BTC:short"));
    assert_eq!(parsed[1].paired_with.as_deref(), Some("bybit:BTC:long"));
}

#[test]
fn parse_positions_rejects_bad_size_and_unknown_side() {
    let bad_size = vec![position("BTCUSDT", "bad", 1)];
    assert!(parse_positions(&bad_size).is_err());

    let unknown_side = vec![PositionRow {
        side: String::new(),
        ..position("BTCUSDT", "1", 0)
    }];
    assert!(parse_positions(&unknown_side).is_err());
}

#[test]
fn parse_positions_rejects_blank_leverage_and_margin_evidence() {
    let row = PositionRow {
        leverage: String::new(),
        liq_price: String::new(),
        position_im: String::new(),
        ..position("BTCUSDT", "1", 1)
    };
    assert!(parse_positions(&[row]).is_err());
}

#[test]
fn order_position_idx_uses_one_way_evidence() {
    let rows = vec![position("BTCUSDT", "0", 0)];

    assert_eq!(
        order_position_idx(&rows, OrderSide::Buy, false).expect("one-way buy"),
        0
    );
    assert_eq!(
        order_position_idx(&rows, OrderSide::Sell, true).expect("one-way close"),
        0
    );
}

#[test]
fn order_position_idx_maps_hedge_open_and_reduce_only() {
    let rows = vec![position("BTCUSDT", "0", 1), position("BTCUSDT", "0", 2)];

    assert_eq!(
        order_position_idx(&rows, OrderSide::Buy, false).expect("open long"),
        1
    );
    assert_eq!(
        order_position_idx(&rows, OrderSide::Sell, false).expect("open short"),
        2
    );
    assert_eq!(
        order_position_idx(&rows, OrderSide::Sell, true).expect("close long"),
        1
    );
    assert_eq!(
        order_position_idx(&rows, OrderSide::Buy, true).expect("close short"),
        2
    );
}

#[test]
fn order_position_idx_rejects_missing_mixed_or_unknown_evidence() {
    assert!(order_position_idx(&[], OrderSide::Buy, false).is_err());

    let mixed = vec![position("BTCUSDT", "0", 0), position("BTCUSDT", "0", 1)];
    assert!(order_position_idx(&mixed, OrderSide::Buy, false).is_err());

    let unknown = vec![position("BTCUSDT", "0", 9)];
    assert!(order_position_idx(&unknown, OrderSide::Buy, false).is_err());
}

#[test]
fn parse_open_order_status_mapping() {
    let order =
        parse_open_order(order("777", "PartiallyFilled", "Limit", "Sell")).expect("order parses");
    assert_eq!(order.order_id, "777");
    assert!(matches!(order.side, OrderSide::Sell));
    assert!(matches!(order.order_type, OrderType::Limit));
    assert!(matches!(order.status, OrderStatus::PartiallyFilled));
    assert_eq!(order.fees, 0.08);
    assert_eq!(order.client_order_id, None);
    assert_eq!(order.reduce_only, Some(false));
}

#[test]
fn parse_open_order_uses_official_post_only_and_finality_evidence() {
    let raw = OpenOrderRow {
        time_in_force: "PostOnly".into(),
        leaves_qty: "0.7".into(),
        cancel_type: "CancelByUser".into(),
        reject_reason: "EC_NoError".into(),
        reduce_only: true,
        position_idx: 2,
        order_link_id: "bybit-cli-1".into(),
        ..order("777", "Cancelled", "Limit", "Sell")
    };
    let parsed = parse_open_order(raw).expect("post-only order parses");
    assert!(matches!(parsed.order_type, OrderType::PostOnly));
    assert!(matches!(parsed.status, OrderStatus::Canceled));
    assert_eq!(parsed.client_order_id.as_deref(), Some("bybit-cli-1"));
    assert_eq!(parsed.reduce_only, Some(true));
}

#[test]
fn parse_open_order_handles_market_blank_price_and_untriggered_status() {
    let raw = OpenOrderRow {
        price: String::new(),
        order_type: "Market".into(),
        order_status: "Untriggered".into(),
        cum_exec_qty: "0".into(),
        avg_price: String::new(),
        ..order("777", "New", "Limit", "Buy")
    };
    let parsed = parse_open_order(raw).expect("market order parses");
    assert_eq!(parsed.price, 0.0);
    assert_eq!(parsed.filled_price, 0.0);
    assert!(matches!(parsed.order_type, OrderType::Market));
    assert!(matches!(parsed.status, OrderStatus::Pending));
}

#[test]
fn parse_open_order_rejects_unknown_enums_and_bad_numbers() {
    assert!(parse_open_order(order("777", "Mystery", "Limit", "Buy")).is_err());
    assert!(parse_open_order(order("777", "Triggered", "Limit", "Buy")).is_err());
    assert!(parse_open_order(order("777", "New", "Iceberg", "Buy")).is_err());
    assert!(parse_open_order(order("777", "New", "Limit", "Hold")).is_err());

    let blank_limit_price = OpenOrderRow {
        price: String::new(),
        ..order("777", "New", "Limit", "Buy")
    };
    assert!(parse_open_order(blank_limit_price).is_err());

    let bad_qty = OpenOrderRow {
        qty: "bad".into(),
        ..order("777", "New", "Limit", "Buy")
    };
    assert!(parse_open_order(bad_qty).is_err());

    let bad_time = OpenOrderRow {
        created_time: "0".into(),
        ..order("777", "New", "Limit", "Buy")
    };
    assert!(parse_open_order(bad_time).is_err());

    let bad_leaves = OpenOrderRow {
        leaves_qty: "bad".into(),
        ..order("777", "New", "Limit", "Buy")
    };
    assert!(parse_open_order(bad_leaves).is_err());

    let bad_position_idx = OpenOrderRow {
        position_idx: 9,
        ..order("777", "New", "Limit", "Buy")
    };
    assert!(parse_open_order(bad_position_idx).is_err());
}

#[test]
fn bybit_private_strict_fixture_sweep_fails_closed() {
    for name in ["wallet_missing_coin", "wallet_missing_equity"] {
        assert!(
            serde_json::from_value::<crate::adapters::bybit_response::BybitResponse<
                WalletAccount,
            >>(strict_fixture_case(name))
            .is_err(),
            "{name} must fail closed"
        );
    }

    let wallet: crate::adapters::bybit_response::BybitResponse<WalletAccount> =
        serde_json::from_value(strict_fixture_case("wallet_non_finite"))
            .expect("non-finite wallet fixture decodes");
    let accounts = wallet
        .into_list("wallet")
        .expect("wallet response succeeds");
    assert!(parse_balance_response(accounts, None).is_err());

    for name in ["position_zero_non_finite", "position_unknown_side"] {
        let response: crate::adapters::bybit_response::BybitResponse<PositionRow> =
            serde_json::from_value(strict_fixture_case(name))
                .expect("position rejection fixture decodes");
        let positions = response
            .into_list("positions")
            .expect("position response succeeds");
        assert!(
            parse_positions(&positions).is_err(),
            "{name} must fail closed"
        );
    }

    assert!(
        serde_json::from_value::<crate::adapters::bybit_response::BybitResponse<OpenOrderRow>>(
            strict_fixture_case("order_missing_created_time")
        )
        .is_err()
    );

    for name in [
        "order_invalid_created_time",
        "order_unknown_status",
        "order_unknown_cancel_type",
    ] {
        let response: crate::adapters::bybit_response::BybitResponse<OpenOrderRow> =
            serde_json::from_value(strict_fixture_case(name))
                .expect("order rejection fixture decodes");
        let orders = response
            .into_list("orders")
            .expect("order response succeeds");
        assert!(
            parse_open_orders(orders).is_err(),
            "{name} must fail closed"
        );
    }
}

fn strict_fixture_case(name: &str) -> serde_json::Value {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/bybit/private_parser_strict_rejections.json"
    ))
    .expect("strict rejection fixture JSON");
    fixture
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("missing strict fixture case: {name}"))
}

fn wallet_account(
    account_type: &str,
    total_available_balance: &str,
    coin: Vec<CoinBalance>,
) -> WalletAccount {
    WalletAccount {
        account_type: account_type.into(),
        total_available_balance: total_available_balance.into(),
        total_equity: "100".into(),
        total_initial_margin: "10".into(),
        total_maintenance_margin: "5".into(),
        account_im_rate: "0.1".into(),
        account_mm_rate: "0.05".into(),
        coin,
    }
}

fn balance(
    coin: &str,
    equity: &str,
    _available: &str,
    locked: &str,
    unrealised_pnl: &str,
) -> CoinBalance {
    CoinBalance {
        coin: coin.into(),
        equity: equity.into(),
        locked: locked.into(),
        unrealised_pnl: unrealised_pnl.into(),
    }
}

fn position(symbol: &str, size: &str, position_idx: i32) -> PositionRow {
    PositionRow {
        symbol: symbol.into(),
        side: if size == "0" {
            String::new()
        } else {
            match position_idx {
                2 => "Sell".into(),
                _ => "Buy".into(),
            }
        },
        size: size.into(),
        avg_price: "30000".into(),
        mark_price: "31000".into(),
        unrealised_pnl: "10".into(),
        leverage: "2".into(),
        liq_price: "15000".into(),
        position_im: "100".into(),
        position_idx,
    }
}

fn order(order_id: &str, status: &str, order_type: &str, side: &str) -> OpenOrderRow {
    OpenOrderRow {
        symbol: "BTCUSDT".into(),
        order_id: order_id.into(),
        order_status: status.into(),
        order_type: order_type.into(),
        side: side.into(),
        price: "30000".into(),
        qty: "1".into(),
        cum_exec_qty: "0.3".into(),
        avg_price: "30000".into(),
        created_time: "1700000000000".into(),
        cum_exec_fee: "0.08".into(),
        position_idx: 1,
        cancel_type: "UNKNOWN".into(),
        reject_reason: "EC_NoError".into(),
        leaves_qty: "0.7".into(),
        time_in_force: "GTC".into(),
        order_link_id: String::new(),
        reduce_only: false,
    }
}

// PR-EM: official Bybit V5 REST response-envelope fixtures, exercised end-to-end
// through BybitResponse / BybitObjectResponse decoding + into_list / into_result
// + parse_open_order / ack_from_row. The struct-level tests above build
// OpenOrderRow directly and so skip the official `{retCode,retMsg,result}`
// envelope, the non-zero retCode fail-closed path, and the empty-list
// "order not found" path; these fixtures cover them.
fn bybit_get_order_envelope(list_body: &str, ret_code: i32) -> String {
    format!(
        r#"{{"retCode":{ret_code},"retMsg":"OK","result":{{"category":"linear","list":[{list_body}],"nextPageCursor":""}},"retExtInfo":{{}},"time":1700000000000}}"#
    )
}

const BYBIT_OFFICIAL_ORDER_ROW: &str = r#"{"symbol":"BTCUSDT","orderId":"1779020","orderLinkId":"bybit-cli-7","side":"Buy","orderType":"Limit","orderStatus":"Filled","price":"30000","qty":"1","cumExecQty":"1","avgPrice":"30000","cumExecFee":"0.55","createdTime":"1700000000000","positionIdx":1,"cancelType":"UNKNOWN","rejectReason":"EC_NoError","leavesQty":"0","timeInForce":"GTC","reduceOnly":false}"#;

fn bybit_cancel_envelope(ret_code: i32) -> String {
    format!(
        r#"{{"retCode":{ret_code},"retMsg":"OK","result":{{"orderId":"1779020","orderLinkId":"bybit-cli-7"}},"retExtInfo":{{}},"time":1700000000000}}"#
    )
}

#[test]
fn bybit_get_order_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/bybit/order_realtime_linear_open.json");
    let resp: crate::adapters::bybit_response::BybitResponse<OpenOrderRow> =
        serde_json::from_str(fixture).expect("envelope decodes");
    let mut rows = resp.into_list("get order").expect("retCode 0 yields rows");
    let parsed = rows
        .pop()
        .map(parse_open_order)
        .transpose()
        .expect("order parses")
        .expect("order present");
    assert_eq!(parsed.order_id, "fd4300ae-7847-404e-b947-b46980a4d140");
    assert_eq!(parsed.symbol, "ETH");
    assert!(matches!(parsed.status, OrderStatus::Open));
    assert!(matches!(parsed.side, OrderSide::Buy));
    assert_eq!(parsed.client_order_id, None);
    assert_eq!(parsed.reduce_only, Some(false));
    assert_eq!(parsed.quantity, 0.10);
    assert_eq!(parsed.fees, 0.0);
}

#[test]
fn bybit_position_mode_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/bybit/position_list_one_way.json");
    let resp: crate::adapters::bybit_response::BybitResponse<PositionRow> =
        serde_json::from_str(fixture).expect("envelope decodes");
    let rows = resp
        .into_list("position mode")
        .expect("retCode 0 yields rows");
    let mode = parse_position_mode(&rows).expect("positionIdx mode parses");

    assert_eq!(mode, PositionModeEvidence::OneWay);
    assert_eq!(mode.as_str(), "one_way");
}

#[test]
fn get_order_official_empty_list_is_not_found() {
    let text = bybit_get_order_envelope("", 0);
    let resp: crate::adapters::bybit_response::BybitResponse<OpenOrderRow> =
        serde_json::from_str(&text).expect("envelope decodes");
    let mut rows = resp.into_list("get order").expect("retCode 0 yields empty");
    let parsed = rows.pop().map(parse_open_order).transpose().expect("ok");
    assert!(parsed.is_none());
}

#[test]
fn get_order_envelope_rejects_nonzero_ret_code() {
    let text = bybit_get_order_envelope(BYBIT_OFFICIAL_ORDER_ROW, 110001);
    let resp: crate::adapters::bybit_response::BybitResponse<OpenOrderRow> =
        serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_list("get order").is_err());
}

#[test]
fn cancel_order_official_envelope_builds_ack() {
    let text = bybit_cancel_envelope(0);
    let resp: crate::adapters::bybit_response::BybitObjectResponse<
        crate::adapters::bybit_trade_data::OrderAckRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    let row = resp
        .into_result("cancel order")
        .expect("retCode 0 yields ack row");
    let ack = crate::adapters::bybit_trade_data::ack_from_row(
        "internal-1".to_owned(),
        "public-1".to_owned(),
        "venue-fallback".to_owned(),
        row,
        shared_types::LiveOrderState::Cancelled,
        None,
    );
    assert_eq!(ack.exchange_order_id.as_deref(), Some("1779020"));
    assert!(matches!(ack.state, shared_types::LiveOrderState::Cancelled));
}

#[test]
fn cancel_order_envelope_rejects_nonzero_ret_code() {
    let text = bybit_cancel_envelope(110001);
    let resp: crate::adapters::bybit_response::BybitObjectResponse<
        crate::adapters::bybit_trade_data::OrderAckRow,
    > = serde_json::from_str(&text).expect("envelope decodes");
    assert!(resp.into_result("cancel order").is_err());
}
