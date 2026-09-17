use leptos::prelude::*;

use crate::panels::modules::futures::data::{FuturesSummary, StrategyFilter};

pub(in crate::panels::modules::futures::view) fn futures_kpis(
    summary: Memo<FuturesSummary>,
    kpi_placeholder: Memo<Option<String>>,
    strategy: Memo<StrategyFilter>,
) -> impl IntoView {
    view! {
        <div class="overview-kpis futures-kpis">
            <div class="large-kpi blue">
                <span>"候选"</span>
                <strong>{move || {
                    kpi_value_or(kpi_placeholder.get(), summary.get().candidates.to_string())
                }}</strong>
                <em>{move || {
                    let summary = summary.get();
                    let detail = if summary.candidates == 0 {
                        format!("{}当前无候选", strategy.get().label())
                    } else if summary.filtered_candidates == summary.candidates {
                        format!("当前页 {}", summary.filtered_candidates)
                    } else {
                        format!(
                            "当前页 {} / 全部 {}",
                            summary.filtered_candidates, summary.candidates
                        )
                    };
                    kpi_detail_or(kpi_placeholder.get().as_deref(), detail, "候选读取失败")
                }}</em>
            </div>
            <div class=move || {
                if summary.get().executable_candidates > 0 {
                    "large-kpi green"
                } else {
                    "large-kpi orange"
                }
            }>
                <span>"可预检"</span>
                <strong>{move || {
                    let summary = summary.get();
                    kpi_value_or(
                        kpi_placeholder.get(),
                        summary.executable_candidates.to_string(),
                    )
                }}</strong>
                <em>{move || {
                    let summary = summary.get();
                    kpi_detail_or(
                        kpi_placeholder.get().as_deref(),
                        executable_count_detail(&summary),
                        "执行判断不可用",
                    )
                }}</em>
            </div>
            <div
                class=move || {
                    let summary = summary.get();
                    match (
                        summary.best_monitor_net_bps.is_some(),
                        summary.best_monitor_preview_ready,
                    ) {
                        (true, true) => "large-kpi green",
                        (true, false) => "large-kpi blue",
                        (false, _) => "large-kpi orange",
                    }
                }
                title=move || kpi_detail_or(
                    kpi_placeholder.get().as_deref(),
                    best_profit_kpi_detail(&summary.get()),
                    "收益证据不可用",
                )
            >
                <span>"最佳监控边际"</span>
                <strong>{move || {
                    let summary = summary.get();
                    kpi_value_or(
                        kpi_placeholder.get(),
                        summary.best_monitor_net_bps
                            .map_or_else(|| "—".into(), |value| format!("{:+.3}%", value / 100.0)),
                    )
                }}</strong>
                <em>{move || {
                    let summary = summary.get();
                    kpi_detail_or(
                        kpi_placeholder.get().as_deref(),
                        best_profit_kpi_detail(&summary),
                        "收益证据不可用",
                    )
                }}</em>
            </div>
            <div class="large-kpi orange" title="构建时重新核验双腿 0.05% 可吃深度">
                <span>"深度门"</span>
                <strong>"0.05%"</strong>
                <em>"构建时核验双腿"</em>
            </div>
        </div>
    }
}

fn kpi_value_or(placeholder: Option<String>, value: String) -> String {
    placeholder.map_or(value, |_| "—".to_owned())
}

fn kpi_detail_or(placeholder: Option<&str>, value: String, error_detail: &str) -> String {
    match placeholder {
        Some("加载中") => "等待首批候选".to_owned(),
        Some("预热中") => "市场数据预热".to_owned(),
        Some("错误") => error_detail.to_owned(),
        Some(state) => state.to_owned(),
        None => value,
    }
}

pub(super) fn best_profit_kpi_detail(summary: &FuturesSummary) -> String {
    if summary.candidates == 0 {
        return "当前策略暂无候选".into();
    }
    if summary.best_monitor_net_bps.is_none() {
        return "没有通过策略与收益校验的候选".into();
    }
    let readiness = if summary.best_monitor_preview_ready {
        "可进入构建预检"
    } else {
        "仅监控，当前不可构建"
    };
    format!(
        "{} · {} · {readiness}",
        summary.best_monitor_pair, summary.best_monitor_profit_detail
    )
}

fn executable_count_detail(summary: &FuturesSummary) -> String {
    if summary.filtered_candidates == 0 {
        return "当前页没有候选".into();
    }
    if summary.executable_candidates == 0 {
        return "当前页全部仅观察".into();
    }
    format!(
        "当前页 {} / {}",
        summary.executable_candidates, summary.filtered_candidates
    )
}
