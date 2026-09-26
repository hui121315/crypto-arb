use leptos::prelude::*;
use shared_types::{ApiProblem, VenueOperationHealth, VenueOperationStatus, VenueQuality};

use super::format::record_time;
use super::venue_quality_metrics::{
    fill_class, fill_value, jitter_class, jitter_value, latency_class, latency_value,
    operation_status_label, sample_status_class, sample_status_label, slippage_class,
    slippage_value, uptime_class, uptime_value,
};

pub(super) const QUALITY_DETAIL_ID: &str = "review-venue-quality-detail";

pub(super) fn venue_quality_detail(
    row: Memo<VenueQuality>,
    on_close: Callback<()>,
) -> impl IntoView {
    let attention = Memo::new(move |_| {
        row.with(|row| {
            row.operation_health
                .iter()
                .filter(|operation| operation.status != VenueOperationStatus::Ok)
                .cloned()
                .collect::<Vec<_>>()
        })
    });
    let healthy = Memo::new(move |_| {
        row.with(|row| {
            row.operation_health
                .iter()
                .filter(|operation| operation.status == VenueOperationStatus::Ok)
                .cloned()
                .collect::<Vec<_>>()
        })
    });

    view! {
        <section id=QUALITY_DETAIL_ID class="review-quality-detail" aria-label="当前场所运行数据依据" tabindex="-1">
            <header>
                <div><span>{move || row.get().source}</span><strong>{move || row.get().venue}</strong></div>
                <button class="review-detail-close" type="button" on:click=move |_| on_close.run(())>"关闭"</button>
            </header>
            <div class="review-quality-detail-metrics">
                {move || { let row = row.get(); view! { <>
                <QualityDetailMetric label="样本状态" value=sample_status_label(&row) class=sample_status_class(&row)/>
                <QualityDetailMetric label="REST 延迟" value=latency_value(&row) class=latency_class(&row)/>
                <QualityDetailMetric label="WS P99" value=jitter_value(&row) class=jitter_class(&row)/>
                <QualityDetailMetric label="成交率" value=fill_value(&row) class=fill_class(&row)/>
                <QualityDetailMetric label="平均滑点" value=slippage_value(&row) class=slippage_class(&row)/>
                <QualityDetailMetric label="7D 可用率" value=uptime_value(&row) class=uptime_class(&row)/>
                </> } }}
            </div>
            <section class="review-quality-operation-section" aria-label="需关注运行数据依据">
                <header><strong>"需关注运行数据依据"</strong><span>{move || format!("{}/{} · {}", attention.get().len(), row.get().operation_health.len(),
                    row.get().retry_after_ms.map(|retry| format!("重试 {retry}ms")).unwrap_or_else(|| "无重试等待".into()))}</span></header>
                {move || if attention.with(Vec::is_empty) {
                    view! { <div class="review-quality-detail-empty">"当前没有警告、阻断或待验证运行数据依据。"</div> }.into_any()
                } else {
                    operation_list(attention.get()).into_any()
                }}
            </section>
            <Show when=move || !healthy.with(Vec::is_empty)>
                <details class="review-quality-healthy-disclosure">
                    <summary><strong>"正常运行数据依据"</strong><span>{move || format!("{} 项", healthy.get().len())}</span></summary>
                    {move || operation_list(healthy.get())}
                </details>
            </Show>
            {move || row.get().last_problem.map(last_problem_detail)}
        </section>
    }
}

#[component]
fn QualityDetailMetric(
    #[prop(into)] label: String,
    value: String,
    #[prop(into)] class: String,
) -> impl IntoView {
    view! { <div><span>{label}</span><strong class=class>{value}</strong></div> }
}

fn operation_list(operations: Vec<VenueOperationHealth>) -> impl IntoView {
    view! {
        <ul class="review-quality-operation-list">
            {operations.into_iter().map(|operation| view! { <QualityOperationItem operation=operation/> }).collect_view()}
        </ul>
    }
}

#[component]
fn QualityOperationItem(operation: VenueOperationHealth) -> impl IntoView {
    let tone = operation_status_class(operation.status);
    let status = operation_status_label(operation.status);
    let message = operation
        .error
        .clone()
        .filter(|message| !message.trim().is_empty())
        .or_else(|| (!operation.message.trim().is_empty()).then(|| operation.message.clone()))
        .or_else(|| {
            operation
                .problem
                .as_ref()
                .map(|problem| problem.message.clone())
        })
        .unwrap_or_else(|| "未提供附加消息".to_owned());
    let meta = operation_meta(&operation);

    view! {
        <li class=tone>
            <header><strong>{operation.operation}</strong><span>{status}</span></header>
            <p>{message}</p>
            <small>{meta}</small>
        </li>
    }
}

fn operation_meta(operation: &VenueOperationHealth) -> String {
    let mut parts = vec![
        operation.source.clone(),
        record_time(operation.observed_at_ms),
    ];
    if let Some(latency) = operation.latency_p95_ms.or(operation.latency_ms) {
        parts.push(format!("延迟 {latency}ms"));
    }
    if let Some(freshness) = operation.freshness_ms {
        parts.push(format!("时效 {freshness}ms"));
    }
    if let Some(retry) = operation.retry_after_ms {
        parts.push(format!("重试 {retry}ms"));
    }
    if let Some(request_id) = operation
        .problem
        .as_ref()
        .and_then(|problem| problem.request_id.as_deref())
        .or_else(|| {
            operation
                .evidence
                .as_ref()
                .and_then(|evidence| evidence.request_id.as_deref())
        })
    {
        parts.push(format!("request {request_id}"));
    }
    parts.join(" · ")
}

fn operation_status_class(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "quality-ok",
        VenueOperationStatus::Blocked => "quality-danger",
        VenueOperationStatus::Warn
        | VenueOperationStatus::Unknown
        | VenueOperationStatus::Unsupported => "quality-warn",
    }
}

fn last_problem_detail(problem: ApiProblem) -> impl IntoView {
    let mut meta = Vec::new();
    if let Some(source) = problem.source {
        meta.push(source);
    }
    if let Some(status) = problem.status {
        meta.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = problem.request_id {
        meta.push(format!("request {request_id}"));
    }
    if let Some(retry) = problem.retry_after_ms {
        meta.push(format!("重试 {retry}ms"));
    }
    let meta = (!meta.is_empty()).then(|| meta.join(" · "));

    view! {
        <section class="review-quality-last-problem" aria-label="最后运行问题">
            <header><strong>{problem.code}</strong><span>"最后问题"</span></header>
            <p>{problem.message}</p>
            {meta.map(|meta| view! { <small>{meta}</small> })}
        </section>
    }
}
