//! 资金费周期统计的纯文案/样式助手：样本健康标签、诊断拼接、来源/新鲜度/重试格式化。
//! 视图模型见 `view_model.rs`，组件见 `component.rs`。

use shared_types::FundingDiffSampleHealth;

pub(in crate::panels::modules::funding_stats) fn health_label(
    health: FundingDiffSampleHealth,
    sample_count: usize,
) -> Option<&'static str> {
    match health {
        FundingDiffSampleHealth::Empty => Some("无周期样本"),
        FundingDiffSampleHealth::Thin => Some("样本不足"),
        FundingDiffSampleHealth::Stale => Some("历史过期"),
        FundingDiffSampleHealth::Unknown if sample_count == 0 => Some("无周期样本"),
        FundingDiffSampleHealth::Unknown => Some("历史状态未知"),
        FundingDiffSampleHealth::Ok => None,
    }
}

pub(in crate::panels::modules::funding_stats) fn bar_style(percentile: u8) -> String {
    format!("width: {}%;", percentile.clamp(4, 100))
}

pub(in crate::panels::modules::funding_stats) fn append_diagnostic(
    base: String,
    diagnostic: &str,
) -> String {
    if diagnostic.is_empty() {
        return base;
    }
    format!("{base} · {diagnostic}")
}

pub(in crate::panels::modules::funding_stats) fn diagnostic_text(
    source: &str,
    freshness_ms: Option<i64>,
    problem: Option<&str>,
    retry_after_ms: Option<i64>,
) -> String {
    let mut parts = Vec::new();
    if let Some(source) = source_text(source) {
        parts.push(source);
    }
    if let Some(freshness_ms) = freshness_ms {
        parts.push(format!("新鲜度 {}", duration_text(freshness_ms)));
    }
    if let Some(retry_after_ms) = retry_after_ms {
        parts.push(format!("重试 {}", duration_text(retry_after_ms)));
    }
    if let Some(problem) = problem.map(str::trim).filter(|problem| !problem.is_empty()) {
        parts.push(format!("问题 {problem}"));
    }
    parts.join(" · ")
}

fn source_text(source: &str) -> Option<String> {
    let source = source.trim();
    if source.is_empty() {
        return None;
    }
    Some(format!("来源 {source}"))
}

pub(in crate::panels::modules::funding_stats) fn duration_text(ms: i64) -> String {
    let ms = ms.max(0);
    if ms < 1_000 {
        return format!("{ms}ms");
    }
    let seconds = ms as f64 / 1_000.0;
    if seconds < 60.0 {
        return format!("{seconds:.1}s");
    }
    format!("{:.1}m", seconds / 60.0)
}
