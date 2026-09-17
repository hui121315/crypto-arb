//! 场所质量指标的采样判定、风险评分与阈值分级纯逻辑。
//! 对外文案/分级/卡片表面见父模块 `venue_quality_metrics.rs`。

use shared_types::{VenueQuality, VenueQualitySampleStatus};
use std::cmp::Ordering;

use super::{QualityCard, Tone};

pub(super) fn metric_card(
    label: &'static str,
    rows: &[VenueQuality],
    sampled: impl Fn(&VenueQuality) -> bool,
    score: impl Fn(&VenueQuality) -> f64,
    value: impl Fn(&VenueQuality) -> String,
    tone: impl Fn(f64) -> Tone,
) -> Option<QualityCard> {
    rows.iter()
        .filter(|row| sampled(row))
        .max_by(|a, b| score(a).partial_cmp(&score(b)).unwrap_or(Ordering::Equal))
        .map(|row| QualityCard {
            label,
            venue: row.venue.clone(),
            value: value(row),
            tone: tone(score(row).abs()),
        })
}

pub(super) fn risk_score(row: &VenueQuality) -> f64 {
    if !has_any_samples(row) {
        return -1.0;
    }
    let mut score = 0.0;
    if has_rest_samples(row) {
        score += row.avg_rest_latency_ms as f64 / 50.0;
    }
    if has_jitter_samples(row) {
        score += row.ws_jitter_p99_ms as f64 / 80.0;
    }
    if has_slippage_samples(row) {
        score += row.avg_slippage_bps * 4.0;
    }
    if has_fill_samples(row) {
        score += (100.0 - row.fill_rate_pct).max(0.0) * 2.0;
    }
    if has_uptime_samples(row) {
        score += (100.0 - row.uptime_window_pct).max(0.0) * 3.0;
    }
    score
}

fn has_any_samples(row: &VenueQuality) -> bool {
    row.sample_status != VenueQualitySampleStatus::NoSample && total_samples(row) > 0
}

pub(super) fn has_rest_samples(row: &VenueQuality) -> bool {
    row.rest_latency_samples > 0
}

pub(super) fn has_jitter_samples(row: &VenueQuality) -> bool {
    row.ws_jitter_samples > 0
}

pub(super) fn has_fill_samples(row: &VenueQuality) -> bool {
    row.fill_window_samples > 0
}

pub(super) fn has_slippage_samples(row: &VenueQuality) -> bool {
    row.slippage_samples > 0
}

pub(super) fn has_uptime_samples(row: &VenueQuality) -> bool {
    row.uptime_window_samples > 0
}

pub(super) fn total_samples(row: &VenueQuality) -> u32 {
    row.rest_latency_samples
        .saturating_add(row.ws_jitter_samples)
        .saturating_add(row.fill_window_samples)
        .saturating_add(row.slippage_samples)
        .saturating_add(row.uptime_window_samples)
}

pub(super) fn latency_tone_f64(value: f64) -> Tone {
    if value > 150.0 {
        Tone::Danger
    } else if value > 50.0 {
        Tone::Warn
    } else {
        Tone::Ok
    }
}

pub(super) fn jitter_tone(value: u32) -> Tone {
    if value > 250 {
        Tone::Danger
    } else if value > 100 {
        Tone::Warn
    } else {
        Tone::Ok
    }
}

pub(super) fn fill_tone(value: f64) -> Tone {
    if value < 95.0 {
        Tone::Danger
    } else if value < 98.0 {
        Tone::Warn
    } else {
        Tone::Ok
    }
}

pub(super) fn slippage_tone(value: f64) -> Tone {
    if value > 3.0 {
        Tone::Danger
    } else if value > 1.5 {
        Tone::Warn
    } else {
        Tone::Ok
    }
}

pub(super) fn uptime_tone(value: f64) -> Tone {
    if value < 99.0 {
        Tone::Danger
    } else if value < 99.8 {
        Tone::Warn
    } else {
        Tone::Ok
    }
}
