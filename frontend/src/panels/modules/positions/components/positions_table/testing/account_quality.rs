use super::super::quality::{
    bounded_overflow_quality, position_margin_quality_rows, position_overflow_quality_rows,
};
use super::quality;
use shared_types::AccountFieldQualityStatus;

#[test]
fn leverage_margin_and_overflow_quality_have_explicit_destinations() {
    let rows = vec![
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "leverage",
            AccountFieldQualityStatus::Estimated,
        ),
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "margin",
            AccountFieldQualityStatus::Unknown,
        ),
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "maintenanceMarginRatio",
            AccountFieldQualityStatus::Estimated,
        ),
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "riskRate",
            AccountFieldQualityStatus::Missing,
        ),
    ];

    let margin = position_margin_quality_rows(&rows);
    let overflow = position_overflow_quality_rows(&rows);

    assert_eq!(margin.len(), 2);
    assert!(margin.iter().any(|row| row.field == "margin"));
    assert!(margin
        .iter()
        .any(|row| row.field == "maintenanceMarginRatio"));
    assert_eq!(overflow.len(), 1);
    assert_eq!(overflow[0].field, "riskRate");
}

#[test]
fn overflow_quality_keeps_four_inline_and_discloses_the_rest() {
    let rows = (0..6)
        .map(|index| {
            quality(
                "binance",
                "BTCUSDT",
                "long",
                &format!("extraField{index}"),
                AccountFieldQualityStatus::Unknown,
            )
        })
        .collect::<Vec<_>>();

    let (inline, overflow) = bounded_overflow_quality(&rows);

    assert_eq!(inline.len(), 4);
    assert_eq!(overflow.len(), 2);
}
