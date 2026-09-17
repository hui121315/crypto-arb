use super::*;

impl MarketDataCache {
    pub(crate) fn stats_snapshot(&self) -> MarketDataStats {
        let hits = self.cache_hit_total.load(Ordering::Relaxed);
        let misses = self.cache_miss_total.load(Ordering::Relaxed);
        let stale = self.cache_stale_total.load(Ordering::Relaxed);
        let rest_baseline = self.rest_baseline.stats_snapshot();
        MarketDataStats {
            cache_hit_total: hits,
            cache_miss_total: misses,
            cache_stale_total: stale,
            cache_hit_ratio: hit_ratio(hits, misses),
            rest_baseline_orderbook_guard_keys: rest_baseline.orderbook_keys,
            rest_baseline_orderbook_in_flight: rest_baseline.orderbook_in_flight,
            rest_baseline_orderbook_wait_count_total: rest_baseline.orderbook_wait_count_total,
            rest_baseline_orderbook_wait_ms_total: rest_baseline.orderbook_wait_ms_total,
            rest_baseline_orderbook_guard_evicted_total: rest_baseline
                .orderbook_guard_evicted_total,
            rest_baseline_orderbook_guard_oldest_idle_ms: rest_baseline
                .orderbook_guard_oldest_idle_ms,
            rest_baseline_snapshot_feed_keys: rest_baseline.snapshot_feed_keys,
            rest_baseline_snapshot_feed_in_flight: rest_baseline.snapshot_feed_in_flight,
            rest_baseline_snapshot_wait_count_total: rest_baseline.snapshot_wait_count_total,
            rest_baseline_snapshot_wait_ms_total: rest_baseline.snapshot_wait_ms_total,
            perp_ticker_snapshot_served_stale_total: self
                .perp_ticker_snapshot_served_stale_total
                .load(Ordering::Relaxed),
            spot_tick_snapshot_served_stale_total: self
                .spot_tick_snapshot_served_stale_total
                .load(Ordering::Relaxed),
        }
    }

    pub(crate) fn runtime_health_snapshot(&self) -> Vec<MarketRuntimeHealth> {
        let now_ms = common::time::now_ms();
        self.prune_runtime_health(now_ms);
        let mut rows: Vec<_> = self
            .runtime_health
            .iter()
            .map(|entry| stale_adjusted_runtime_health(entry.value().clone(), now_ms))
            .collect();
        rows.sort_by(|left, right| {
            left.venue
                .cmp(&right.venue)
                .then_with(|| left.operation.cmp(right.operation))
        });
        rows
    }

    pub(crate) fn cache_access_metrics_snapshot(&self) -> Vec<MarketCacheAccessMetric> {
        let mut rows: Vec<_> = self
            .cache_access
            .iter()
            .map(|entry| MarketCacheAccessMetric {
                key: *entry.key(),
                count: entry.value().load(Ordering::Relaxed),
            })
            .collect();
        rows.sort_by(|left, right| {
            left.key
                .feed
                .cmp(right.key.feed)
                .then_with(|| left.key.outcome.cmp(right.key.outcome))
                .then_with(|| left.key.source.as_str().cmp(right.key.source.as_str()))
                .then_with(|| left.key.quality.as_str().cmp(right.key.quality.as_str()))
        });
        rows
    }
}
