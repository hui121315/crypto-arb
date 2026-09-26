use shared_types::{AccountFieldQuality, AccountFieldQualityStatus};

pub(in crate::panels::modules::positions) fn field_quality_title(
    row: &AccountFieldQuality,
) -> String {
    let mut parts = vec![
        field_quality_label(row).to_owned(),
        field_quality_status_label(row.status).to_owned(),
        row.source.clone(),
    ];
    if let Some(problem) = row.problem.as_ref() {
        parts.push(problem.code.clone());
        parts.push(problem.message.clone());
    }
    parts.join(" · ")
}

pub(in crate::panels::modules::positions) fn field_quality_label(
    row: &AccountFieldQuality,
) -> &'static str {
    match (row.field.as_str(), row.source.as_str()) {
        ("liquidationPrice", "binance_position_liquidation_price_zero") => "币安 --",
        ("liquidationDistancePct", "binance_position_liquidation_price_zero_no_distance") => {
            "距离不适用"
        }
        _ => field_quality_status_label_for_field(&row.field, row.status),
    }
}

fn field_quality_status_label_for_field(
    field: &str,
    status: AccountFieldQualityStatus,
) -> &'static str {
    match field {
        "liquidationPrice" if status == AccountFieldQualityStatus::Actual => "交易所强平价",
        "liquidationDistancePct" if status == AccountFieldQualityStatus::Actual => "交易所距离",
        "liquidationDistancePct" if status == AccountFieldQualityStatus::Estimated => "估算距离",
        "liquidationPrice" if status == AccountFieldQualityStatus::Missing => "强平价不可用",
        "liquidationDistancePct" if status == AccountFieldQualityStatus::Missing => {
            "强平距离不可用"
        }
        "markPrice" => "标记价数据待确认",
        "fundingRate8h" => "资金费 数据待确认",
        "nextFundingMs" => "结算时间数据待确认",
        "liquidationPrice" => "强平价异常",
        "liquidationDistancePct" => "强平距离异常",
        "maintenanceMarginRatio" => "维持保证金数据待确认",
        "margin" => "保证金数据待确认",
        "leverage" => "杠杆数据待确认",
        "positionMode" => "持仓模式数据待确认",
        "marginMode" => "保证金模式数据待确认",
        "positionMarginMode" => "逐全仓模式数据待确认",
        "riskRate" => "风险率数据待确认",
        "availablePosition" => "可平量数据待确认",
        "frozenPosition" => "冻结量数据待确认",
        _ => "字段数据待确认",
    }
}

fn field_quality_status_label(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "OK",
        AccountFieldQualityStatus::Estimated => "EST",
        AccountFieldQualityStatus::Unknown => "UNKNOWN",
        AccountFieldQualityStatus::Invalid => "INVALID",
        AccountFieldQualityStatus::Missing => "MISSING",
    }
}

pub(in crate::panels::modules::positions) fn field_quality_status_class(
    status: AccountFieldQualityStatus,
) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "ok",
        AccountFieldQualityStatus::Estimated => "warn",
        AccountFieldQualityStatus::Unknown => "unknown",
        AccountFieldQualityStatus::Invalid | AccountFieldQualityStatus::Missing => "blocked",
    }
}
