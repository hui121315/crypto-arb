use shared_types::{normalized_venue_name, PositionInfo};

pub(super) fn merge_position_rows(current: &mut Vec<PositionInfo>, updates: Vec<PositionInfo>) {
    for mut row in updates {
        let previous = current
            .iter()
            .find(|existing| same_position(existing, &row))
            .cloned();
        current.retain(|existing| !same_position(existing, &row));
        if row.quantity.is_finite() && row.quantity > 0.0 {
            preserve_rest_projection_fields(&mut row, previous.as_ref());
            current.push(row);
        }
    }
}

fn preserve_rest_projection_fields(row: &mut PositionInfo, previous: Option<&PositionInfo>) {
    let Some(previous) = previous else {
        return;
    };
    if !positive_finite(row.mark_price) && positive_finite(previous.mark_price) {
        row.mark_price = previous.mark_price;
    }
    if !positive_finite(row.leverage) && positive_finite(previous.leverage) {
        row.leverage = previous.leverage;
    }
    if row.liquidation_price.is_none() {
        row.liquidation_price = previous.liquidation_price;
    }
    if row.liquidation_distance_pct.is_none() {
        row.liquidation_distance_pct = previous.liquidation_distance_pct;
    }
    if row.next_funding_ms.is_none() {
        row.next_funding_ms = previous.next_funding_ms;
    }
    if row.paired_with.is_none() {
        row.paired_with.clone_from(&previous.paired_with);
    }
    if !has_real_maintenance_ratio(row) && has_real_maintenance_ratio(previous) {
        row.maintenance_margin_ratio = previous.maintenance_margin_ratio;
    }
    if row.risk_rate.is_none() {
        row.risk_rate = previous.risk_rate;
    }
    if row.available_position.is_none() {
        row.available_position = previous.available_position;
    }
    if row.frozen_position.is_none() {
        row.frozen_position = previous.frozen_position;
    }
}

fn has_real_maintenance_ratio(row: &PositionInfo) -> bool {
    positive_finite(row.maintenance_margin_ratio)
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn same_position(left: &PositionInfo, right: &PositionInfo) -> bool {
    left.symbol.eq_ignore_ascii_case(&right.symbol)
        && left.side.eq_ignore_ascii_case(&right.side)
        && normalized_venue_name(&left.exchange) == normalized_venue_name(&right.exchange)
}
