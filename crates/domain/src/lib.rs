#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! 业务领域类型。
//!
//! 现阶段直接 re-export `shared-types`；后续会在此扩展仅后端使用的内部模型
//! （例如：原始套利机会 [`RawOpportunity`]、风险指标、仓位计算结果等）。

pub use shared_types::{
    ArbitrageConfig, ArbitrageOpportunityDto, ArbitrageStats, ArbitrageType, BalanceInfo,
    FundingRateData, OptionGreeks, OptionType, OrderBookInfo, OrderInfo, OrderSide, OrderStatus,
    OrderType, PositionInfo, Recommendation, RiskLevel, TickerInfo,
};
