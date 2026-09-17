use super::*;
use shared_types::{ExecutionMode, OrderSource};

#[test]
fn parse_balances_uses_non_available_total_as_frozen() {
    let items: Vec<BalanceItem> = serde_json::from_str(
        r#"[{
            "asset":"USDT",
            "balance":"100",
            "availableBalance":"75",
            "crossUnPnl":"1.5"
        }]"#,
    )
    .expect("test balance json is valid");

    let balances = parse_balances(items, Some("usdt")).expect("balances parse");
    let usdt = balances.get("USDT").expect("USDT balance exists");

    assert_eq!(usdt.total, 100.0);
    assert_eq!(usdt.available, 75.0);
    assert_eq!(usdt.frozen, 25.0);
    assert_eq!(usdt.unrealized_pnl, 1.5);
}

#[test]
fn binance_balance_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_balance_v3.json");
    let items: Vec<BalanceItem> =
        serde_json::from_str(fixture).expect("official binance USDM balance fixture parses");
    let balances = parse_balances(items, Some("USDT")).expect("balances parse");
    let usdt = balances.get("USDT").expect("USDT balance exists");

    assert_eq!(usdt.total, 122_607.351_379_03);
    assert_eq!(usdt.available, 23.724_692_06);
    assert_eq!(usdt.frozen, 122_583.626_686_97);
    assert_eq!(usdt.unrealized_pnl, 0.0);
}

#[test]
fn binance_account_v3_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_account_v3.json");
    let item: AccountInfoV3 =
        serde_json::from_str(fixture).expect("official Binance USDM account V3 fixture parses");
    let read = parse_account_info_v3(item, Some("USDT"), 1_700_000_000_000)
        .expect("account information parses");

    let usdt = read.balances.get("USDT").expect("USDT balance exists");
    assert_eq!(usdt.total, 23.724_692_06);
    assert_eq!(usdt.available, 23.724_692_06);
    assert_eq!(usdt.frozen, 0.0);
    assert_eq!(read.summary.total_equity_usd, 126.724_692_06);
    assert_eq!(read.summary.total_available_balance_usd, 126.724_692_06);
    assert_eq!(read.summary.withdrawable_balance_usd, Some(126.724_692_06));
    assert_eq!(read.summary.equity_scope, AccountEquityScope::Perpetuals);
    assert_eq!(read.summary.source, "binance.GET /fapi/v3/account");
}

#[test]
fn parse_positions_pairs_hedge_mode_long_and_short() {
    let items: Vec<PositionItem> = serde_json::from_str(
        r#"[
            {
                "symbol":"BTCUSDT",
                "positionAmt":"0.5",
                "entryPrice":"100",
                "markPrice":"110",
                "unRealizedProfit":"5",
                "liquidationPrice":"50",
                "notional":"55",
                "positionInitialMargin":"27.5",
                "maintMargin":"1.375",
                "positionSide":"LONG"
            },
            {
                "symbol":"BTCUSDT",
                "positionAmt":"0.3",
                "entryPrice":"100",
                "markPrice":"90",
                "unRealizedProfit":"-3",
                "liquidationPrice":"150",
                "notional":"-27",
                "positionInitialMargin":"13.5",
                "maintMargin":"0.675",
                "positionSide":"SHORT"
            }
        ]"#,
    )
    .expect("test position json is valid");

    let positions = parse_positions(items, Some("BTCUSDT")).expect("positions parse");

    assert_eq!(positions.len(), 2);
    assert!(positions
        .iter()
        .any(|p| p.side == "long" && p.paired_with.as_deref() == Some("binance:BTC:short")));
    assert!(positions
        .iter()
        .any(|p| p.side == "short" && p.paired_with.as_deref() == Some("binance:BTC:long")));
}

#[test]
fn binance_positions_parse_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_position_risk_btcusdt.json");
    let items: Vec<PositionItem> =
        serde_json::from_str(fixture).expect("official binance USDM positions fixture parses");
    let positions = parse_positions(items, Some("BTCUSDT")).expect("positions parse");

    assert_eq!(positions.len(), 2);
    assert!(positions
        .iter()
        .any(|p| p.side == "long" && (p.quantity - 0.5).abs() < 1e-12));
    assert!(positions
        .iter()
        .any(|p| p.side == "short" && (p.quantity - 0.25).abs() < 1e-12));
    assert!(positions.iter().all(|p| p.paired_with.as_deref().is_some()));
    assert!(positions.iter().all(|p| (p.leverage - 10.0).abs() < 1e-12));
    assert!(positions.iter().all(|p| p.margin > 0.0));
    assert!(positions
        .iter()
        .all(|p| (p.maintenance_margin_ratio - 0.05).abs() < 1e-12));
}

#[test]
fn position_risk_rows_provide_account_mode_without_an_extra_request() {
    let hedge_items: Vec<PositionItem> = serde_json::from_str(include_str!(
        "../../fixtures/binance/usdm_position_risk_btcusdt.json"
    ))
    .expect("official Binance position fixture parses");
    let hedge = parse_positions_with_mode(hedge_items, Some("BTCUSDT"))
        .expect("hedge positions and mode parse");

    assert_eq!(hedge.mode, Some(BinancePositionMode::Hedge));
    assert_eq!(hedge.rows.len(), 2);

    let one_way_items: Vec<PositionItem> = serde_json::from_str(
        r#"[{"symbol":"ETHUSDT","positionAmt":"0.25","entryPrice":"3500","markPrice":"3600","unRealizedProfit":"25","liquidationPrice":"0","notional":"900","positionInitialMargin":"450","maintMargin":"22.5","positionSide":"BOTH"}]"#,
    )
    .expect("documented one-way position shape parses");
    let one_way = parse_positions_with_mode(one_way_items, Some("ETHUSDT"))
        .expect("one-way position and mode parse");

    assert_eq!(one_way.mode, Some(BinancePositionMode::OneWay));
}

#[test]
fn all_flat_mixed_position_rows_leave_account_mode_unproven() {
    let items: Vec<PositionItem> = serde_json::from_str(
        r#"[
            {"symbol":"BTCUSDT","positionAmt":"0","entryPrice":"0","markPrice":"100","unRealizedProfit":"0","liquidationPrice":"0","notional":"0","positionInitialMargin":"0","maintMargin":"0","positionSide":"BOTH"},
            {"symbol":"ETHUSDT","positionAmt":"0","entryPrice":"0","markPrice":"100","unRealizedProfit":"0","liquidationPrice":"0","notional":"0","positionInitialMargin":"0","maintMargin":"0","positionSide":"LONG"}
        ]"#,
    )
    .expect("mixed test fixture decodes");

    let parsed = parse_positions_with_mode(items, None)
        .expect("flat rows remain valid even when they cannot prove account mode");

    assert_eq!(parsed.mode, None);
    assert!(parsed.rows.is_empty());
}

#[test]
fn binance_open_position_preserves_documented_zero_liquidation_price() {
    let items: Vec<PositionItem> = serde_json::from_str(
        r#"[{
            "symbol":"ETHUSDT",
            "positionAmt":"0.25",
            "entryPrice":"3500",
            "markPrice":"3600",
            "unRealizedProfit":"25",
            "liquidationPrice":"0",
            "notional":"900",
            "positionInitialMargin":"450",
            "maintMargin":"22.5",
            "positionSide":"BOTH"
        }]"#,
    )
    .expect("documented Binance position shape is valid");

    let positions = parse_positions(items, Some("ETHUSDT")).expect("position parses");

    assert_eq!(positions.len(), 1);
    assert_eq!(positions[0].liquidation_price, Some(0.0));
}

#[test]
fn parse_balances_rejects_bad_numeric_field() {
    let items: Vec<BalanceItem> = serde_json::from_str(
        r#"[{
            "asset":"USDT",
            "balance":"bad",
            "availableBalance":"75",
            "crossUnPnl":"1.5"
        }]"#,
    )
    .expect("test balance json is valid");

    let err = parse_balances(items, Some("USDT")).expect_err("bad balance must fail");

    assert!(matches!(
        err,
        ExchangeError::Parse(message) if message.contains("balance")
    ));
}

#[test]
fn parse_positions_validates_zero_placeholder_fields_before_skipping() {
    let items: Vec<PositionItem> = serde_json::from_str(
        r#"[{
            "symbol":"BTCUSDT",
            "positionAmt":"0",
            "entryPrice":"bad",
            "markPrice":"bad",
            "unRealizedProfit":"bad",
            "liquidationPrice":"bad",
            "notional":"bad",
            "positionInitialMargin":"bad",
            "maintMargin":"bad",
            "positionSide":"BOTH"
        }]"#,
    )
    .expect("test position json is valid");

    let error = parse_positions(items, Some("BTCUSDT"))
        .expect_err("zero placeholder with malformed official fields must fail closed");

    assert!(matches!(error, ExchangeError::Parse(_)));
}

#[test]
fn parse_positions_rejects_bad_nonzero_numeric_field() {
    let items: Vec<PositionItem> = serde_json::from_str(
        r#"[{
            "symbol":"BTCUSDT",
            "positionAmt":"0.5",
            "entryPrice":"bad",
            "markPrice":"110",
            "unRealizedProfit":"5",
            "liquidationPrice":"50",
            "notional":"55",
            "positionInitialMargin":"27.5",
            "maintMargin":"1.375",
            "positionSide":"LONG"
        }]"#,
    )
    .expect("test position json is valid");

    let err = parse_positions(items, Some("BTCUSDT")).expect_err("bad entry price must fail");

    assert!(matches!(
        err,
        ExchangeError::Parse(message) if message.contains("entryPrice")
    ));
}

#[test]
fn parse_open_order_rejects_bad_numeric_field() {
    let item = open_order_with_price("bad");

    let err = parse_open_order(&item).expect_err("bad price must fail");

    assert!(matches!(
        err,
        ExchangeError::Parse(message) if message.contains("price")
    ));
}

#[test]
fn binance_open_orders_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_open_orders.json");
    let items: Vec<OpenOrderItem> =
        serde_json::from_str(fixture).expect("official binance USDM open-orders fixture parses");
    let parsed = items
        .iter()
        .map(parse_open_order)
        .collect::<ExchangeResult<Vec<_>>>()
        .expect("official binance open-orders list maps to OrderInfo");
    let order = parsed.first().expect("open order present");

    assert_eq!(parsed.len(), 1);
    assert_eq!(order.order_id, "1917641");
    assert_eq!(order.symbol, "BTC");
    assert_eq!(order.status, OrderStatus::Open);
    assert_eq!(order.client_order_id.as_deref(), Some("binance-cli-1"));
    assert_eq!(order.reduce_only, Some(false));
    assert_eq!(order.quantity, 0.40);
    assert_eq!(order.filled_quantity, 0.10);
}

#[test]
fn parse_open_order_surfaces_client_order_id_and_reduce_only() {
    let mut item = open_order_with_price("100");
    item.client_order_id = "binance-cli-1".into();
    item.reduce_only = true;

    let order = parse_open_order(&item).expect("valid open order");

    assert_eq!(order.client_order_id.as_deref(), Some("binance-cli-1"));
    assert_eq!(order.reduce_only, Some(true));
}

#[test]
fn parse_open_order_collapses_blank_client_order_id() {
    let order = parse_open_order(&open_order_with_price("100")).expect("valid open order");

    assert_eq!(order.client_order_id, None);
    assert_eq!(order.reduce_only, Some(false));
}

#[test]
fn official_private_envelopes_reject_missing_required_fields() {
    let balance = r#"[{"asset":"USDT","balance":"1","crossUnPnl":"0"}]"#;
    assert!(serde_json::from_str::<Vec<BalanceItem>>(balance).is_err());

    let position = r#"[{"symbol":"BTCUSDT","positionAmt":"1","entryPrice":"1","markPrice":"1","unRealizedProfit":"0","liquidationPrice":"0","notional":"1","maintMargin":"0.05","positionSide":"BOTH"}]"#;
    assert!(serde_json::from_str::<Vec<PositionItem>>(position).is_err());

    let order = r#"{"orderId":1,"symbol":"BTCUSDT","status":"NEW","type":"LIMIT","side":"BUY","price":"1","origQty":"1","executedQty":"0","avgPrice":"0","time":1700000000000,"timeInForce":"GTC","clientOrderId":"cid"}"#;
    assert!(serde_json::from_str::<OpenOrderItem>(order).is_err());
}

#[test]
fn binance_private_strict_fixture_sweep_fails_closed() {
    assert!(
        serde_json::from_value::<Vec<BalanceItem>>(strict_fixture_case(
            "balance_missing_available_balance"
        ))
        .is_err()
    );

    let balances: Vec<BalanceItem> =
        serde_json::from_value(strict_fixture_case("balance_non_finite"))
            .expect("non-finite balance fixture decodes");
    assert!(parse_balances(balances, None).is_err());

    for name in [
        "position_non_finite_mark_price",
        "position_unknown_side",
        "position_negative_liquidation_price",
    ] {
        let positions: Vec<PositionItem> = serde_json::from_value(strict_fixture_case(name))
            .expect("position rejection fixture decodes");
        assert!(
            parse_positions(positions, None).is_err(),
            "{name} must fail closed"
        );
    }

    for name in [
        "order_missing_time",
        "order_invalid_time",
        "order_invalid_update_time",
    ] {
        let orders: Vec<OpenOrderItem> = serde_json::from_value(strict_fixture_case(name))
            .expect("timestamp rejection fixture decodes");
        assert!(
            parse_open_order(&orders[0]).is_err(),
            "{name} must fail closed"
        );
    }

    let orders: Vec<OpenOrderItem> =
        serde_json::from_value(strict_fixture_case("order_legacy_limit_maker"))
            .expect("legacy type fixture decodes");
    assert!(parse_open_order(&orders[0]).is_err());
}

fn strict_fixture_case(name: &str) -> serde_json::Value {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../fixtures/binance/private_parser_strict_rejections.json"
    ))
    .expect("strict rejection fixture JSON");
    fixture
        .get(name)
        .cloned()
        .unwrap_or_else(|| panic!("missing strict fixture case: {name}"))
}

#[test]
fn parse_position_mode_maps_official_dual_side_field() {
    let one_way = parse_position_mode(&PositionSideDualResponse {
        dual_side_position: false,
    });
    let hedge = parse_position_mode(&PositionSideDualResponse {
        dual_side_position: true,
    });

    assert_eq!(one_way, BinancePositionMode::OneWay);
    assert_eq!(hedge, BinancePositionMode::Hedge);
}

#[test]
fn binance_position_mode_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_position_side_dual.json");
    let row: PositionSideDualResponse =
        serde_json::from_str(fixture).expect("official binance USDM position mode fixture parses");
    let mode = parse_position_mode(&row);

    assert_eq!(mode, BinancePositionMode::Hedge);
}

#[test]
fn position_mode_side_mapping_is_fail_closed() {
    let mut order = intent(OrderSide::Buy, false);
    assert_eq!(
        BinancePositionMode::OneWay.position_side_for_intent(&order),
        "BOTH"
    );
    assert_eq!(
        BinancePositionMode::Hedge.position_side_for_intent(&order),
        "LONG"
    );

    order.side = OrderSide::Sell;
    assert_eq!(
        BinancePositionMode::Hedge.position_side_for_intent(&order),
        "SHORT"
    );

    order.reduce_only = true;
    assert_eq!(
        BinancePositionMode::Hedge.position_side_for_intent(&order),
        "LONG"
    );
    order.side = OrderSide::Buy;
    assert_eq!(
        BinancePositionMode::Hedge.position_side_for_intent(&order),
        "SHORT"
    );
}

fn open_order_with_price(price: &str) -> OpenOrderItem {
    OpenOrderItem {
        order_id: 1,
        symbol: "BTCUSDT".into(),
        status: "NEW".into(),
        order_type: "LIMIT".into(),
        side: "BUY".into(),
        price: price.into(),
        orig_qty: "0.01".into(),
        executed_qty: "0".into(),
        avg_price: "0".into(),
        time: Some(1_700_000_000_000),
        update_time: None,
        time_in_force: "GTC".into(),
        client_order_id: String::new(),
        reduce_only: false,
    }
}

fn intent(side: OrderSide, reduce_only: bool) -> OrderIntent {
    OrderIntent {
        id: "int-1".into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "binance".into(),
        symbol: "BTC".into(),
        side,
        order_type: OrderType::Limit,
        quantity: 0.01,
        price: Some(50_000.0),
        slippage_tolerance_bps: None,
        reduce_only,
        time_in_force: shared_types::TimeInForce::Gtc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "cid-1".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

#[test]
fn binance_get_order_parses_official_fixture() {
    let fixture = include_str!("../../fixtures/binance/usdm_get_order_filled.json");
    let item: OpenOrderItem =
        serde_json::from_str(fixture).expect("official binance USDM query-order fixture parses");
    let info = parse_open_order(&item).expect("official binance get-order maps to OrderInfo");
    assert_eq!(info.order_id, "283194212");
    assert_eq!(info.status, OrderStatus::Filled);
}
