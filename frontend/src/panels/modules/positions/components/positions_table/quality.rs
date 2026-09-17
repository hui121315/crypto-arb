//! 持仓表的账户字段质量匹配与缺证据 chip 渲染：按行/字段筛选 attention、状态文案/样式。
//! 表格与单元格派生见 `derive.rs`，表格组件见父模块 `positions_table.rs`。

use leptos::prelude::*;
use shared_types::{
    AccountDataHealth, AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubjectKind,
    PositionOrigin, PositionRow, PositionSide,
};

#[cfg(test)]
#[path = "quality/account_quality.rs"]
mod account_quality;
#[path = "quality/labels.rs"]
mod labels;

#[cfg(test)]
pub(super) use account_quality::{
    bounded_overflow_quality, position_margin_quality_rows, position_overflow_quality_rows,
};
pub(super) use labels::{field_quality_label, field_quality_status_class, field_quality_title};

pub(super) fn position_quality_for_row(
    row: &PositionRow,
    field_quality: &[AccountFieldQuality],
) -> Vec<AccountFieldQuality> {
    if row.origin == PositionOrigin::ExecutionLedger {
        return Vec::new();
    }
    field_quality
        .iter()
        .filter(|quality| quality_matches_row(quality, row))
        .cloned()
        .collect()
}

fn quality_matches_row(quality: &AccountFieldQuality, row: &PositionRow) -> bool {
    if quality.subject.kind != AccountFieldSubjectKind::Position {
        return false;
    }
    let venue_match = quality
        .subject
        .venue
        .as_deref()
        .is_some_and(|venue| venue.eq_ignore_ascii_case(&row.venue));
    let symbol_match = quality
        .subject
        .symbol
        .as_deref()
        .is_some_and(|symbol| symbol.eq_ignore_ascii_case(&row.symbol));
    let side_match = quality
        .subject
        .side
        .as_deref()
        .is_none_or(|side| side.eq_ignore_ascii_case(side_key(row.side)));
    venue_match
        && symbol_match
        && side_match
        && (quality.status != AccountFieldQualityStatus::Actual
            || matches!(
                quality.field.as_str(),
                "liquidationPrice" | "liquidationDistancePct"
            ))
}

pub(super) fn position_quality_by_field(
    rows: &[AccountFieldQuality],
    field: &str,
) -> Option<AccountFieldQuality> {
    rows.iter().find(|row| row.field == field).cloned()
}

#[cfg(test)]
pub(super) fn liquidation_quality_rows(rows: &[AccountFieldQuality]) -> Vec<AccountFieldQuality> {
    rows.iter()
        .filter(|row| {
            matches!(
                row.field.as_str(),
                "liquidationPrice" | "liquidationDistancePct"
            )
        })
        .cloned()
        .collect()
}

#[cfg(test)]
pub(super) fn funding_quality_rows(rows: &[AccountFieldQuality]) -> Vec<AccountFieldQuality> {
    rows.iter()
        .filter(|row| matches!(row.field.as_str(), "fundingRate8h" | "nextFundingMs"))
        .cloned()
        .collect()
}

pub(super) fn position_row_health_for_row(
    row: &PositionRow,
    row_health: &[AccountDataHealth],
) -> Vec<AccountDataHealth> {
    if row.origin == PositionOrigin::ExecutionLedger {
        return Vec::new();
    }
    row_health
        .iter()
        .filter(|health| position_health_matches_row(health, row))
        .cloned()
        .collect()
}

fn position_health_matches_row(health: &AccountDataHealth, row: &PositionRow) -> bool {
    if health.subject.kind != AccountFieldSubjectKind::Position {
        return false;
    }
    let venue_match = health
        .subject
        .venue
        .as_deref()
        .map(shared_types::normalized_venue_name)
        == Some(shared_types::normalized_venue_name(&row.venue));
    let symbol_match = health
        .subject
        .symbol
        .as_deref()
        .is_some_and(|symbol| symbol.eq_ignore_ascii_case(&row.symbol));
    let side_match = health
        .subject
        .side
        .as_deref()
        .is_none_or(|side| side.eq_ignore_ascii_case(side_key(row.side)));
    venue_match && symbol_match && side_match
}

fn side_key(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "long",
        PositionSide::Short => "short",
    }
}

pub(super) fn position_row_evidence_toggle(
    quality: &[AccountFieldQuality],
    health: &[AccountDataHealth],
    expanded: Memo<bool>,
    controls: String,
    on_toggle: Callback<()>,
) -> AnyView {
    if quality.is_empty() && health.is_empty() {
        return ().into_any();
    }
    let has_problem = quality
        .iter()
        .any(|row| row.status != AccountFieldQualityStatus::Actual)
        || health.iter().any(|row| row.last_error.is_some());
    let count = quality.len() + health.len();
    let class = if has_problem {
        "position-evidence-summary warn"
    } else {
        "position-evidence-summary"
    };
    let label = if has_problem {
        format!("证据待核 {count}")
    } else {
        format!("证据 {count}")
    };
    view! {
        <button
            type="button"
            class=class
            aria-expanded=move || expanded.get().to_string()
            aria-controls=controls
            on:click=move |_| on_toggle.run(())
        >
            <span class="position-evidence-disclosure" aria-hidden="true">
                {move || if expanded.get() { "-" } else { "+" }}
            </span>
            {label}
        </button>
    }
    .into_any()
}

pub(super) fn position_row_evidence_panel(
    quality: &[AccountFieldQuality],
    health: &[AccountDataHealth],
) -> AnyView {
    view! {
        <div class="position-row-evidence-body" role="list" aria-label="持仓字段与账户运行证据">
            {health.iter().map(position_data_health_record).collect_view()}
            {quality.iter().map(position_field_quality_record).collect_view()}
        </div>
    }
    .into_any()
}

fn position_data_health_record(row: &AccountDataHealth) -> impl IntoView {
    let state = position_data_health_class(row);
    let detail = position_data_health_title(row);
    let status = if row.last_error.is_some() {
        "失败"
    } else if row.freshness_ms.is_some() {
        "有样本"
    } else {
        "未知"
    };
    let checked_at = evidence_time_label(Some(row.observed_at_ms));
    view! {
        <div class="position-evidence-record" data-state=state role="listitem">
            <strong>"账户快照"</strong>
            <span>{detail}</span>
            <small>{format!("{status} · {checked_at}")}</small>
        </div>
    }
}

fn position_field_quality_record(row: &AccountFieldQuality) -> impl IntoView {
    let state = field_quality_status_class(row.status);
    let detail = field_quality_title(row);
    let checked_at = evidence_time_label(row.observed_at_ms);
    view! {
        <div class="position-evidence-record" data-state=state role="listitem">
            <strong>{field_quality_label(row)}</strong>
            <span>{detail}</span>
            <small>{checked_at}</small>
        </div>
    }
}

fn evidence_time_label(observed_at_ms: Option<i64>) -> String {
    observed_at_ms
        .and_then(crate::panels::modules::timestamp::local_hms)
        .map(|time| format!("检查 {time}"))
        .unwrap_or_else(|| "检查时间未知".to_string())
}

pub(super) fn position_data_health_title(row: &AccountDataHealth) -> String {
    let mut parts = vec![position_data_health_subject(row), row.source.clone()];
    if let Some(freshness_ms) = row.freshness_ms {
        parts.push(format!("新鲜度 {}", duration_label(freshness_ms)));
    }
    if let Some(last_success_ms) = row.last_success_ms {
        parts.push(
            crate::panels::modules::timestamp::local_hms(last_success_ms)
                .map(|time| format!("最近成功 {time}"))
                .unwrap_or_else(|| "最近成功时间未知".to_string()),
        );
    }
    if let Some(retry_after_ms) = row.retry_after_ms {
        parts.push(format!("重试 {}", duration_label(retry_after_ms as i64)));
    }
    if let Some(request_id) = row.request_id.as_ref() {
        parts.push(format!("请求 {request_id}"));
    }
    if let Some(problem) = row.last_error.as_ref() {
        parts.push(problem.code.clone());
        parts.push(problem.message.clone());
    }
    parts.join(" · ")
}

fn position_data_health_subject(row: &AccountDataHealth) -> String {
    let venue = row.subject.venue.as_deref().unwrap_or("账户");
    match (row.subject.symbol.as_deref(), row.subject.side.as_deref()) {
        (Some(symbol), Some(side)) => format!("{venue} {symbol} {side}"),
        (Some(symbol), None) => format!("{venue} {symbol}"),
        _ => venue.to_owned(),
    }
}

pub(super) fn position_data_health_class(row: &AccountDataHealth) -> &'static str {
    if row.last_error.is_some() {
        "blocked"
    } else if row.freshness_ms.is_some() {
        "ok"
    } else {
        "unknown"
    }
}

fn duration_label(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let secs = ms / 1_000;
    if secs < 60 {
        return format!("{secs}s");
    }
    format!("{}m", secs / 60)
}
