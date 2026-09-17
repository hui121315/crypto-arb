use super::*;
use pretty_assertions::assert_eq;
use shared_types::{OrderSide, OrderStatus, OrderType};

#[test]
fn parse_balance_response_filters_currency() {
    let rows = vec![AccountBalanceItem {
        details: vec![
            balance("USDT", "100", "90", "5", "2"),
            balance("BTC", "1", "0.5", "0.1", "0"),
        ],
    }];
    let parsed = parse_balance_response(rows, Some("USDT")).expect("balance parses");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed["USDT"].available, 90.0);
    assert_eq!(parsed["USDT"].frozen, 5.0);
    assert_eq!(parsed["USDT"].unrealized_pnl, 2.0);
}

#[test]
fn parse_balance_response_rejects_bad_numeric() {
    let rows = vec![AccountBalanceItem {
        details: vec![balance("USDT", "bad", "90", "5", "")],
    }];
    let error = parse_balance_response(rows, Some("USDT")).expect_err("bad eq rejected");
    assert!(error.to_string().contains("eq"));
}

#[test]
fn okx_account_balance_parses_official_fixture() {
    let body = include_str!("../../fixtures/okx/account_balance_usdt.json");
    let rows: Vec<AccountBalanceItem> =
        crate::adapters::okx_response::data_from_text(body, "account balance")
            .expect("official account-balance envelope decodes");
    let parsed = parse_balance_response(rows, Some("USDT")).expect("account balance parses");
    let usdt = parsed.get("USDT").expect("USDT balance present");

    assert_eq!(parsed.len(), 1);
    assert_eq!(usdt.total, 10_000.0);
    assert_eq!(usdt.available, 9_000.0);
    assert_eq!(usdt.frozen, 1_000.0);
    assert_eq!(usdt.unrealized_pnl, 5.0);
}

#[test]
fn parse_positions_skips_zero_and_filters_target() {
    let rows = vec![
        position("BTC-USDT-SWAP", "0.5"),
        position("ETH-USDT-SWAP", "0"),
        position("SOL-USDT-SWAP", "-2"),
    ];
    let parsed = parse_positions(&rows, Some("BTC-USDT-SWAP")).expect("positions parse");
    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].symbol, "BTC");
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[0].quantity, 0.5);
}

#[test]
fn okx_positions_parse_official_fixture() {
    let body = include_str!("../../fixtures/okx/account_positions_swap.json");
    let rows: Vec<PositionRow> =
        crate::adapters::okx_response::data_from_text(body, "account positions")
            .expect("official positions envelope decodes");
    let parsed = parse_positions(&rows, Some("BTC-USDT-SWAP")).expect("positions parse");

    assert_eq!(parsed.len(), 1);
    assert_eq!(parsed[0].symbol, "BTC");
    assert_eq!(parsed[0].side, "long");
    assert_eq!(parsed[0].quantity, 0.5);
    assert_eq!(parsed[0].leverage, 5.0);
}

#[test]
fn parse_positions_uses_pos_side_before_sign() {
    let mut row = position("BTC-USDT-SWAP", "0.5");
    row.pos_side = "short".into();
    let parsed = parse_positions(&[row], None).expect("positions parse");
    assert_eq!(parsed[0].side, "short");
}

#[test]
fn parse_positions_rejects_bad_nonzero_numeric() {
    let mut row = position("BTC-USDT-SWAP", "0.5");
    row.mark_px = "bad".into();
    let error = parse_positions(&[row], None).expect_err("bad mark rejected");
    assert!(error.to_string().contains("markPx"));

    let mut signed_hedge = position("BTC-USDT-SWAP", "-0.5");
    signed_hedge.pos_side = "long".into();
    assert!(parse_positions(&[signed_hedge], None).is_err());
}

#[test]
fn parse_positions_rejects_blank_leverage_and_margin_evidence() {
    let mut row = position("BTC-USDT-SWAP", "0.5");
    row.lever.clear();
    row.liq_px.clear();
    row.imr.clear();
    assert!(parse_positions(&[row], None).is_err());
}

#[test]
fn parse_open_orders_maps_side_type_and_status() {
    let raw = OpenOrderItem {
        inst_id: "BTC-USDT-SWAP".into(),
        ord_id: "987".into(),
        state: "live".into(),
        ord_type: "post_only".into(),
        side: "sell".into(),
        px: "30000".into(),
        sz: "0.5".into(),
        acc_fill_sz: "0.25".into(),
        avg_px: "30000".into(),
        c_time: "1700000000000".into(),
        cl_ord_id: "cli-okx-1".into(),
        reduce_only: "true".into(),
        fee: "-0.01".into(),
    };
    let parsed = parse_open_orders(vec![raw]).expect("orders parse");
    let order = &parsed[0];
    assert_eq!(order.order_id, "987");
    assert_eq!(order.symbol, "BTC");
    assert!(matches!(order.side, OrderSide::Sell));
    assert!(matches!(order.order_type, OrderType::PostOnly));
    assert!(matches!(order.status, OrderStatus::Open));
    assert_eq!(order.filled_quantity, 0.25);
    assert_eq!(order.client_order_id.as_deref(), Some("cli-okx-1"));
    assert_eq!(order.reduce_only, Some(true));
}

#[test]
fn okx_open_orders_parses_official_fixture() {
    let body = include_str!("../../fixtures/okx/trade_orders_pending.json");
    let rows: Vec<OpenOrderItem> =
        crate::adapters::okx_response::data_from_text(body, "orders pending")
            .expect("official orders-pending envelope decodes");
    let parsed = parse_open_orders(rows).expect("official open-orders list parses");
    let order = parsed.first().expect("open order present");

    assert_eq!(parsed.len(), 1);
    assert_eq!(order.order_id, "590908157585625111");
    assert_eq!(order.symbol, "BTC");
    assert!(matches!(order.side, OrderSide::Buy));
    assert!(matches!(order.order_type, OrderType::Limit));
    assert!(matches!(order.status, OrderStatus::Open));
    assert_eq!(order.client_order_id.as_deref(), Some("okx-cli-1"));
    assert_eq!(order.reduce_only, Some(false));
    assert_eq!(order.filled_quantity, 0.25);
    assert_eq!(order.filled_price, 29990.0);
}

#[test]
fn parse_open_orders_maps_okx_limit_like_and_market_types() {
    let ioc = open_order("ioc", "live", "buy", "30000");
    let fok = open_order("fok", "partially_filled", "sell", "30001");
    let market = open_order("optimal_limit_ioc", "filled", "buy", "");
    let parsed = parse_open_orders(vec![ioc, fok, market]).expect("orders parse");
    assert!(matches!(parsed[0].order_type, OrderType::Limit));
    assert!(matches!(parsed[1].order_type, OrderType::Limit));
    assert!(matches!(parsed[2].order_type, OrderType::Market));
    assert_eq!(parsed[2].price, 0.0);
    assert_eq!(parsed[0].client_order_id, None);
    assert_eq!(parsed[0].reduce_only, Some(false));
}

#[test]
fn parse_open_orders_rejects_unknown_side_type_status() {
    let cases = [
        open_order("limit", "live", "hold", "30000"),
        open_order("mmp", "live", "buy", "30000"),
        open_order("limit", "mystery", "buy", "30000"),
    ];
    for row in cases {
        assert!(parse_open_orders(vec![row]).is_err());
    }
}

#[test]
fn parse_open_orders_rejects_bad_numeric_and_timestamp() {
    let mut bad_qty = open_order("limit", "live", "buy", "30000");
    bad_qty.sz = "bad".into();
    assert!(parse_open_orders(vec![bad_qty]).is_err());

    let mut bad_time = open_order("limit", "live", "buy", "30000");
    bad_time.c_time = "bad".into();
    assert!(parse_open_orders(vec![bad_time]).is_err());

    let blank_limit_price = open_order("limit", "live", "buy", "");
    assert!(parse_open_orders(vec![blank_limit_price]).is_err());
}

#[test]
fn okx_private_strict_fixture_sweep_fails_closed() {
    for name in [
        "balance_missing_details",
        "balance_missing_available_balance",
    ] {
        let body = strict_fixture_case(name).to_string();
        assert!(
            crate::adapters::okx_response::data_from_text::<AccountBalanceItem>(&body, "balance")
                .is_err(),
            "{name} must fail closed"
        );
    }

    let non_finite_balance = strict_fixture_case("balance_non_finite").to_string();
    let balances: Vec<AccountBalanceItem> =
        crate::adapters::okx_response::data_from_text(&non_finite_balance, "balance")
            .expect("non-finite balance fixture decodes");
    assert!(parse_balance_response(balances, None).is_err());

    for name in ["position_zero_non_finite", "position_unknown_side"] {
        let body = strict_fixture_case(name).to_string();
        let positions: Vec<PositionRow> =
            crate::adapters::okx_response::data_from_text(&body, "position")
                .expect("position rejection fixture decodes");
        assert!(
            parse_positions(&positions, None).is_err(),
            "{name} must fail closed"
        );
    }

    let missing_timestamp = strict_fixture_case("order_missing_created_time").to_string();
    assert!(
        crate::adapters::okx_response::data_from_text::<OpenOrderItem>(&missing_timestamp, "order")
            .is_err()
    );

    for name in [
        "order_invalid_created_time",
        "order_unknown_state",
        "order_unknown_reduce_only",
    ] {
        let body = strict_fixture_case(name).to_string();
        let orders: Vec<OpenOrderItem> =
            crate::adapters::okx_response::data_from_text(&body, "order")
                .expect("order rejection fixture decodes");
        assert!(
            parse_open_orders(orders).is_err(),
            "{name} must fail closed"
        );
    }
}

fn strict_fixture_case(name: &str) -> serde_json::Value {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/okx/private_parser_strict_rejections.json"
    ))
    .expect("strict rejection fixture JSON");
    fixture
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("missing strict fixture case: {name}"))
}

fn balance(ccy: &str, eq: &str, avail: &str, frozen: &str, upl: &str) -> BalanceDetail {
    BalanceDetail {
        ccy: ccy.into(),
        eq: eq.into(),
        avail_bal: avail.into(),
        frozen_bal: frozen.into(),
        upl: upl.into(),
    }
}

fn position(inst_id: &str, pos: &str) -> PositionRow {
    PositionRow {
        inst_id: inst_id.into(),
        pos: pos.into(),
        avg_px: "30000".into(),
        mark_px: "31000".into(),
        upl: "10".into(),
        lever: "2".into(),
        liq_px: "15000".into(),
        imr: "100".into(),
        pos_side: "net".into(),
    }
}

fn open_order(ord_type: &str, state: &str, side: &str, price: &str) -> OpenOrderItem {
    OpenOrderItem {
        inst_id: "BTC-USDT-SWAP".into(),
        ord_id: format!("{ord_type}-{state}-{side}"),
        state: state.into(),
        ord_type: ord_type.into(),
        side: side.into(),
        px: price.into(),
        sz: "0.5".into(),
        acc_fill_sz: "0".into(),
        avg_px: "".into(),
        c_time: "1700000000000".into(),
        cl_ord_id: "".into(),
        reduce_only: "false".into(),
        fee: "0".into(),
    }
}
