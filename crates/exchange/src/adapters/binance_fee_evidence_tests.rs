use super::*;

#[test]
fn parses_official_commission_fixture_without_zeroing_rates() {
    let body = include_str!("../../fixtures/binance/usdm_account_commission_rate_btcusdt.json");
    let row: BinanceCommissionRateResponse = serde_json::from_str(body).expect("fixture schema");
    let evidence = parse_commission_rate(&row, "btcusdt", 1_700_000_000_000)
        .expect("official commission evidence");

    assert_eq!(evidence.symbol, "BTCUSDT");
    assert_eq!(evidence.maker_commission_rate, 0.0002);
    assert_eq!(evidence.taker_commission_rate, 0.0004);
    assert_eq!(evidence.rpi_commission_rate, 0.00005);
    assert_eq!(evidence.source_url, BINANCE_COMMISSION_RATE_DOC_URL);
}

#[test]
fn preserves_signed_rebate_and_rejects_invalid_or_missing_rates() {
    let rebate: BinanceCommissionRateResponse = serde_json::from_str(
        r#"{"symbol":"BTCUSDT","makerCommissionRate":"-0.0001","takerCommissionRate":"0.0004","rpiCommissionRate":"0.00005"}"#,
    )
    .expect("rebate schema");
    let parsed = parse_commission_rate(&rebate, "BTCUSDT", 1).expect("signed rebate");
    assert_eq!(parsed.maker_commission_rate, -0.0001);

    let missing =
        r#"{"symbol":"BTCUSDT","makerCommissionRate":"0.0002","takerCommissionRate":"0.0004"}"#;
    assert!(serde_json::from_str::<BinanceCommissionRateResponse>(missing).is_err());

    let invalid: BinanceCommissionRateResponse = serde_json::from_str(
        r#"{"symbol":"BTCUSDT","makerCommissionRate":"NaN","takerCommissionRate":"0.0004","rpiCommissionRate":"0.00005"}"#,
    )
    .expect("invalid numeric schema");
    assert!(parse_commission_rate(&invalid, "BTCUSDT", 1).is_err());
}

#[test]
fn rejects_symbol_mismatch_and_nonpositive_evidence_time() {
    let body = include_str!("../../fixtures/binance/usdm_account_commission_rate_btcusdt.json");
    let row: BinanceCommissionRateResponse = serde_json::from_str(body).expect("fixture schema");
    assert!(parse_commission_rate(&row, "ETHUSDT", 1).is_err());

    let row: BinanceCommissionRateResponse = serde_json::from_str(body).expect("fixture schema");
    assert!(parse_commission_rate(&row, "BTCUSDT", 0).is_err());
}
