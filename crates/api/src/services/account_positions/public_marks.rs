use super::projection::public_ws_mark_price_quality;
use super::*;

pub(super) fn apply_fresh_public_marks(
    state: &AppState,
    rows: &mut [PositionInfo],
    now_ms: i64,
) -> Vec<AccountFieldQuality> {
    rows.iter_mut()
        .filter_map(|row| {
            let (mark, observed_at_ms) =
                state
                    .market_data()
                    .fresh_ws_mark_index(&row.exchange, &row.symbol, now_ms)?;
            apply_public_mark(row, mark.mark_price);
            Some(public_ws_mark_price_quality(row, observed_at_ms))
        })
        .collect()
}

pub(super) fn apply_public_mark(row: &mut PositionInfo, mark_price: f64) {
    if !mark_price.is_finite() || mark_price <= 0.0 {
        return;
    }
    row.mark_price = mark_price;
    row.liquidation_distance_pct = row
        .liquidation_price
        .filter(|price| price.is_finite() && *price > 0.0)
        .and_then(|liquidation_price| {
            let signed_distance =
                if row.side.eq_ignore_ascii_case("long") || row.side.eq_ignore_ascii_case("buy") {
                    mark_price - liquidation_price
                } else if row.side.eq_ignore_ascii_case("short")
                    || row.side.eq_ignore_ascii_case("sell")
                {
                    liquidation_price - mark_price
                } else {
                    return None;
                };
            Some((signed_distance / mark_price) * 100.0)
        });
}
