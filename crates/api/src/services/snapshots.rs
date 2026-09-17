use realtime::RefreshingSnapshot;
use shared_types::{FundingDiffStatsRow, SystemHealth};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const ARBITRAGE_SNAPSHOT_REFRESH_INTERVAL_MS: u64 = 5_000;

pub(crate) fn new_system_health_snapshot() -> Arc<RefreshingSnapshot<SystemHealth>> {
    Arc::new(RefreshingSnapshot::new(Duration::from_secs(5)))
}

pub(crate) fn new_funding_diff_stats_snapshot() -> Arc<RefreshingSnapshot<Vec<FundingDiffStatsRow>>>
{
    Arc::new(RefreshingSnapshot::new(Duration::from_secs(60)))
}
