//! 资金费率数据模型。

use crate::{market::FeedSnapshot, ApiProblem};
use serde::{Deserialize, Serialize};

/// 标准化的资金费率数据。
///
/// 对应 Python `core/arbitrage/interfaces.py::FundingRateData`。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingRateData {
    pub symbol: String,
    pub exchange: String,

    /// 原始费率（来自交易所，未标准化）。
    pub rate: f64,
    /// 标准化到 8h 的费率。
    pub rate_8h: f64,
    /// 预测费率。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub predicted_rate: Option<f64>,
    /// 下次结算时间戳（毫秒）。
    pub next_funding_time: i64,
    /// 结算间隔（小时）。
    pub funding_interval: u32,
    /// 24h 成交量（USD）。
    pub volume_24h: f64,
    /// 数据时间戳（毫秒）。
    pub timestamp: i64,

    /// EWMA 平滑后费率。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smoothed_rate: Option<f64>,
    /// 费率标准差。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_std: Option<f64>,
    /// 是否被识别为异常值。
    #[serde(default)]
    pub is_outlier: bool,
}

pub type FundingRatesEnvelope = FeedSnapshot<Vec<FundingRateData>>;

/// Settled private funding payment from an authenticated account ledger.
///
/// This is distinct from [`FundingRateData`]: rates describe the market, while
/// payments describe money that has actually moved in a venue account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingPaymentData {
    pub venue: String,
    pub symbol: String,
    pub amount: f64,
    pub currency: String,
    pub funding_time_ms: i64,
    pub venue_event_id: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingPaymentIngestSkipReason {
    #[default]
    InvalidRow,
    DuplicateOrAlreadyRecorded,
    InvalidMatchKey,
    NoMatchingOrder,
    NoFilledAnchor,
    AmbiguousOrderGroup,
    UnmatchedOrAmbiguousOrder,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingPaymentIngestSkipReasonCount {
    pub reason: FundingPaymentIngestSkipReason,
    pub count: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingPaymentIngestRouteFailure {
    pub venue: String,
    pub operation: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingPaymentIngestReport {
    pub observed_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_start_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_end_ms: Option<i64>,
    pub fetched: usize,
    pub mapped: usize,
    pub ledger_events: usize,
    pub skipped: usize,
    pub invalid: usize,
    pub duplicate_or_already_recorded: usize,
    pub invalid_match_key: usize,
    pub no_matching_order: usize,
    pub no_filled_anchor: usize,
    pub ambiguous_order_group: usize,
    pub unmatched_or_ambiguous_order: usize,
    pub route_failures: usize,
    pub unsupported: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skip_reasons: Vec<FundingPaymentIngestSkipReasonCount>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route_failure_details: Vec<FundingPaymentIngestRouteFailure>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fetch_error: Option<String>,
}

impl FundingPaymentIngestReport {
    pub fn is_success(&self) -> bool {
        self.fetch_error.is_none()
    }

    pub fn refresh_skip_reasons(&mut self) {
        let mut reasons = Vec::new();
        push_skip_reason(
            &mut reasons,
            FundingPaymentIngestSkipReason::InvalidRow,
            self.invalid,
        );
        push_skip_reason(
            &mut reasons,
            FundingPaymentIngestSkipReason::DuplicateOrAlreadyRecorded,
            self.duplicate_or_already_recorded,
        );
        push_skip_reason(
            &mut reasons,
            FundingPaymentIngestSkipReason::InvalidMatchKey,
            self.invalid_match_key,
        );
        push_skip_reason(
            &mut reasons,
            FundingPaymentIngestSkipReason::NoMatchingOrder,
            self.no_matching_order,
        );
        push_skip_reason(
            &mut reasons,
            FundingPaymentIngestSkipReason::NoFilledAnchor,
            self.no_filled_anchor,
        );
        push_skip_reason(
            &mut reasons,
            FundingPaymentIngestSkipReason::AmbiguousOrderGroup,
            self.ambiguous_order_group,
        );
        push_skip_reason(
            &mut reasons,
            FundingPaymentIngestSkipReason::UnmatchedOrAmbiguousOrder,
            self.unmatched_or_ambiguous_order,
        );
        self.skip_reasons = reasons;
    }
}

fn push_skip_reason(
    reasons: &mut Vec<FundingPaymentIngestSkipReasonCount>,
    reason: FundingPaymentIngestSkipReason,
    count: usize,
) {
    if count > 0 {
        reasons.push(FundingPaymentIngestSkipReasonCount { reason, count });
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingDiffStatsRow {
    pub symbol: String,
    pub long_exchange: String,
    pub short_exchange: String,
    pub computed_at_ms: i64,
    pub latest_at_ms: i64,
    pub latest_diff_bps: f64,
    pub base_interval_hours: u32,
    #[serde(default)]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem_detail: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
    #[serde(default)]
    pub evidence: FundingHistoryEvidence,
    pub windows: Vec<FundingDiffWindowStats>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FundingDiffSampleHealth {
    #[default]
    Unknown,
    Ok,
    Thin,
    Empty,
    Stale,
}

impl FundingDiffSampleHealth {
    pub fn score_weight(self) -> f64 {
        match self {
            Self::Ok => 1.0,
            Self::Thin => 0.5,
            Self::Unknown | Self::Empty | Self::Stale => 0.0,
        }
    }

    pub fn is_scoring_usable(self) -> bool {
        self.score_weight() > 0.0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingHistoryEvidence {
    pub source: String,
    pub observed_at_ms: i64,
    pub latest_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    pub sample_count: usize,
    pub sample_health: FundingDiffSampleHealth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
}

impl FundingHistoryEvidence {
    pub fn is_usable(&self) -> bool {
        !self.source.trim().is_empty()
            && self.observed_at_ms > 0
            && self.latest_at_ms > 0
            && self.sample_count > 0
            && self.sample_health.is_scoring_usable()
            && self.problem.is_none()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FundingDiffWindowStats {
    pub cycles: u32,
    pub window_hours: u32,
    pub sample_count: usize,
    pub mean_diff_bps: f64,
    pub p50_diff_bps: f64,
    pub p75_diff_bps: f64,
    pub p90_diff_bps: f64,
    pub p95_diff_bps: f64,
    pub stddev_diff_bps: f64,
    pub positive_ratio: f64,
    pub reversal_count: usize,
    pub current_percentile: u8,
    #[serde(default)]
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness_ms: Option<i64>,
    #[serde(default)]
    pub sample_health: FundingDiffSampleHealth,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem_detail: Option<ApiProblem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_after_ms: Option<i64>,
    #[serde(default)]
    pub evidence: FundingHistoryEvidence,
}

#[cfg(test)]
mod tests;
