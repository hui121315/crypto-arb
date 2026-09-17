use super::*;
use shared_types::{OrderSide, OrderStatus, OrderType};

#[test]
fn capabilities_reflect_demo_or_live_mode() {
    let demo = must_adapter(true);
    let live = must_adapter(false);

    assert!(demo.capabilities().supports_testnet);
    assert!(!demo.capabilities().supports_live);
    assert!(!live.capabilities().supports_testnet);
    assert!(live.capabilities().supports_live);
}

#[test]
fn live_headers_do_not_include_simulated_trading() {
    let demo_headers = must_adapter(true).headers("GET", "/api/v5/account/balance", "");
    let live_headers = must_adapter(false).headers("GET", "/api/v5/account/balance", "");

    assert!(has_simulated_trading_header(&demo_headers));
    assert!(!has_simulated_trading_header(&live_headers));
}

#[test]
fn parse_live_order_row() {
    let row: OrderRow = match serde_json::from_value(serde_json::json!({
        "instId": "BTC-USDT-SWAP",
        "ordId": "123",
        "clOrdId": "client",
        "state": "partially_filled",
        "ordType": "post_only",
        "side": "sell",
        "px": "50000",
        "sz": "0.01",
        "accFillSz": "0.005",
        "avgPx": "49990",
        "cTime": "1700000000000"
    })) {
        Ok(row) => row,
        Err(error) => panic!("okx live order row: {error}"),
    };
    let parsed = parse_order(row).expect("order parses");
    assert_eq!(parsed.symbol, "BTC");
    assert_eq!(parsed.order_id, "123");
    assert!(matches!(parsed.status, OrderStatus::PartiallyFilled));
    assert!(matches!(parsed.order_type, OrderType::PostOnly));
    assert!(matches!(parsed.side, OrderSide::Sell));
}

#[test]
fn parse_live_order_accepts_market_blank_price_and_rejects_limit_blank_price() {
    let market = order_row("optimal_limit_ioc", "filled", "buy", "");
    let parsed = parse_order(market).expect("market order parses");
    assert!(matches!(parsed.order_type, OrderType::Market));
    assert_eq!(parsed.price, 0.0);

    let limit = order_row("limit", "live", "buy", "");
    let error = parse_order(limit).expect_err("blank limit price rejected");
    assert!(error.to_string().contains("px"));
}

#[test]
fn parse_live_order_rejects_unknown_side_type_status() {
    for row in [
        order_row("limit", "live", "hold", "1"),
        order_row("mmp", "live", "buy", "1"),
        order_row("limit", "mystery", "buy", "1"),
    ] {
        assert!(parse_order(row).is_err());
    }
}

#[test]
fn parse_live_order_rejects_bad_numeric_and_timestamp() {
    assert!(parse_order(order_row_with(OrderFixture {
        sz: "bad",
        ..OrderFixture::default()
    }))
    .is_err());

    assert!(parse_order(order_row_with(OrderFixture {
        c_time: "bad",
        ..OrderFixture::default()
    }))
    .is_err());
}

#[test]
fn parse_live_balances_rejects_bad_numeric() {
    let rows = vec![balance_item("USDT", "bad", "1", "0", "")];
    let error = parse_balances(rows, None).expect_err("bad balance rejected");
    assert!(error.to_string().contains("eq"));
}

#[test]
fn parse_live_account_read_preserves_official_equity_summary() {
    let rows = vec![serde_json::from_value(serde_json::json!({
        "totalEq": "102.5",
        "availEq": "80.0",
        "imr": "12.0",
        "mmr": "1.5",
        "details": [{
            "ccy": "USDT",
            "eq": "100",
            "availBal": "80",
            "frozenBal": "20",
            "upl": "2.5"
        }]
    }))
    .expect("account balance item")];

    let read = parse_account_read(rows, None, 1_700_000_000_000).expect("account read");
    assert_eq!(read.balances.len(), 1);
    assert_eq!(read.summaries[0].total_equity_usd, 102.5);
    assert_eq!(read.summaries[0].total_initial_margin_usd, 12.0);
    assert_eq!(read.summaries[0].total_maintenance_margin_usd, 1.5);
}

#[test]
fn parse_live_positions_uses_pos_side_and_rejects_bad_numeric() {
    let short = position_row(PositionFixture {
        pos_side: "short",
        ..PositionFixture::default()
    });
    let parsed = parse_positions(&[short], None).expect("positions parse");
    assert_eq!(parsed[0].side, "short");

    let bad = position_row(PositionFixture {
        avg_px: "bad",
        ..PositionFixture::default()
    });
    assert!(parse_positions(&[bad], None).is_err());
}

#[test]
fn parse_live_positions_allows_blank_optional_fields() {
    let row = position_row(PositionFixture {
        lever: "",
        liq_px: "",
        imr: "",
        ..PositionFixture::default()
    });
    let parsed = parse_positions(&[row], None).expect("positions parse");
    assert_eq!(parsed[0].leverage, 1.0);
    assert_eq!(parsed[0].liquidation_price, None);
    assert_eq!(parsed[0].margin, 0.0);
}

#[test]
fn parse_account_config_position_mode() {
    let net: AccountConfigRow = serde_json::from_value(serde_json::json!({
        "posMode": "net_mode"
    }))
    .expect("net account config");
    assert_eq!(parse_position_mode(&net).unwrap(), OkxPositionMode::Net);

    let long_short: AccountConfigRow = serde_json::from_value(serde_json::json!({
        "posMode": "long_short_mode"
    }))
    .expect("long-short account config");
    assert_eq!(
        parse_position_mode(&long_short).unwrap(),
        OkxPositionMode::LongShort
    );
}

#[test]
fn okx_account_config_parses_official_fixture_position_mode() {
    let body = include_str!("../../fixtures/okx/account_config_long_short.json");
    let rows =
        crate::adapters::okx_response::data_from_text::<AccountConfigRow>(body, "account config")
            .expect("official account-config envelope parses");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        parse_position_mode(&rows[0]).expect("position mode parses"),
        OkxPositionMode::LongShort
    );
}

#[test]
fn parse_account_config_rejects_unknown_position_mode() {
    let row: AccountConfigRow = serde_json::from_value(serde_json::json!({
        "posMode": "mystery"
    }))
    .expect("account config");
    assert!(parse_position_mode(&row).is_err());
}

#[test]
fn order_margin_modes_follow_configured_td_mode() {
    assert_eq!(
        must_adapter(false).order_margin_modes(),
        vec![MarginMode::Cross]
    );
    assert_eq!(
        adapter_with_td_mode(OkxTdMode::Isolated).order_margin_modes(),
        vec![MarginMode::Isolated]
    );
    assert!(adapter_with_td_mode(OkxTdMode::Cash)
        .order_margin_modes()
        .is_empty());
}

fn adapter_with_td_mode(td_mode: OkxTdMode) -> OkxLive {
    OkxLive::new(OkxLiveConfig {
        credentials: OkxLiveCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
            passphrase: "pass".into(),
        },
        testnet: false,
        timeout_secs: 1,
        qps: 1,
        base_url_override: Some("http://localhost".into()),
        td_mode,
    })
    .unwrap()
}

fn must_adapter(testnet: bool) -> OkxLive {
    OkxLive::new(OkxLiveConfig {
        credentials: OkxLiveCredentials {
            api_key: "key".into(),
            api_secret: "secret".into(),
            passphrase: "pass".into(),
        },
        testnet,
        timeout_secs: 1,
        qps: 1,
        base_url_override: Some("http://localhost".into()),
        td_mode: OkxTdMode::Cross,
    })
    .unwrap()
}

fn has_simulated_trading_header(headers: &[(String, String)]) -> bool {
    headers
        .iter()
        .any(|(key, value)| key == "x-simulated-trading" && value == "1")
}

fn order_row(ord_type: &str, state: &str, side: &str, px: &str) -> OrderRow {
    order_row_with(OrderFixture {
        ord_type,
        state,
        side,
        px,
        ..OrderFixture::default()
    })
}

#[derive(Clone, Copy)]
struct OrderFixture<'a> {
    ord_type: &'a str,
    state: &'a str,
    side: &'a str,
    px: &'a str,
    sz: &'a str,
    acc_fill_sz: &'a str,
    avg_px: &'a str,
    c_time: &'a str,
}

impl Default for OrderFixture<'_> {
    fn default() -> Self {
        Self {
            ord_type: "limit",
            state: "live",
            side: "buy",
            px: "1",
            sz: "0.01",
            acc_fill_sz: "0",
            avg_px: "",
            c_time: "1700000000000",
        }
    }
}

fn order_row_with(fixture: OrderFixture<'_>) -> OrderRow {
    serde_json::from_value(serde_json::json!({
        "instId": "BTC-USDT-SWAP",
        "ordId": format!("{}-{}-{}", fixture.ord_type, fixture.state, fixture.side),
        "clOrdId": "client",
        "state": fixture.state,
        "ordType": fixture.ord_type,
        "side": fixture.side,
        "px": fixture.px,
        "sz": fixture.sz,
        "accFillSz": fixture.acc_fill_sz,
        "avgPx": fixture.avg_px,
        "cTime": fixture.c_time
    }))
    .expect("order row")
}

fn balance_item(ccy: &str, eq: &str, avail: &str, frozen: &str, upl: &str) -> AccountBalanceItem {
    serde_json::from_value(serde_json::json!({
        "details": [{
            "ccy": ccy,
            "eq": eq,
            "availBal": avail,
            "frozenBal": frozen,
            "upl": upl
        }]
    }))
    .expect("balance item")
}

#[derive(Clone, Copy)]
struct PositionFixture<'a> {
    inst_id: &'a str,
    pos: &'a str,
    pos_side: &'a str,
    avg_px: &'a str,
    lever: &'a str,
    liq_px: &'a str,
    imr: &'a str,
}

impl Default for PositionFixture<'_> {
    fn default() -> Self {
        Self {
            inst_id: "BTC-USDT-SWAP",
            pos: "0.5",
            pos_side: "net",
            avg_px: "30000",
            lever: "2",
            liq_px: "15000",
            imr: "100",
        }
    }
}

fn position_row(fixture: PositionFixture<'_>) -> PositionRow {
    serde_json::from_value(serde_json::json!({
        "instId": fixture.inst_id,
        "pos": fixture.pos,
        "posSide": fixture.pos_side,
        "avgPx": fixture.avg_px,
        "markPx": "31000",
        "upl": "10",
        "lever": fixture.lever,
        "liqPx": fixture.liq_px,
        "imr": fixture.imr
    }))
    .expect("position row")
}

// PR-CU: official OKX V5 get-order REST response-envelope fixtures.
// Source: <https://www.okx.com/docs-v5/en/#order-book-trading-trade-get-order-details>
// `GET /api/v5/trade/order` wraps a single order in {code,msg,data:[...]}; an
// empty `data` array is the documented "order not found" shape and must map to
// `None` rather than a parse error.
#[test]
fn okx_get_order_parses_official_fixture() {
    let body = include_str!("../../fixtures/okx/trade_get_order_filled.json");
    let mut rows = crate::adapters::okx_response::data_from_text::<OrderRow>(body, "get order")
        .expect("official get-order envelope parses");
    assert_eq!(rows.len(), 1, "single order expected");
    let parsed = parse_order(rows.remove(0)).expect("order parses");
    assert_eq!(parsed.order_id, "680800019749904384");
    assert_eq!(parsed.symbol, "BTC");
    assert_eq!(parsed.quantity, 100.0);
    assert_eq!(parsed.filled_quantity, 0.00192834);
    assert_eq!(parsed.filled_price, 51858.0);
    assert_eq!(parsed.price, 0.0);
    assert!(matches!(parsed.side, OrderSide::Buy));
    assert!(matches!(parsed.order_type, OrderType::Market));
    assert!(matches!(parsed.status, OrderStatus::Filled));
}

#[test]
fn get_order_empty_data_envelope_means_not_found() {
    let body = r#"{"code":"0","msg":"","data":[]}"#;
    let rows = crate::adapters::okx_response::data_from_text::<OrderRow>(body, "get order")
        .expect("empty get-order envelope parses");
    assert!(rows.is_empty(), "empty data must map to no order (None)");
}

#[test]
fn get_order_maps_live_51603_response_to_not_found() {
    let missing = ExchangeError::Api {
        exchange: "okx".to_owned(),
        code: "51603".to_owned(),
        message: "Order does not exist".to_owned(),
    };
    let permission_error = ExchangeError::Api {
        exchange: "okx".to_owned(),
        code: "50120".to_owned(),
        message: "permission denied".to_owned(),
    };

    assert!(okx_order_not_found(&missing));
    assert!(!okx_order_not_found(&permission_error));
}
