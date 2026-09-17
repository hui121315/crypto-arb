use super::*;

#[test]
fn parses_official_my_trades_fixture_without_combining_fee_units() {
    let body = include_str!("../../fixtures/gate/futures_usdt_my_trades_order.json");
    let rows: Vec<GateMyTradeRow> = serde_json::from_str(body).expect("fixture schema");

    let fills = parse_my_trades(rows, "usdt", "21893289839").expect("fill evidence");

    assert_eq!(fills.len(), 2);
    assert_eq!(fills[0].trade_id, "121234231");
    assert_eq!(fills[0].order_id, "21893289839");
    assert_eq!(fills[0].contract, "BTC_USDT");
    assert_eq!(fills[0].fee, 0.01);
    assert_eq!(fills[0].fee_currency, "USDT");
    assert_eq!(fills[0].point_fee, 0.0);
    assert_eq!(fills[0].occurred_at_ms, 1_514_764_800_123);
    assert_eq!(fills[1].fee, -0.0025);
    assert_eq!(fills[1].point_fee, 0.4);
    assert_eq!(fills[1].role, GateLiquidityRole::Maker);
}

#[test]
fn rejects_fill_from_a_different_order() {
    let body = include_str!("../../fixtures/gate/futures_usdt_my_trades_order.json");
    let rows: Vec<GateMyTradeRow> = serde_json::from_str(body).expect("fixture schema");

    let error = parse_my_trades(rows, "usdt", "7").expect_err("order mismatch must fail");

    assert!(error.to_string().contains("order mismatch"));
}

#[test]
fn rejects_unverified_fee_settle() {
    let body = include_str!("../../fixtures/gate/futures_usdt_my_trades_order.json");
    let rows: Vec<GateMyTradeRow> = serde_json::from_str(body).expect("fixture schema");

    let error =
        parse_my_trades(rows, "usd", "21893289839").expect_err("unsupported settle must fail");

    assert!(error.to_string().contains("unsupported settle evidence"));
}

#[test]
fn rejects_null_native_fee_instead_of_coercing_zero() {
    let body = r#"[{"id":1,"create_time":1,"contract":"BTC_USDT","order_id":"7","size":"1","price":"1","fee":null,"point_fee":"0","role":"taker"}]"#;
    let rows: Vec<GateMyTradeRow> = serde_json::from_str(body).expect("fixture schema");

    let error = parse_my_trades(rows, "usdt", "7").expect_err("null fee must fail");

    assert!(error.to_string().contains("my_trades.fee"));
}
