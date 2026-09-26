use super::*;

#[test]
fn position_quality_for_row_keeps_current_position_attention_only() {
    let target = row("binance", "BTCUSDT");
    let rows = vec![
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "liquidationPrice",
            AccountFieldQualityStatus::Missing,
        ),
        quality(
            "okx",
            "BTCUSDT",
            "long",
            "liquidationPrice",
            AccountFieldQualityStatus::Missing,
        ),
        quality(
            "binance",
            "ETHUSDT",
            "long",
            "markPrice",
            AccountFieldQualityStatus::Invalid,
        ),
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "markPrice",
            AccountFieldQualityStatus::Actual,
        ),
    ];

    let filtered = position_quality_for_row(&target, &rows);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].field, "liquidationPrice");
}

#[test]
fn position_quality_keeps_actual_liquidation_source_but_not_other_actual_fields() {
    let target = row("binance", "BTCUSDT");
    let rows = vec![
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "liquidationDistancePct",
            AccountFieldQualityStatus::Actual,
        ),
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "markPrice",
            AccountFieldQualityStatus::Actual,
        ),
    ];

    let filtered = position_quality_for_row(&target, &rows);

    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].field, "liquidationDistancePct");
}

#[test]
fn liquidation_quality_rows_keeps_liquidation_fields() {
    let rows = vec![
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "liquidationPrice",
            AccountFieldQualityStatus::Missing,
        ),
        quality(
            "binance",
            "BTCUSDT",
            "long",
            "liquidationDistancePct",
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

    let filtered = liquidation_quality_rows(&rows);

    assert_eq!(filtered.len(), 2);
    assert!(filtered
        .iter()
        .all(|row| row.field.starts_with("liquidation")));
}

#[test]
fn binance_zero_liquidation_evidence_uses_exchange_style_placeholder_copy() {
    let mut price = quality(
        "binance",
        "ETHUSDT",
        "long",
        "liquidationPrice",
        AccountFieldQualityStatus::Actual,
    );
    price.source = "binance_position_liquidation_price_zero".to_owned();
    let mut distance = quality(
        "binance",
        "ETHUSDT",
        "long",
        "liquidationDistancePct",
        AccountFieldQualityStatus::Actual,
    );
    distance.source = "binance_position_liquidation_price_zero_no_distance".to_owned();

    assert_eq!(field_quality_label(&price), "币安 --");
    assert_eq!(field_quality_label(&distance), "距离不适用");
}

#[test]
fn mark_quality_masks_mark_and_unrealized_pnl_values() {
    let quality = quality(
        "binance",
        "BTCUSDT",
        "long",
        "markPrice",
        AccountFieldQualityStatus::Missing,
    );

    assert_eq!(
        value_or_missing(Some(&quality), "$100.00".to_owned()),
        "数据待确认"
    );
    let pnl = pnl_display(12.0, Some(&quality));

    assert_eq!(pnl.value, "数据待确认");
    assert_eq!(pnl.class, "muted");
}
