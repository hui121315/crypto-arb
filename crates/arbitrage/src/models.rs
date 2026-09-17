//! 套利引擎内部数据模型。
//!
//! 公共 DTO（[`ArbitrageOpportunityDto`] 等）来自 [`shared_types`]；本模块定义引擎内部
//! 流转的中间结构（原始机会、风险指标、仓位计算结果）。

use shared_types::{
    ArbitrageType, FundingDiffWindowStats, FundingRateData, IndexCompositionEvidence,
    IndexCompositionQuality, MarketDataQuality, MarketDataSourceKind, OnchainMetadata,
    OpportunityLegMarketEvidence, OpportunityQuoteConversion, RoundTripCostBreakdown, SpotLegMode,
    StrategyKind,
};

#[derive(Debug, Clone, Copy)]
pub struct FundingMarketEvidence {
    pub quality: MarketDataQuality,
    pub source: MarketDataSourceKind,
    pub observed_at_ms: i64,
}

/// 策略特定扩展字段。默认空值保证老的 funding-only 策略保持零额外开销。
#[derive(Debug, Clone, Default)]
pub struct RawOpportunityExtra {
    pub strategy_kind: Option<StrategyKind>,
    pub type_label: Option<String>,
    pub description: Option<String>,
    pub long_action: Option<String>,
    pub short_action: Option<String>,
    pub spot_leg_mode: Option<SpotLegMode>,
    pub long_price: Option<f64>,
    pub short_price: Option<f64>,
    pub long_market_symbol: Option<String>,
    pub short_market_symbol: Option<String>,
    pub long_leg_market_evidence: Option<OpportunityLegMarketEvidence>,
    pub short_leg_market_evidence: Option<OpportunityLegMarketEvidence>,
    pub long_funding_evidence: Option<FundingMarketEvidence>,
    pub short_funding_evidence: Option<FundingMarketEvidence>,
    pub quote_conversions: Vec<OpportunityQuoteConversion>,
    pub price_deviation: Option<f64>,
    pub basis_spread: Option<f64>,
    pub basis_annual_cost: Option<f64>,
    pub basis_bps: Option<f64>,
    pub annualized_funding_bps: Option<f64>,
    pub triangular_path: Option<Vec<String>>,
    pub onchain_metadata: Option<OnchainMetadata>,
    pub long_depth_symbol: Option<String>,
    pub short_depth_symbol: Option<String>,
    pub history_stats: Option<OpportunityHistoryStats>,
    pub price_spread_convergence: Option<PriceSpreadConvergence>,
    pub index_composition: Option<IndexCompositionRisk>,
    pub cost_round_trip: Option<RoundTripCostBreakdown>,
    pub execution_blockers: Vec<String>,
}

/// Empirical close-side convergence evidence for a cross-venue perpetual spread.
/// The values are derived from executable bid/ask observations, not funding history.
#[derive(Debug, Clone, Copy, Default)]
pub struct PriceSpreadConvergence {
    pub sample_count: usize,
    pub span_ms: i64,
    pub completed_episodes: usize,
    pub profitable_episodes: usize,
    pub success_ratio: f64,
    pub expected_gross_bps: f64,
    pub projected_net_bps: f64,
    pub lower_quartile_net_bps: Option<f64>,
    pub median_net_bps: Option<f64>,
    pub recommended_hold_ms: i64,
    pub convergence_ready: bool,
    pub funding_runway_ready: bool,
    pub funding_runway_ms: Option<i64>,
    pub required_runway_ms: i64,
    pub evidence_ready: bool,
}

#[derive(Debug, Clone)]
pub struct OpportunityHistoryStats {
    pub window: FundingDiffWindowStats,
    pub windows: Vec<FundingDiffWindowStats>,
}

#[derive(Debug, Clone)]
pub struct IndexCompositionRisk {
    pub overlap_score: f64,
    pub long_quality: IndexCompositionQuality,
    pub short_quality: IndexCompositionQuality,
    pub hidden_price: bool,
    pub blocker: Option<String>,
    pub long_evidence: Option<IndexCompositionEvidence>,
    pub short_evidence: Option<IndexCompositionEvidence>,
}

/// 原始套利机会：扫描器枚举出的、尚未经成本/风险/仓位计算的中间结构。
#[derive(Debug, Clone)]
pub struct RawOpportunity {
    pub symbol: String,
    pub arb_type: ArbitrageType,
    pub long_exchange: String,
    pub short_exchange: String,
    /// 做多侧资金费率快照
    pub long_rate: FundingRateData,
    /// 做空侧资金费率快照
    pub short_rate: FundingRateData,
    /// Legacy comparison slot. Native-event strategies store their next-event projection here.
    pub spread_8h: f64,
    /// 当前快照可证明的单次投影收益；一次性基差不得按 Funding 周期重复计算。
    pub single_yield: f64,
    pub extra: RawOpportunityExtra,
}

/// 风险指标。
#[derive(Debug, Clone, Copy)]
pub struct RiskMetrics {
    pub volatility: f64,
    pub sharpe_ratio: f64,
    pub calmar_ratio: f64,
    pub var_95: f64,
    pub max_drawdown: f64,
    pub win_rate: f64,
    pub rate_mean: f64,
    pub rate_std: f64,
    pub sample_size: usize,
}

impl Default for RiskMetrics {
    fn default() -> Self {
        Self {
            volatility: 0.0,
            sharpe_ratio: 0.0,
            calmar_ratio: 0.0,
            var_95: 0.0,
            max_drawdown: 0.0,
            win_rate: 0.5,
            rate_mean: 0.0,
            rate_std: 0.0,
            sample_size: 0,
        }
    }
}

/// 仓位计算结果。
#[derive(Debug, Clone, Copy)]
pub struct PositionSizing {
    pub kelly_fraction: f64,
    pub optimal_position: f64,
    pub max_position: f64,
    pub risk_adjusted_position: f64,
}

impl Default for PositionSizing {
    fn default() -> Self {
        Self {
            kelly_fraction: 0.0,
            optimal_position: 0.0,
            max_position: 0.0,
            risk_adjusted_position: 0.0,
        }
    }
}

/// 成本计算结果。
#[derive(Debug, Clone, Copy, Default)]
pub struct CostBreakdown {
    /// 单边手续费率（taker / maker 平均）
    pub fee_rate: f64,
    /// 滑点率
    pub slippage: f64,
    /// 策略执行周期成本率；现货跨所为首轮双边成交成本，其余策略通常为双边开平仓成本
    pub round_trip_cost: f64,
}
