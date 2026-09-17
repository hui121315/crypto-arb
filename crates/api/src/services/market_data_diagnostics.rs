use crate::services::market_data::{MarketQuality, MarketSource};
use crate::state::AppState;
use shared_types::{
    MarketCacheAccessRow, MarketDataCacheCounters, MarketDataDiagnosticsSnapshot,
    MarketDataQuality, MarketDataSourceKind, RestBaselineDiagnostics,
};

pub(crate) fn snapshot(state: &AppState) -> MarketDataDiagnosticsSnapshot {
    let now_ms = common::time::now_ms();
    let cache = state.market_data();
    let stats = cache.stats_snapshot();
    let access_rows = cache
        .cache_access_metrics_snapshot()
        .into_iter()
        .map(|metric| MarketCacheAccessRow {
            feed: metric.key.feed.to_owned(),
            outcome: metric.key.outcome.to_owned(),
            source: source(metric.key.source),
            quality: quality(metric.key.quality),
            count: metric.count,
        })
        .collect::<Vec<_>>();
    let access_total = access_rows.iter().map(|row| row.count).sum();

    MarketDataDiagnosticsSnapshot {
        generated_at_ms: now_ms,
        cache: MarketDataCacheCounters {
            hit_total: stats.cache_hit_total,
            miss_total: stats.cache_miss_total,
            stale_total: stats.cache_stale_total,
            hit_ratio: stats.cache_hit_ratio,
            perp_ticker_snapshot_served_stale_total: stats.perp_ticker_snapshot_served_stale_total,
            spot_tick_snapshot_served_stale_total: stats.spot_tick_snapshot_served_stale_total,
        },
        rest_baseline: RestBaselineDiagnostics {
            orderbook_guard_keys: stats.rest_baseline_orderbook_guard_keys as u64,
            orderbook_in_flight: stats.rest_baseline_orderbook_in_flight as u64,
            orderbook_wait_count_total: stats.rest_baseline_orderbook_wait_count_total,
            orderbook_wait_ms_total: stats.rest_baseline_orderbook_wait_ms_total,
            orderbook_guard_evicted_total: stats.rest_baseline_orderbook_guard_evicted_total,
            orderbook_guard_oldest_idle_ms: stats.rest_baseline_orderbook_guard_oldest_idle_ms,
            snapshot_feed_keys: stats.rest_baseline_snapshot_feed_keys as u64,
            snapshot_feed_in_flight: stats.rest_baseline_snapshot_feed_in_flight as u64,
            snapshot_wait_count_total: stats.rest_baseline_snapshot_wait_count_total,
            snapshot_wait_ms_total: stats.rest_baseline_snapshot_wait_ms_total,
        },
        status: cache.snapshot_status(now_ms),
        access_rows,
        access_total,
    }
}

fn quality(value: MarketQuality) -> MarketDataQuality {
    match value {
        MarketQuality::Fresh => MarketDataQuality::Fresh,
        MarketQuality::Warming => MarketDataQuality::Unverified,
        MarketQuality::StaleAllowed => MarketDataQuality::StaleAllowed,
        MarketQuality::Missing => MarketDataQuality::Missing,
        MarketQuality::RateLimited => MarketDataQuality::RateLimited,
        MarketQuality::CircuitOpen => MarketDataQuality::CircuitOpen,
        MarketQuality::Unsupported => MarketDataQuality::Unsupported,
    }
}

fn source(value: MarketSource) -> MarketDataSourceKind {
    match value {
        MarketSource::WsPush => MarketDataSourceKind::WsPush,
        MarketSource::RestColdStart => MarketDataSourceKind::RestColdStart,
        MarketSource::RestBaseline => MarketDataSourceKind::RestBaseline,
        MarketSource::LocalCache => MarketDataSourceKind::LocalCache,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use common::config::AppConfig;

    #[tokio::test]
    async fn snapshot_exposes_empty_cache_diagnostics() -> anyhow::Result<()> {
        let mut config = AppConfig::default();
        config.history.enabled = false;
        let state = AppState::new(config).await?;

        let snapshot = snapshot(&state);

        assert_eq!(snapshot.cache.hit_total, 0);
        assert_eq!(snapshot.rest_baseline.orderbook_guard_keys, 0);
        // 4 个 per-operation 空缓存行 + 8 个 venue 的 fee_schedule evidence 行。
        // 全市场 orderbook 基线已移除，深度只在候选确认/构建/下单前按需读取。
        assert_eq!(snapshot.status.rows.len(), 12);
        assert_eq!(snapshot.access_total, 0);
        Ok(())
    }
}
