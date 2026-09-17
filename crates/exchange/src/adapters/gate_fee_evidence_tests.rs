use super::*;

#[test]
fn parses_official_fee_fixture_and_preserves_maker_rebate() {
    let body = include_str!("../../fixtures/gate/futures_usdt_fee.json");
    let rows: GateFuturesFeeResponse = serde_json::from_str(body).expect("fixture schema");

    let parsed =
        parse_futures_fee_evidence(rows, "usdt", 1_700_000_000_000).expect("fee evidence parses");
    let btc = parsed
        .iter()
        .find(|row| row.contract == "BTC_USDT")
        .expect("BTC fee row");

    assert_eq!(btc.settle, "USDT");
    assert_eq!(btc.maker_fee_rate, -0.0001);
    assert_eq!(btc.taker_fee_rate, 0.00025);
    assert_eq!(btc.source_url, GATE_FUTURES_FEE_DOC_URL);
}

#[test]
fn cache_is_settle_scoped_and_expiry_bounded() {
    let body = include_str!("../../fixtures/gate/futures_usdt_fee.json");
    let rows: GateFuturesFeeResponse = serde_json::from_str(body).expect("fixture schema");
    let parsed = parse_futures_fee_evidence(rows, "usdt", 1_000).expect("fee evidence");
    let cache = GateFuturesFeeCache::default();
    cache.replace(parsed);

    assert_eq!(
        cache
            .fresh("usdt", "btc_usdt", 2_000)
            .map(|row| row.maker_fee_rate),
        Some(-0.0001)
    );
    assert!(cache.fresh("btc", "BTC_USDT", 2_000).is_none());
    assert!(cache
        .fresh("usdt", "BTC_USDT", 1_000 + FEE_EVIDENCE_TTL_MS)
        .is_none());

    let replacement: GateFuturesFeeResponse =
        serde_json::from_str(r#"{"SOL_USDT":{"maker_fee":"-0.0001","taker_fee":"0.00025"}}"#)
            .expect("replacement schema");
    cache.replace(
        parse_futures_fee_evidence(replacement, "usdt", 2_000).expect("replacement evidence"),
    );
    assert!(cache.fresh("usdt", "BTC_USDT", 2_001).is_none());
    assert!(cache.fresh("usdt", "SOL_USDT", 2_001).is_some());
}

#[test]
fn rejects_non_finite_or_implausible_rates() {
    let rows: GateFuturesFeeResponse =
        serde_json::from_str(r#"{"BTC_USDT":{"maker_fee":"NaN","taker_fee":"0.00025"}}"#)
            .expect("fixture schema");
    assert!(parse_futures_fee_evidence(rows, "usdt", 1).is_err());

    let rows: GateFuturesFeeResponse =
        serde_json::from_str(r#"{"BTC_USDT":{"maker_fee":"-1.1","taker_fee":"0.00025"}}"#)
            .expect("fixture schema");
    assert!(parse_futures_fee_evidence(rows, "usdt", 1).is_err());
}
