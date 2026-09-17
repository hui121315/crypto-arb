//! Margin-cell and overflow field-quality routing for position rows.

use shared_types::AccountFieldQuality;

const INLINE_OVERFLOW_QUALITY_LIMIT: usize = 4;

pub(in crate::panels::modules::positions) fn position_margin_quality_rows(
    rows: &[AccountFieldQuality],
) -> Vec<AccountFieldQuality> {
    rows.iter()
        .filter(|row| {
            matches!(
                row.field.as_str(),
                "margin" | "maintenanceMarginRatio" | "marginMode" | "positionMarginMode"
            )
        })
        .cloned()
        .collect()
}

pub(in crate::panels::modules::positions) fn position_overflow_quality_rows(
    rows: &[AccountFieldQuality],
) -> Vec<AccountFieldQuality> {
    rows.iter()
        .filter(|row| !quality_has_dedicated_cell(&row.field))
        .cloned()
        .collect()
}

fn quality_has_dedicated_cell(field: &str) -> bool {
    matches!(
        field,
        "markPrice"
            | "fundingRate8h"
            | "liquidationPrice"
            | "liquidationDistancePct"
            | "leverage"
            | "margin"
            | "maintenanceMarginRatio"
            | "marginMode"
            | "positionMarginMode"
    )
}

pub(in crate::panels::modules::positions) fn bounded_overflow_quality(
    rows: &[AccountFieldQuality],
) -> (Vec<AccountFieldQuality>, Vec<AccountFieldQuality>) {
    let split = rows.len().min(INLINE_OVERFLOW_QUALITY_LIMIT);
    (rows[..split].to_vec(), rows[split..].to_vec())
}
