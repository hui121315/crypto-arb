//! 余额证据 chip、折叠区与字段健康明细渲染。

use leptos::prelude::*;
use shared_types::{AccountDataHealth, AccountFieldQuality, VenueOperationHealth};

use super::derive::{
    account_data_health_subject, account_field_label, account_quality_status_class,
    account_quality_status_label, account_quality_subject, account_quality_title,
    balance_data_health_class, balance_data_health_title, balance_evidence_title,
    balance_operation_label, balance_status_class, balance_status_label, duration_label,
};

const INLINE_EVIDENCE_LIMIT: usize = 4;

pub(super) fn render_balance_row_diagnostics(
    field_quality: &[AccountFieldQuality],
    row_health: &[AccountDataHealth],
) -> AnyView {
    let count = field_quality.len().saturating_add(row_health.len());
    if count == 0 {
        return ().into_any();
    }
    view! {
        <details class="balance-row-diagnostics">
            <summary>{format!("数据数据依据 {count}")}</summary>
            {render_balance_data_health(row_health)}
            {render_balance_row_quality(field_quality)}
        </details>
    }
    .into_any()
}

pub(super) fn render_balance_diagnostics(
    health: Vec<VenueOperationHealth>,
    quality: &[AccountFieldQuality],
) -> AnyView {
    let count = health.len().saturating_add(quality.len());
    if count == 0 {
        return ().into_any();
    }
    view! {
        <details class="balance-diagnostics">
            <summary>
                <span>"账户数据数据依据"</span>
                <em>{count}</em>
            </summary>
            <div class="balance-diagnostics-body">
                {render_balance_evidence(health)}
                {render_account_field_quality(quality)}
            </div>
        </details>
    }
    .into_any()
}

fn render_balance_row_quality(rows: &[AccountFieldQuality]) -> AnyView {
    if rows.is_empty() {
        return ().into_any();
    }
    view! {
        <div class="balance-row-quality">
            {rows.iter().map(account_quality_chip).collect_view()}
        </div>
    }
    .into_any()
}

pub(super) fn render_balance_evidence(rows: Vec<VenueOperationHealth>) -> AnyView {
    if rows.is_empty() {
        return ().into_any();
    }
    let (inline, overflow) = bounded_evidence_rows(rows);
    let overflow_count = overflow.len();
    view! {
        <div class="balance-evidence">
            {inline.into_iter().map(balance_evidence_chip).collect_view()}
            {evidence_disclosure(
                overflow_count,
                overflow.into_iter().map(balance_evidence_chip).collect_view(),
            )}
        </div>
    }
    .into_any()
}

pub(super) fn render_account_field_quality(rows: &[AccountFieldQuality]) -> AnyView {
    if rows.is_empty() {
        return ().into_any();
    }
    let (inline, overflow) = bounded_evidence_rows(rows.to_vec());
    let overflow_count = overflow.len();
    view! {
        <div class="balance-evidence">
            {inline.iter().map(account_quality_chip).collect_view()}
            {evidence_disclosure(
                overflow_count,
                overflow.iter().map(account_quality_chip).collect_view(),
            )}
        </div>
    }
    .into_any()
}

pub(super) fn render_balance_data_health(rows: &[AccountDataHealth]) -> AnyView {
    if rows.is_empty() {
        return ().into_any();
    }
    let (inline, overflow) = bounded_evidence_rows(rows.to_vec());
    let overflow_count = overflow.len();
    view! {
        <div class="balance-evidence">
            {inline.iter().map(balance_data_health_chip).collect_view()}
            {evidence_disclosure(
                overflow_count,
                overflow.iter().map(balance_data_health_chip).collect_view(),
            )}
        </div>
    }
    .into_any()
}

pub(super) fn bounded_evidence_rows<T>(mut rows: Vec<T>) -> (Vec<T>, Vec<T>) {
    let overflow = if rows.len() > INLINE_EVIDENCE_LIMIT {
        rows.split_off(INLINE_EVIDENCE_LIMIT)
    } else {
        Vec::new()
    };
    (rows, overflow)
}

fn evidence_disclosure(count: usize, content: impl IntoView + 'static) -> AnyView {
    if count == 0 {
        return ().into_any();
    }
    let title = format!("展开其余 {count} 条数据依据");
    view! {
        <details class="balance-evidence-more">
            <summary title=title>{format!("+{count}")}</summary>
            <div class="balance-evidence">{content}</div>
        </details>
    }
    .into_any()
}

fn balance_evidence_chip(row: VenueOperationHealth) -> impl IntoView {
    let class = format!("balance-evidence-chip {}", balance_status_class(row.status));
    let title = balance_evidence_title(&row);
    view! {
        <span class=class title=title>
            {row.venue} " · " {balance_operation_label(&row.operation)}
            " · " {balance_status_label(row.status)}
            {row.freshness_ms.map(|freshness| view! {
                <em>{duration_label(freshness)}</em>
            })}
        </span>
    }
}

fn account_quality_chip(row: &AccountFieldQuality) -> impl IntoView {
    let class = format!(
        "balance-evidence-chip {}",
        account_quality_status_class(row.status)
    );
    let title = account_quality_title(row);
    let subject = account_quality_subject(row);
    let field = account_field_label(&row.field);
    let status = account_quality_status_label(row.status);
    view! {
        <span class=class title=title>
            {subject} " · " {field} " · " {status}
        </span>
    }
}

fn balance_data_health_chip(row: &AccountDataHealth) -> impl IntoView {
    let class = format!("balance-evidence-chip {}", balance_data_health_class(row));
    let title = balance_data_health_title(row);
    let subject = account_data_health_subject(row);
    view! {
        <span class=class title=title>
            {subject} " · " {row.source.clone()}
            {row.freshness_ms.map(|freshness| view! {
                <em>{duration_label(freshness)}</em>
            })}
        </span>
    }
}
