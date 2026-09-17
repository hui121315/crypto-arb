use super::*;
use crate::services::market_data::{MarketDataCache, MarketSource};
use arbitrage::OpportunityMarketDataSource;
use shared_types::{FundingDiffStatsRow, FundingRateData};

#[test]
fn market_snapshot_carries_cache_status_and_history_stats() {
    let market_data = Arc::new(MarketDataCache::default());
    market_data.store_funding_rows(&[funding("BTC", "okx")], MarketSource::RestBaseline);
    let stats = Arc::new(realtime::RefreshingSnapshot::default());
    stats.set_now(vec![diff_stats_row("BTC", "okx", "binance")]);
    let source = OpportunitySnapshotSource::new(stats, market_data);

    let snapshot = source.market_snapshot();

    assert!(snapshot.status.is_some());
    assert_eq!(snapshot.funding_diff_stats.len(), 1);
    assert_eq!(
        snapshot
            .status
            .as_ref()
            .and_then(|status| status.rows.iter().find(|row| {
                row.venue == "all"
                    && row.operation == shared_types::MarketDataSnapshotOperation::FundingRates
            }))
            .map(|row| row.health.quality),
        Some(shared_types::MarketDataQuality::Fresh)
    );
}

fn funding(symbol: &str, exchange: &str) -> FundingRateData {
    FundingRateData {
        symbol: symbol.to_owned(),
        exchange: exchange.to_owned(),
        rate: 0.0001,
        rate_8h: 0.0001,
        predicted_rate: None,
        next_funding_time: 0,
        funding_interval: 8,
        volume_24h: 1_000_000.0,
        timestamp: common::time::now_ms(),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}

fn diff_stats_row(symbol: &str, long_exchange: &str, short_exchange: &str) -> FundingDiffStatsRow {
    FundingDiffStatsRow {
        symbol: symbol.to_owned(),
        long_exchange: long_exchange.to_owned(),
        short_exchange: short_exchange.to_owned(),
        computed_at_ms: 1,
        latest_at_ms: 1,
        latest_diff_bps: 1.0,
        base_interval_hours: 8,
        source: "test".to_owned(),
        freshness_ms: Some(0),
        problem: None,
        problem_detail: None,
        retry_after_ms: None,
        evidence: Default::default(),
        windows: Vec::new(),
    }
}
