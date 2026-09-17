#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::too_many_lines
    )
)]
//! 套利引擎 V3。
//!
//! M4 阶段实现：交易所数据源 → 跨所配对扫描 → 风险/成本/仓位 → DTO 输出。

pub mod algorithms;
pub mod calculator;
pub mod engine_v3;
pub mod interfaces;
pub mod models;
pub mod profit_proof;
pub mod strategy_registry;

pub use algorithms::market::annualize_rate_8h_bps;
pub use calculator::OpportunityBuilder;
pub use engine_v3::ArbitrageEngineV3;
pub use interfaces::{MarketDataSnapshot, OpportunityMarketDataSource};
pub use models::{CostBreakdown, PositionSizing, RawOpportunity, RiskMetrics};
