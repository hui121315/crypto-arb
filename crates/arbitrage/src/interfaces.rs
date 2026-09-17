//! 套利引擎服务层接口定义。
//!
//! 遵循面向接口编程原则：引擎本体只依赖抽象，便于测试与替换。

use shared_types::{
    FundingDiffStatsRow, FundingRateData, IndexCompositionSnapshot, MarketDataRowEvidence,
    MarketDataSnapshotStatus, SpotTick, TickerInfo,
};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct MarketDataSnapshot {
    pub funding: Arc<HashMap<String, HashMap<String, FundingRateData>>>,
    pub funding_row_evidence: Vec<MarketDataRowEvidence>,
    pub perp_tickers: Arc<Vec<TickerInfo>>,
    pub perp_ticker_row_evidence: Vec<MarketDataRowEvidence>,
    pub spot_ticks: Arc<Vec<SpotTick>>,
    pub spot_tick_row_evidence: Vec<MarketDataRowEvidence>,
    pub funding_diff_stats: Vec<FundingDiffStatsRow>,
    pub index_compositions: Arc<Vec<IndexCompositionSnapshot>>,
    pub status: Option<MarketDataSnapshotStatus>,
}

/// 机会扫描数据源。引擎只消费生命周期发布的统一市场快照。
pub trait OpportunityMarketDataSource: Send + Sync {
    fn market_snapshot(&self) -> MarketDataSnapshot;
}
