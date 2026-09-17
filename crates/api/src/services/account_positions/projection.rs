use super::*;

mod liquidation;

use liquidation::{liquidation_distance_quality, liquidation_price_quality};

pub(super) fn position_field_quality(
    rows: &[PositionInfo],
    observed_at_ms: i64,
) -> Vec<AccountFieldQuality> {
    rows.iter()
        .flat_map(|row| {
            let mut quality = Vec::new();
            quality.extend(mark_price_quality(row, observed_at_ms));
            quality.push(liquidation_price_quality(row, observed_at_ms));
            quality.push(liquidation_distance_quality(row, observed_at_ms));
            quality.extend(maintenance_margin_quality(row, observed_at_ms));
            quality.extend(margin_quality(row, observed_at_ms));
            quality.extend(leverage_quality(row, observed_at_ms));
            quality.extend(venue_semantic_quality(row, observed_at_ms));
            quality
        })
        .collect()
}

pub(super) fn public_ws_mark_price_quality(
    row: &PositionInfo,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    actual_position_field(row, "markPrice", "public_ws_mark_index", observed_at_ms)
}

fn mark_price_quality(row: &PositionInfo, observed_at_ms: i64) -> Option<AccountFieldQuality> {
    if positive_finite(row.mark_price) {
        return None;
    }
    if positive_finite(row.entry_price) {
        return Some(estimated_position_field(
            row,
            "markPrice",
            "position_entry_price_fallback",
            observed_at_ms,
        ));
    }
    Some(unavailable_position_field(
        row,
        "markPrice",
        AccountFieldQualityStatus::Invalid,
        observed_at_ms,
    ))
}
fn maintenance_margin_quality(
    row: &PositionInfo,
    observed_at_ms: i64,
) -> Option<AccountFieldQuality> {
    if shared_types::normalized_venue_name(&row.exchange) == "gate" {
        return Some(gate_maintenance_quality(row, observed_at_ms));
    }
    (!positive_finite(row.maintenance_margin_ratio)).then(|| {
        unavailable_position_field(
            row,
            "maintenanceMarginRatio",
            AccountFieldQualityStatus::Unknown,
            observed_at_ms,
        )
    })
}

fn gate_maintenance_quality(row: &PositionInfo, observed_at_ms: i64) -> AccountFieldQuality {
    let (status, source) = if !positive_finite(row.maintenance_margin_ratio) {
        (
            AccountFieldQualityStatus::Unknown,
            "gate_maintenance_unknown",
        )
    } else if row.margin_mode.is_none() {
        (
            AccountFieldQualityStatus::Estimated,
            "gate_ws_legacy_maintenance_rate",
        )
    } else {
        (
            AccountFieldQualityStatus::Actual,
            "gate_rest_average_maintenance_rate",
        )
    };
    let quality = AccountFieldQuality::new(
        AccountFieldSubject::position(&row.exchange, &row.symbol, &row.side),
        "maintenanceMarginRatio",
        status,
        source,
        Some(observed_at_ms),
    );
    if status == AccountFieldQualityStatus::Actual {
        quality
    } else {
        quality.with_problem(gate_maintenance_problem(
            row,
            status,
            source,
            observed_at_ms,
        ))
    }
}

fn gate_maintenance_problem(
    row: &PositionInfo,
    status: AccountFieldQualityStatus,
    source: &'static str,
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = position_field_problem(row, "maintenanceMarginRatio", status, observed_at_ms)
        .with_source(source);
    if let Some(details) = problem
        .details
        .as_mut()
        .and_then(serde_json::Value::as_object_mut)
    {
        details.insert(
            "source".to_owned(),
            serde_json::Value::String(source.to_owned()),
        );
    }
    problem
}
fn margin_quality(row: &PositionInfo, observed_at_ms: i64) -> Option<AccountFieldQuality> {
    if positive_finite(row.margin) {
        return None;
    }
    if positive_finite(row.leverage)
        && (positive_finite(row.mark_price) || positive_finite(row.entry_price))
    {
        return Some(estimated_position_field(
            row,
            "margin",
            "position_notional_over_leverage_estimate",
            observed_at_ms,
        ));
    }
    Some(unavailable_position_field(
        row,
        "margin",
        AccountFieldQualityStatus::Unknown,
        observed_at_ms,
    ))
}
fn leverage_quality(row: &PositionInfo, observed_at_ms: i64) -> Option<AccountFieldQuality> {
    (!positive_finite(row.leverage)).then(|| {
        unavailable_position_field(
            row,
            "leverage",
            AccountFieldQualityStatus::Invalid,
            observed_at_ms,
        )
    })
}

fn venue_semantic_quality(row: &PositionInfo, observed_at_ms: i64) -> Vec<AccountFieldQuality> {
    let mut quality = Vec::new();
    match shared_types::normalized_venue_name(&row.exchange).as_str() {
        "gate" => {
            quality.extend(mode_field_quality(row, observed_at_ms));
        }
        _ => {}
    }
    quality
}

fn mode_field_quality(row: &PositionInfo, observed_at_ms: i64) -> Vec<AccountFieldQuality> {
    let mut quality = Vec::new();
    if row.position_mode.is_none() {
        quality.push(unavailable_position_field(
            row,
            "positionMode",
            AccountFieldQualityStatus::Unknown,
            observed_at_ms,
        ));
    }
    if row.margin_mode.is_none() {
        quality.push(unavailable_position_field(
            row,
            "marginMode",
            AccountFieldQualityStatus::Unknown,
            observed_at_ms,
        ));
    }
    quality
}

fn positive_finite(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn unavailable_position_field(
    row: &PositionInfo,
    field: &'static str,
    status: AccountFieldQualityStatus,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    AccountFieldQuality::new(
        AccountFieldSubject::position(&row.exchange, &row.symbol, &row.side),
        field,
        status,
        POSITION_SOURCE,
        Some(observed_at_ms),
    )
    .with_problem(position_field_problem(row, field, status, observed_at_ms))
}

fn actual_position_field(
    row: &PositionInfo,
    field: &'static str,
    source: &str,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    AccountFieldQuality::new(
        AccountFieldSubject::position(&row.exchange, &row.symbol, &row.side),
        field,
        AccountFieldQualityStatus::Actual,
        source,
        Some(observed_at_ms),
    )
}

fn estimated_position_field(
    row: &PositionInfo,
    field: &'static str,
    source: &'static str,
    observed_at_ms: i64,
) -> AccountFieldQuality {
    AccountFieldQuality::new(
        AccountFieldSubject::position(&row.exchange, &row.symbol, &row.side),
        field,
        AccountFieldQualityStatus::Estimated,
        source,
        Some(observed_at_ms),
    )
    .with_problem(position_field_problem_with_source(
        row,
        field,
        AccountFieldQualityStatus::Estimated,
        source,
        observed_at_ms,
    ))
}

fn position_field_problem(
    row: &PositionInfo,
    field: &'static str,
    status: AccountFieldQualityStatus,
    observed_at_ms: i64,
) -> ApiProblem {
    position_field_problem_with_source(row, field, status, POSITION_SOURCE, observed_at_ms)
}

fn position_field_problem_with_source(
    row: &PositionInfo,
    field: &'static str,
    status: AccountFieldQualityStatus,
    source: &str,
    observed_at_ms: i64,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::POSITION_FIELD_UNAVAILABLE,
        format!("position field {field} is {status:?}"),
    )
    .with_status(StatusCode::OK.as_u16())
    .with_request_id(common::request_id::current())
    .with_source(source);
    problem.details = Some(serde_json::json!({
        "venue": row.exchange.as_str(), "symbol": row.symbol.as_str(), "side": row.side.as_str(), "field": field, "status": status,
        "operation": POSITION_OPERATION, "path": POSITION_ROUTE, "source": source, "observedAtMs": observed_at_ms,
    }));
    problem
}
