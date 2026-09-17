use super::*;

pub(super) fn liquidation_distance_label(row: &PositionRow) -> String {
    row.liquidation_distance_pct.map_or_else(
        || {
            if row.liquidation_price == Some(0.0) {
                "交易所 --".to_owned()
            } else {
                "距离未知".to_owned()
            }
        },
        |distance| format!("{distance:.1}%"),
    )
}

pub(super) fn liquidation_price_label(value: Option<f64>) -> String {
    match value {
        Some(value) if value > 0.0 => price(value),
        Some(_) => "强平价 --".to_owned(),
        None => "强平价未知".to_owned(),
    }
}

pub(super) fn close_button_title(
    has_pair: bool,
    requires_live: bool,
    live_ready: bool,
) -> &'static str {
    if requires_live && !live_ready {
        "请先在设置的执行环境中两步启用实盘"
    } else if has_pair {
        "reduce-only 市价平两条配对腿"
    } else {
        "reduce-only 市价平仓"
    }
}

pub(super) fn pair_risk_label(row: &PositionRow, rows: &[PositionRow], has_pair: bool) -> String {
    if !has_pair {
        return String::new();
    }
    pair_liquidation_risk(row, rows).map_or_else(
        || "双边风险待证".to_owned(),
        |risk| {
            if risk.evidence_complete {
                format!("双边最小 {:.1}% · {}", risk.distance_pct, risk.venue)
            } else {
                format!(
                    "已知腿 {:.1}% · {} · 另腿待证",
                    risk.distance_pct, risk.venue
                )
            }
        },
    )
}
