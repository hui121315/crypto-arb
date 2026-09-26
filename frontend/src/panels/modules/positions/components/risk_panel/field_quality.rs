//! 持仓字段质量证据的筛选、文案与 chip 视图。
//! 风险快照/VaR/限额视图见父模块 `risk_panel.rs`。

use leptos::prelude::*;
use shared_types::{AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubjectKind};

pub(super) fn render_position_field_quality(rows: &[AccountFieldQuality]) -> AnyView {
    if rows.is_empty() {
        return ().into_any();
    }
    let total = rows.len();
    view! {
        <details class="risk-evidence-disclosure">
            <summary>
                <span>"字段数据依据待核"</span>
                <em>{format!("{total} 项")}</em>
            </summary>
            <div class="balance-evidence">
                {rows.iter().map(position_quality_chip).collect_view()}
            </div>
        </details>
    }
    .into_any()
}

fn position_quality_chip(row: &AccountFieldQuality) -> impl IntoView {
    let class = format!(
        "balance-evidence-chip {}",
        position_quality_status_class(row.status)
    );
    let title = position_quality_title(row);
    view! {
        <span class=class title=title>
            {position_quality_subject(row)}
            " · "
            {position_field_label(&row.field)}
            " · "
            {position_quality_status_label(row.status)}
        </span>
    }
}

pub(super) fn position_field_quality_rows(
    rows: Vec<AccountFieldQuality>,
) -> Vec<AccountFieldQuality> {
    rows.into_iter()
        .filter(|row| row.subject.kind == AccountFieldSubjectKind::Position)
        .filter(|row| row.status != AccountFieldQualityStatus::Actual)
        .collect()
}

fn position_quality_title(row: &AccountFieldQuality) -> String {
    let mut parts = vec![
        position_quality_subject(row),
        row.field.clone(),
        position_quality_status_label(row.status).to_owned(),
        row.source.clone(),
    ];
    if let Some(problem) = row.problem.as_ref() {
        parts.push(problem.code.clone());
        parts.push(problem.message.clone());
    }
    parts.join(" · ")
}

fn position_quality_subject(row: &AccountFieldQuality) -> String {
    let venue = row.subject.venue.as_deref().unwrap_or("持仓");
    match row.subject.symbol.as_deref() {
        Some(symbol) => format!("{venue} {symbol}"),
        None => venue.to_owned(),
    }
}

fn position_field_label(field: &str) -> String {
    match field {
        "markPrice" => "标记价".to_owned(),
        "fundingRate8h" => "资金费".to_owned(),
        "liquidationPrice" => "强平价".to_owned(),
        "liquidationDistancePct" => "强平距离".to_owned(),
        "maintenanceMarginRatio" => "维持保证金".to_owned(),
        "margin" => "保证金".to_owned(),
        "leverage" => "杠杆".to_owned(),
        "positionMode" => "持仓模式".to_owned(),
        "marginMode" => "保证金模式".to_owned(),
        "positionMarginMode" => "逐全仓模式".to_owned(),
        "riskRate" => "风险率".to_owned(),
        "availablePosition" => "可平量".to_owned(),
        "frozenPosition" => "冻结量".to_owned(),
        _ => field.to_owned(),
    }
}

fn position_quality_status_label(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "OK",
        AccountFieldQualityStatus::Estimated => "EST",
        AccountFieldQualityStatus::Unknown => "UNKNOWN",
        AccountFieldQualityStatus::Invalid => "INVALID",
        AccountFieldQualityStatus::Missing => "MISSING",
    }
}

fn position_quality_status_class(status: AccountFieldQualityStatus) -> &'static str {
    match status {
        AccountFieldQualityStatus::Actual => "ok",
        AccountFieldQualityStatus::Estimated => "warn",
        AccountFieldQualityStatus::Unknown => "unknown",
        AccountFieldQualityStatus::Invalid | AccountFieldQualityStatus::Missing => "blocked",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{AccountFieldSubject, ApiProblem};

    #[test]
    fn position_quality_rows_keep_only_position_attention() {
        let rows = vec![
            quality(
                AccountFieldSubject::position("binance", "BTCUSDT", "long"),
                "liquidationPrice",
                AccountFieldQualityStatus::Missing,
            ),
            quality(
                AccountFieldSubject::account("binance"),
                "equity",
                AccountFieldQualityStatus::Unknown,
            ),
            quality(
                AccountFieldSubject::position("okx", "ETHUSDT", "short"),
                "markPrice",
                AccountFieldQualityStatus::Actual,
            ),
        ];

        let filtered = position_field_quality_rows(rows);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].field, "liquidationPrice");
    }

    #[test]
    fn position_quality_title_keeps_problem_code() {
        let row = quality(
            AccountFieldSubject::position("binance", "BTCUSDT", "long"),
            "maintenanceMarginRatio",
            AccountFieldQualityStatus::Unknown,
        )
        .with_problem(ApiProblem::new(
            "POSITION_FIELD_UNAVAILABLE",
            "missing maintenance",
        ));

        let title = position_quality_title(&row);

        assert!(title.contains("POSITION_FIELD_UNAVAILABLE"));
        assert!(title.contains("missing maintenance"));
    }

    fn quality(
        subject: AccountFieldSubject,
        field: &str,
        status: AccountFieldQualityStatus,
    ) -> AccountFieldQuality {
        AccountFieldQuality::new(subject, field, status, "account_position_runtime", Some(1))
    }
}
