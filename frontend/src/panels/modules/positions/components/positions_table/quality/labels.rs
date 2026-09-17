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
        "markPrice" => "标记价缺证据",
        "fundingRate8h" => "Funding 缺证据",
        "nextFundingMs" => "结算时间缺证据",
        "liquidationPrice" => "强平价异常",
        "liquidationDistancePct" => "强平距离异常",
        "maintenanceMarginRatio" => "维持保证金缺证据",
        "margin" => "保证金缺证据",
        "leverage" => "杠杆缺证据",
        "positionMode" => "持仓模式缺证据",
        "marginMode" => "保证金模式缺证据",
        "positionMarginMode" => "逐全仓模式缺证据",
        "riskRate" => "风险率缺证据",
        "availablePosition" => "可平量缺证据",
        "frozenPosition" => "冻结量缺证据",
        _ => "字段缺证据",
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
