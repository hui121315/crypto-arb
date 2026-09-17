use super::*;

impl MarketDataCache {
    pub(super) fn record_cache_hit(&self) {
        self.cache_hit_total.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_cache_hit_for_feed(&self, feed: &'static str) {
        self.record_cache_hit();
        self.record_cache_access(
            feed,
            CACHE_OUTCOME_HIT,
            MarketSource::LocalCache,
            MarketQuality::Fresh,
        );
    }

    pub(super) fn record_cache_hit_for_read<T>(&self, feed: &'static str, read: &MarketRead<T>) {
        self.record_cache_hit();
        self.record_cache_access(feed, CACHE_OUTCOME_HIT, read.source, read.quality);
    }

    pub(super) fn record_cache_miss(&self) {
        self.cache_miss_total.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_cache_miss_for_feed(&self, feed: &'static str) {
        self.record_cache_miss();
        self.record_cache_access(
            feed,
            CACHE_OUTCOME_MISS,
            MarketSource::LocalCache,
            MarketQuality::Missing,
        );
    }

    pub(super) fn record_cache_stale(&self) {
        self.cache_stale_total.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_cache_stale_for_feed(&self, feed: &'static str, source: MarketSource) {
        self.record_cache_stale();
        self.record_cache_access(
            feed,
            CACHE_OUTCOME_STALE,
            source,
            MarketQuality::StaleAllowed,
        );
    }

    pub(super) fn record_cache_access(
        &self,
        feed: &'static str,
        outcome: &'static str,
        source: MarketSource,
        quality: MarketQuality,
    ) {
        let key = MarketCacheAccessKey {
            feed,
            outcome,
            source,
            quality,
        };
        self.cache_access
            .entry(key)
            .or_insert_with(|| AtomicU64::new(0))
            .fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn record_snapshot_stale_served(&self, feed: SnapshotFeed) {
        match feed {
            SnapshotFeed::PerpTickers => {
                self.record_cache_stale_for_feed(FEED_PERP_TICKERS, MarketSource::LocalCache);
                self.perp_ticker_snapshot_served_stale_total
                    .fetch_add(1, Ordering::Relaxed);
            }
            SnapshotFeed::SpotTicks => {
                self.record_cache_stale_for_feed(FEED_SPOT_TICKS, MarketSource::LocalCache);
                self.spot_tick_snapshot_served_stale_total
                    .fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(super) fn upsert_runtime_health(&self, health: MarketRuntimeHealth) {
        self.prune_runtime_health(health.observed_at_ms);
        let key = MarketRuntimeKey {
            venue: health.venue.clone(),
            operation: health.operation,
        };
        if self
            .runtime_health
            .get(&key)
            .is_some_and(|current| fresh_ws_feed_wins(&current, &health))
        {
            return;
        }
        self.runtime_health.insert(key, health);
    }

    pub(super) fn prune_runtime_health(&self, now_ms: i64) {
        self.runtime_health.retain(|_, row| {
            now_ms.saturating_sub(row.observed_at_ms) <= runtime_health_ttl_ms(row.operation)
        });
        self.ws_warmups.retain(|key, state| {
            now_ms.saturating_sub(state.last_seen_ms) <= runtime_health_ttl_ms(key.operation)
        });
    }
}

fn fresh_ws_feed_wins(current: &MarketRuntimeHealth, incoming: &MarketRuntimeHealth) -> bool {
    if current.source != MarketSource::WsPush
        || current.quality != MarketQuality::Fresh
        || incoming.source == MarketSource::WsPush
    {
        return false;
    }
    let freshness_ms = match incoming.operation {
        MARKET_OP_PERP_TICKERS | MARKET_OP_SPOT_TICKS => TICKER_FRESH_MS,
        MARKET_OP_FUNDING_RATES => FUNDING_FRESH_MS,
        _ => return false,
    };
    incoming
        .observed_at_ms
        .saturating_sub(current.observed_at_ms)
        <= freshness_ms
}
