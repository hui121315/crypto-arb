use super::StrategyKind;
use crate::execution_ledger::ExecutionFillConfidence;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StrategyPerformanceSampleStatus {
    Complete,
    PartialEvidence,
    NoCompleteSample,
    #[default]
    NoTrades,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StrategyPerformance {
    pub kind: StrategyKind,
    /// Missing on legacy aggregates; never infer it from the current trading mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_environment: Option<crate::ExecutionEnvironment>,
    #[serde(default)]
    pub sample_window_days: u32,
    #[serde(default)]
    pub total_trades_30d: u32,
    pub trades_30d: u32,
    #[serde(default)]
    pub actual_trades_30d: u32,
    #[serde(default)]
    pub estimated_trades_30d: u32,
    #[serde(default)]
    pub skipped_trades_30d: u32,
    #[serde(default)]
    pub partial_evidence_trades_30d: u32,
    #[serde(default)]
    pub sample_status: StrategyPerformanceSampleStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowest_fill_confidence: Option<ExecutionFillConfidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lowest_fill_confidence_score: Option<f64>,
    #[serde(default)]
    pub profitable_trades_30d: u32,
    #[serde(default)]
    pub losing_trades_30d: u32,
    #[serde(default)]
    pub break_even_trades_30d: u32,
    #[serde(default)]
    pub independent_periods_30d: u32,
    #[serde(default)]
    pub data_missing_rate_pct: f64,
    pub hit_rate_pct: f64,
    pub avg_pnl_per_trade_usd: f64,
    pub sharpe_30d: f64,
    pub sortino_30d: f64,
    pub max_drawdown_pct: f64,
    #[serde(default)]
    pub max_drawdown_usd: f64,
    pub gross_pnl_30d_usd: f64,
    #[serde(default)]
    pub gross_profit_30d_usd: f64,
    #[serde(default)]
    pub gross_loss_30d_usd: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profit_factor: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_loss_p95_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worst_trade_pnl_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_latency_p50_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_latency_p95_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finality_latency_max_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trade_order_error_rate_pct: Option<f64>,
    #[serde(default)]
    pub actual_net_pnl_30d_usd: f64,
    #[serde(default)]
    pub estimated_net_pnl_30d_usd: f64,
    /// Realized-performance headline. This mirrors actual Net evidence only; estimated Net remains
    /// isolated in `estimated_net_pnl_30d_usd`.
    pub net_pnl_30d_usd: f64,
    pub avg_holding_hours: f64,
}
