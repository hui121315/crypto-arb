use super::*;

const BINANCE_ZERO_LIQUIDATION_SOURCE: &str = "binance_position_liquidation_price_zero";
const BINANCE_ZERO_DISTANCE_SOURCE: &str = "binance_position_liquidation_price_zero_no_distance";

pub(super) fn liquidation_price_quality(
    row: &PositionInfo,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    match row.liquidation_price {
        Some(value) if value == 0.0 && is_binance(row) => actual_position_field(
            row,
            "liquidationPrice",
            BINANCE_ZERO_LIQUIDATION_SOURCE,
            observed_at_ms,
        ),
        Some(value) if positive_finite(value) => actual_position_field(
            row,
            "liquidationPrice",
            &format!(
                "{}_position_liquidation_price",
                shared_types::normalized_venue_name(&row.exchange)
            ),
            observed_at_ms,
        ),
        Some(_) => unavailable_position_field(
            row,
            "liquidationPrice",
            AccountFieldQualityStatus::Invalid,
            observed_at_ms,
        ),
        None => unavailable_position_field(
            row,
            "liquidationPrice",
            AccountFieldQualityStatus::Missing,
            observed_at_ms,
        ),
    }
}

pub(super) fn liquidation_distance_quality(
    row: &PositionInfo,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    match row.liquidation_distance_pct {
        _ if row.liquidation_price == Some(0.0) && is_binance(row) => actual_position_field(
            row,
            "liquidationDistancePct",
            BINANCE_ZERO_DISTANCE_SOURCE,
            observed_at_ms,
        ),
        Some(value)
            if value.is_finite()
                && shared_types::normalized_venue_name(&row.exchange) == "gate" =>
        {
            actual_position_field(
                row,
                "liquidationDistancePct",
                "gate_rest_liq_price_derived_distance",
                observed_at_ms,
            )
        }
        Some(value) if value.is_finite() => actual_position_field(
            row,
            "liquidationDistancePct",
            &format!(
                "{}_position_liquidation_distance",
                shared_types::normalized_venue_name(&row.exchange)
            ),
            observed_at_ms,
        ),
        _ if row.liquidation_price.is_some_and(positive_finite)
            && positive_finite(row.mark_price) =>
        {
            estimated_position_field(
                row,
                "liquidationDistancePct",
                "position_liquidation_price_derived_distance",
                observed_at_ms,
            )
        }
        Some(_) => unavailable_position_field(
            row,
            "liquidationDistancePct",
            AccountFieldQualityStatus::Invalid,
            observed_at_ms,
        ),
        None => unavailable_position_field(
            row,
            "liquidationDistancePct",
            AccountFieldQualityStatus::Missing,
            observed_at_ms,
        ),
    }
}

fn is_binance(row: &PositionInfo) -> bool {
    shared_types::normalized_venue_name(&row.exchange) == "binance"
}
