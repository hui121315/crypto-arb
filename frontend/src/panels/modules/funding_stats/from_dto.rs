//! Shared funding-window DTO projection used by production opportunity details and tests.

use shared_types::{
    ApiProblem, ArbitrageOpportunityDto, FundingDiffSampleHealth, FundingDiffWindowStats,
    FundingHistoryEvidence,
};

use super::format::duration_text;
use super::view_model::{FundingCycleStatsView, FundingCycleWindowView};
use crate::panels::modules::rate_format::signed_bps_percent;

impl FundingCycleStatsView {
    pub(crate) fn from_dto(dto: &ArbitrageOpportunityDto) -> Self {
        let windows = dto
            .funding_diff_windows
            .iter()
            .map(FundingCycleWindowView::from_window)
            .collect::<Vec<_>>();
        if let Some(window) = dto.funding_diff_window.as_ref() {
            return Self::from_stats_window(window, windows);
        }
        if let Some(window) = dto
            .funding_diff_windows
            .iter()
            .max_by_key(|window| window.cycles)
        {
            return Self::from_stats_window(window, windows);
        }
        Self::default()
    }

    fn from_stats_window(
        window: &FundingDiffWindowStats,
        windows: Vec<FundingCycleWindowView>,
    ) -> Self {
        let primary = FundingCycleWindowView::from_window(window);
        let windows = if windows.is_empty() {
            vec![primary]
        } else {
            windows
        };
        Self {
            current_percentile: window.current_percentile,
            cycles: window.cycles,
            window_hours: window.window_hours,
            sample_count: window.sample_count,
            positive_ratio_pct: window.positive_ratio * 100.0,
            reversal_count: window.reversal_count,
            stddev_diff_bps: window.stddev_diff_bps,
            mean_diff_bps: window.mean_diff_bps,
            source: window.source.clone(),
            sample_health: window.sample_health,
            problem: problem_text(window.problem_detail.as_ref(), window.problem.as_deref()),
            freshness_ms: window.freshness_ms,
            retry_after_ms: window.retry_after_ms,
            windows,
        }
    }
}

impl FundingCycleWindowView {
    fn from_window(window: &FundingDiffWindowStats) -> Self {
        Self {
            cycles: window.cycles,
            window_hours: window.window_hours,
            sample_count: window.sample_count,
            mean_diff_bps: window.mean_diff_bps,
            positive_ratio_pct: window.positive_ratio * 100.0,
            current_percentile: window.current_percentile,
            source: window.source.clone(),
            sample_health: window.sample_health,
            problem: problem_text(window.problem_detail.as_ref(), window.problem.as_deref()),
            freshness_ms: window.freshness_ms,
            retry_after_ms: window.retry_after_ms,
            distribution: distribution_text(window),
            evidence: evidence_text(&window.evidence),
        }
    }
}

fn distribution_text(window: &FundingDiffWindowStats) -> String {
    format!(
        "P50 {} · P75 {} · P90 {} · P95 {}",
        signed_bps_percent(window.p50_diff_bps),
        signed_bps_percent(window.p75_diff_bps),
        signed_bps_percent(window.p90_diff_bps),
        signed_bps_percent(window.p95_diff_bps),
    )
}

fn evidence_text(evidence: &FundingHistoryEvidence) -> String {
    let mut parts = Vec::new();
    if !evidence.source.trim().is_empty() {
        parts.push(format!("历史 {}", evidence.source.trim()));
    }
    if evidence.observed_at_ms > 0 {
        parts.push(format!("检查 {}", checked_at_text(evidence.observed_at_ms)));
    }
    if evidence.latest_at_ms > 0 {
        parts.push(format!("最新 {}", checked_at_text(evidence.latest_at_ms)));
    }
    if let Some(freshness_ms) = evidence.freshness_ms {
        parts.push(format!("新鲜度 {}", duration_text(freshness_ms)));
    }
    parts.push(format!(
        "样本 {} · {}",
        evidence.sample_count,
        sample_health_text(evidence.sample_health)
    ));
    if let Some(problem) = evidence.problem.as_ref() {
        parts.push(format!("问题 {}", problem_detail_text(problem)));
    }
    if let Some(retry_after_ms) = evidence.retry_after_ms {
        parts.push(format!("重试 {}", duration_text(retry_after_ms)));
    }
    parts.join(" · ")
}

fn sample_health_text(health: FundingDiffSampleHealth) -> &'static str {
    match health {
        FundingDiffSampleHealth::Ok => "健康",
        FundingDiffSampleHealth::Thin => "样本不足",
        FundingDiffSampleHealth::Empty => "无样本",
        FundingDiffSampleHealth::Stale => "已过期",
        FundingDiffSampleHealth::Unknown => "状态未知",
    }
}

fn problem_text(detail: Option<&ApiProblem>, fallback: Option<&str>) -> Option<String> {
    detail
        .map(problem_detail_text)
        .or_else(|| fallback.map(str::to_owned))
}

fn problem_detail_text(problem: &ApiProblem) -> String {
    match problem.source.as_deref() {
        Some(source) if !source.is_empty() => {
            format!("{} {} ({source})", problem.code, problem.message)
        }
        _ => format!("{} {}", problem.code, problem.message),
    }
}

#[cfg(target_arch = "wasm32")]
fn checked_at_text(ms: i64) -> String {
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms as f64));
    String::from(date.to_iso_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn checked_at_text(ms: i64) -> String {
    format!("{ms}ms")
}
