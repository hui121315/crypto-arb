//! 资金费周期统计视图模型：把 DTO 窗口收敛成分位/趋势文案，诊断异常不伪装健康。
//! 生产 DTO 投影见 `from_dto.rs`，纯文案助手见 `format.rs`，组件见 `component.rs`。

use shared_types::FundingDiffSampleHealth;

use super::format::{append_diagnostic, diagnostic_text, health_label};
use crate::panels::modules::rate_format::{signed_bps_percent, unsigned_bps_percent};

#[derive(Clone, Default, PartialEq)]
pub(crate) struct FundingCycleStatsView {
    pub current_percentile: u8,
    pub cycles: u32,
    pub window_hours: u32,
    pub sample_count: usize,
    pub positive_ratio_pct: f64,
    pub reversal_count: usize,
    pub stddev_diff_bps: f64,
    pub mean_diff_bps: f64,
    pub source: String,
    pub sample_health: FundingDiffSampleHealth,
    pub problem: Option<String>,
    pub freshness_ms: Option<i64>,
    pub retry_after_ms: Option<i64>,
    pub windows: Vec<FundingCycleWindowView>,
}

#[derive(Clone, Default, PartialEq)]
pub(crate) struct FundingCycleWindowView {
    pub cycles: u32,
    pub window_hours: u32,
    pub sample_count: usize,
    pub mean_diff_bps: f64,
    pub positive_ratio_pct: f64,
    pub current_percentile: u8,
    pub source: String,
    pub sample_health: FundingDiffSampleHealth,
    pub problem: Option<String>,
    pub freshness_ms: Option<i64>,
    pub retry_after_ms: Option<i64>,
    pub distribution: String,
    pub evidence: String,
}

impl FundingCycleStatsView {
    pub(crate) fn percentile_text(&self) -> String {
        if let Some(label) = health_label(self.sample_health, self.sample_count) {
            if self.sample_count == 0 {
                return label.into();
            }
            return format!("{label} · {}周期", self.cycles);
        }
        if self.has_problem() {
            return format!("诊断异常 · {}周期", self.cycles);
        }
        format!("P{} · {}周期", self.current_percentile, self.cycles)
    }

    pub(crate) fn detail_text(&self) -> String {
        let diagnostic = self.diagnostic_text();
        if self.sample_count == 0 {
            let status = health_label(self.sample_health, self.sample_count)
                .unwrap_or("无周期样本")
                .to_owned();
            return append_diagnostic(status, &diagnostic);
        }
        let status = self.health_status_prefix();
        let base = format!(
            "{}P{} · {}周期/{}h · 样本 {} · 正向 {:.0}% · 反转 {} · 波动 {}",
            status,
            self.current_percentile,
            self.cycles,
            self.window_hours,
            self.sample_count,
            self.positive_ratio_pct,
            self.reversal_count,
            unsigned_bps_percent(self.stddev_diff_bps)
        );
        append_diagnostic(base, &diagnostic)
    }

    pub(crate) fn trend_text(&self) -> String {
        let windows = self.windows_for_display();
        if windows.is_empty() {
            return "无周期样本".into();
        }
        windows
            .into_iter()
            .map(|window| {
                if let Some(status) = window.diagnostic_status() {
                    let base = format!(
                        "{}周期 {} · {} · 样本 {}",
                        window.cycles,
                        signed_bps_percent(window.mean_diff_bps),
                        status,
                        window.sample_count
                    );
                    let diagnostic = window.diagnostic_text();
                    return append_diagnostic(base, &diagnostic);
                }
                let base = format!(
                    "{}周期 {} · P{} · 正向 {:.0}%",
                    window.cycles,
                    signed_bps_percent(window.mean_diff_bps),
                    window.current_percentile,
                    window.positive_ratio_pct
                );
                let diagnostic = window.diagnostic_text();
                append_diagnostic(base, &diagnostic)
            })
            .collect::<Vec<_>>()
            .join(" / ")
    }

    fn windows_for_display(&self) -> Vec<FundingCycleWindowView> {
        if !self.windows.is_empty() {
            return self.windows.clone();
        }
        if self.sample_count == 0 {
            return Vec::new();
        }
        vec![FundingCycleWindowView {
            cycles: self.cycles,
            window_hours: self.window_hours,
            sample_count: self.sample_count,
            mean_diff_bps: self.mean_diff_bps,
            positive_ratio_pct: self.positive_ratio_pct,
            current_percentile: self.current_percentile,
            source: self.source.clone(),
            sample_health: self.sample_health,
            problem: self.problem.clone(),
            freshness_ms: self.freshness_ms,
            retry_after_ms: self.retry_after_ms,
            distribution: "分布未提供".into(),
            evidence: String::new(),
        }]
    }

    pub(in crate::panels::modules::funding_stats) fn into_windows_for_display(
        self,
    ) -> Vec<FundingCycleWindowView> {
        let Self {
            current_percentile,
            cycles,
            window_hours,
            sample_count,
            positive_ratio_pct,
            mean_diff_bps,
            source,
            sample_health,
            problem,
            freshness_ms,
            retry_after_ms,
            windows,
            ..
        } = self;
        if !windows.is_empty() {
            return windows;
        }
        if sample_count == 0 {
            return Vec::new();
        }
        vec![FundingCycleWindowView {
            cycles,
            window_hours,
            sample_count,
            mean_diff_bps,
            positive_ratio_pct,
            current_percentile,
            source,
            sample_health,
            problem,
            freshness_ms,
            retry_after_ms,
            distribution: "分布未提供".into(),
            evidence: String::new(),
        }]
    }

    fn health_status_prefix(&self) -> String {
        if let Some(label) = health_label(self.sample_health, self.sample_count) {
            return format!("{label} · ");
        }
        if self.has_problem() {
            return "诊断异常 · ".into();
        }
        String::new()
    }

    fn diagnostic_text(&self) -> String {
        diagnostic_text(
            &self.source,
            self.freshness_ms,
            self.problem.as_deref(),
            self.retry_after_ms,
        )
    }

    fn has_problem(&self) -> bool {
        self.problem
            .as_deref()
            .map(str::trim)
            .is_some_and(|problem| !problem.is_empty())
    }
}

impl FundingCycleWindowView {
    fn health_label(&self) -> Option<&'static str> {
        health_label(self.sample_health, self.sample_count)
    }

    pub(in crate::panels::modules::funding_stats) fn status_text(&self) -> String {
        let status = self.diagnostic_status().unwrap_or_else(|| {
            format!(
                "P{} · {:.0}%",
                self.current_percentile, self.positive_ratio_pct
            )
        });
        let diagnostic = self.diagnostic_text();
        append_diagnostic(status, &diagnostic)
    }

    fn diagnostic_status(&self) -> Option<String> {
        self.health_label().map(str::to_owned).or_else(|| {
            if self.has_problem() {
                Some("诊断异常".into())
            } else {
                None
            }
        })
    }

    pub(in crate::panels::modules::funding_stats) fn diagnostic_text(&self) -> String {
        diagnostic_text(
            &self.source,
            self.freshness_ms,
            self.problem.as_deref(),
            self.retry_after_ms,
        )
    }

    pub(in crate::panels::modules::funding_stats) fn detail_evidence_text(&self) -> String {
        [self.distribution.as_str(), self.evidence.as_str()]
            .into_iter()
            .filter(|value| !value.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" · ")
    }

    fn has_problem(&self) -> bool {
        self.problem
            .as_deref()
            .map(str::trim)
            .is_some_and(|problem| !problem.is_empty())
    }
}
