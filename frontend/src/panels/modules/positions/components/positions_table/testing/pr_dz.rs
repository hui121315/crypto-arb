use super::*;

#[test]
fn funding_quality_rows_keeps_rate_and_settlement_time_evidence() {
    let rows = vec![
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "fundingRate8h",
            AccountFieldQualityStatus::Missing,
        ),
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "nextFundingMs",
            AccountFieldQualityStatus::Missing,
        ),
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "markPrice",
            AccountFieldQualityStatus::Invalid,
        ),
    ];

    let filtered = funding_quality_rows(&rows);

    assert_eq!(filtered.len(), 2);
    assert_eq!(filtered[0].field, "fundingRate8h");
    assert_eq!(filtered[1].field, "nextFundingMs");
}

#[test]
fn unknown_liquidation_severity_is_visibly_distinct_from_ok() {
    assert_eq!(severity_class(PositionSeverity::Unknown), "unknown-row");
    assert_eq!(severity_class(PositionSeverity::Ok), "");
}
