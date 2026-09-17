//! 资金费周期趋势组件：把视图模型窗口渲染成分位条 + 诊断标题。
//! 视图模型见 `view_model.rs`，纯文案/样式助手见 `format.rs`。

use leptos::prelude::*;

use super::format::bar_style;
use super::view_model::{FundingCycleStatsView, FundingCycleWindowView};
use crate::panels::modules::rate_format::signed_bps_percent;

pub(in crate::panels::modules) fn funding_cycle_trend(
    stats: FundingCycleStatsView,
) -> impl IntoView {
    let title = stats.trend_text();
    let windows = stats.into_windows_for_display();
    if windows.is_empty() {
        return view! { <div class="funding-cycle-empty">"无周期样本"</div> }.into_any();
    }
    view! {
        <div class="funding-cycle-trend" title=title>
            {windows.into_iter().map(|window| view! {
                <FundingCycleBar window=window/>
            }).collect_view()}
        </div>
    }
    .into_any()
}

#[component]
fn FundingCycleBar(window: FundingCycleWindowView) -> impl IntoView {
    let diagnostic = window.diagnostic_text();
    let detail_evidence = window.detail_evidence_text();
    let title = [diagnostic.as_str(), detail_evidence.as_str()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    let status = window.status_text();
    let FundingCycleWindowView {
        cycles,
        mean_diff_bps,
        current_percentile,
        distribution,
        evidence,
        ..
    } = window;
    let bar_class = if mean_diff_bps >= 0.0 {
        "funding-cycle-fill positive-fill"
    } else {
        "funding-cycle-fill negative-fill"
    };
    view! {
        <div class="funding-cycle-window" title=title>
            <div class="funding-cycle-row">
                <span>{format!("{cycles}周期")}</span>
                <div class="funding-cycle-track">
                    <i class=bar_class style=bar_style(current_percentile)></i>
                </div>
                <strong>{signed_bps_percent(mean_diff_bps)}</strong>
                <em>{status}</em>
            </div>
            <small class="funding-cycle-distribution">{distribution}</small>
            {(!evidence.is_empty()).then(|| view! {
                <small class="funding-cycle-evidence">{evidence}</small>
            })}
        </div>
    }
}
