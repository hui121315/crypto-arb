//! Lifecycle-owned opportunity market snapshot adapter.

use crate::services::market_data::MarketDataCache;
use arbitrage::{MarketDataSnapshot, OpportunityMarketDataSource};
use shared_types::FundingDiffStatsRow;
use std::sync::Arc;

pub(crate) struct OpportunitySnapshotSource {
    funding_diff_stats_snapshot: Arc<realtime::RefreshingSnapshot<Vec<FundingDiffStatsRow>>>,
    market_data: Arc<MarketDataCache>,
}

impl OpportunityMarketDataSource for OpportunitySnapshotSource {
    fn market_snapshot(&self) -> MarketDataSnapshot {
        let mut snapshot = self.market_data.market_snapshot_cached();
        snapshot.funding_diff_stats = self
            .funding_diff_stats_snapshot
            .value_now()
            .unwrap_or_default();
        snapshot
    }
}

impl OpportunitySnapshotSource {
    pub(crate) fn new(
        funding_diff_stats_snapshot: Arc<realtime::RefreshingSnapshot<Vec<FundingDiffStatsRow>>>,
        market_data: Arc<MarketDataCache>,
    ) -> Self {
        Self {
            funding_diff_stats_snapshot,
            market_data,
        }
    }
}

#[cfg(test)]
#[path = "data_source_tests.rs"]
mod tests;
