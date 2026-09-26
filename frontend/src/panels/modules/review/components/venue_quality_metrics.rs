use shared_types::{VenueOperationStatus, VenueQuality, VenueQualitySampleStatus};
use std::cmp::Ordering;

use crate::panels::modules::rate_format::unsigned_bps_percent;

#[path = "venue_quality_metrics/scoring.rs"]
mod scoring;
use scoring::{
    fill_tone, has_fill_samples, has_jitter_samples, has_rest_samples, has_slippage_samples,
    has_uptime_samples, jitter_tone, latency_tone_f64, metric_card, risk_score, slippage_tone,
    total_samples, uptime_tone,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Tone {
    Ok,
    Warn,
    Danger,
}

impl Tone {
    pub(super) const fn class(self) -> &'static str {
        match self {
            Self::Ok => "quality-ok",
            Self::Warn => "quality-warn",
            Self::Danger => "quality-danger",
        }
    }
}

#[derive(Clone, PartialEq)]
pub(super) struct QualityCard {
    pub(super) label: &'static str,
    pub(super) venue: String,
    pub(super) value: String,
    pub(super) tone: Tone,
}

pub(super) fn sorted_by_risk(mut rows: Vec<VenueQuality>) -> Vec<VenueQuality> {
    rows.sort_by(|a, b| {
        risk_score(b)
            .partial_cmp(&risk_score(a))
            .unwrap_or(Ordering::Equal)
    });
    rows
}

pub(super) fn summary_cards(rows: &[VenueQuality]) -> Vec<QualityCard> {
    [
        metric_card(
            "已采样最慢 REST",
            rows,
            has_rest_samples,
            |r| r.avg_rest_latency_ms as f64,
            latency_value,
            latency_tone_f64,
        ),
        metric_card(
            "已采样最高滑点",
            rows,
            has_slippage_samples,
            |r| r.avg_slippage_bps,
            slippage_value,
            slippage_tone,
        ),
        metric_card(
            "已采样最低成交率",
            rows,
            has_fill_samples,
            |r| -r.fill_rate_pct,
            fill_value,
            fill_tone,
        ),
        metric_card(
            "已采样最低可用率",
            rows,
            has_uptime_samples,
            |r| -r.uptime_window_pct,
            uptime_value,
            uptime_tone,
        ),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub(super) fn sample_status_label(row: &VenueQuality) -> String {
    match row.sample_status {
        VenueQualitySampleStatus::NoSample => "未采样".to_owned(),
        VenueQualitySampleStatus::WarmingUp => {
            format!("采样中 · {}", total_samples(row))
        }
        VenueQualitySampleStatus::Ready => format!("样本 {}", total_samples(row)),
    }
}

pub(super) fn sample_status_class(row: &VenueQuality) -> &'static str {
    match row.sample_status {
        VenueQualitySampleStatus::Ready => Tone::Ok.class(),
        VenueQualitySampleStatus::NoSample | VenueQualitySampleStatus::WarmingUp => {
            Tone::Warn.class()
        }
    }
}

pub(super) fn latency_class(row: &VenueQuality) -> &'static str {
    if has_rest_samples(row) {
        latency_tone_f64(row.avg_rest_latency_ms as f64).class()
    } else {
        Tone::Warn.class()
    }
}

pub(super) fn jitter_class(row: &VenueQuality) -> &'static str {
    if has_jitter_samples(row) {
        jitter_tone(row.ws_jitter_p99_ms).class()
    } else {
        Tone::Warn.class()
    }
}

pub(super) fn fill_class(row: &VenueQuality) -> &'static str {
    if has_fill_samples(row) {
        fill_tone(row.fill_rate_pct).class()
    } else {
        Tone::Warn.class()
    }
}

pub(super) fn slippage_class(row: &VenueQuality) -> &'static str {
    if has_slippage_samples(row) {
        slippage_tone(row.avg_slippage_bps).class()
    } else {
        Tone::Warn.class()
    }
}

pub(super) fn uptime_class(row: &VenueQuality) -> &'static str {
    if has_uptime_samples(row) {
        uptime_tone(row.uptime_window_pct).class()
    } else {
        Tone::Warn.class()
    }
}

pub(super) fn latency_value(row: &VenueQuality) -> String {
    if has_rest_samples(row) {
        format!("{}ms", row.avg_rest_latency_ms)
    } else {
        "未采样".to_owned()
    }
}

pub(super) fn jitter_value(row: &VenueQuality) -> String {
    if !has_jitter_samples(row) {
        "未采样".to_owned()
    } else if row.ws_jitter_samples < 100 {
        "采样中".to_owned()
    } else {
        format!("{}ms", row.ws_jitter_p99_ms)
    }
}

pub(super) fn fill_value(row: &VenueQuality) -> String {
    if has_fill_samples(row) {
        pct(row.fill_rate_pct)
    } else {
        "未采样".to_owned()
    }
}

pub(super) fn slippage_value(row: &VenueQuality) -> String {
    if has_slippage_samples(row) {
        unsigned_bps_percent(row.avg_slippage_bps)
    } else {
        "未采样".to_owned()
    }
}

pub(super) fn uptime_value(row: &VenueQuality) -> String {
    if has_uptime_samples(row) {
        pct(row.uptime_window_pct)
    } else {
        "未采样".to_owned()
    }
}

pub(super) fn operation_class(row: &VenueQuality) -> &'static str {
    if row
        .operation_health
        .iter()
        .any(|operation| operation.status == VenueOperationStatus::Blocked)
    {
        Tone::Danger.class()
    } else if row
        .operation_health
        .iter()
        .any(|operation| operation.status != VenueOperationStatus::Ok)
        || row.operation_health.is_empty()
    {
        Tone::Warn.class()
    } else {
        Tone::Ok.class()
    }
}

pub(super) fn operation_value(row: &VenueQuality) -> String {
    if row.operation_health.is_empty() {
        return "无运行数据依据".to_owned();
    }
    let attention = row
        .operation_health
        .iter()
        .filter(|operation| operation.status != VenueOperationStatus::Ok)
        .count();
    let mut value = format!("{} 项", row.operation_health.len());
    if attention > 0 {
        value.push_str(&format!(" · {attention} 需关注"));
    }
    if let Some(retry_after_ms) = row.retry_after_ms {
        value.push_str(&format!(" · retry {retry_after_ms}ms"));
    }
    value
}

pub(super) fn operation_status_label(status: VenueOperationStatus) -> &'static str {
    match status {
        VenueOperationStatus::Ok => "正常",
        VenueOperationStatus::Warn => "警告",
        VenueOperationStatus::Blocked => "阻断",
        VenueOperationStatus::Unknown => "待验证",
        VenueOperationStatus::Unsupported => "不支持",
    }
}

pub(super) fn pct(value: f64) -> String {
    format!("{value:.1}%")
}

#[cfg(test)]
#[path = "venue_quality_metrics/tests.rs"]
mod tests;
