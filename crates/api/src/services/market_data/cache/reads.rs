use super::*;

impl MarketDataCache {
    pub(super) fn store_tickers(&self, rows: &[TickerInfo], source: MarketSource) -> usize {
        let mut content_changed = 0;
        let mut projection_changed = false;
        for row in rows {
            let outcome = self.store_ticker_row(row, source);
            content_changed += usize::from(outcome.content_changed);
            projection_changed |= outcome.projection_changed;
        }
        if projection_changed {
            self.mark_perp_ticker_projection_dirty();
        }
        content_changed
    }

    pub(crate) fn store_spot_ticks(&self, rows: &[SpotTick], source: MarketSource) -> usize {
        let mut content_changed = 0;
        let mut projection_changed = false;
        for row in rows {
            let outcome = self.store_spot_tick(row, source);
            content_changed += usize::from(outcome.content_changed);
            projection_changed |= outcome.projection_changed;
        }
        if projection_changed {
            self.mark_spot_tick_projection_dirty();
        }
        content_changed
    }

    pub(super) fn read_orderbook_after_blocked_fetch(
        &self,
        key: &MarketKey,
        now_ms: i64,
        retry_after_ms: Option<i64>,
        quality: MarketQuality,
        last_error: Option<String>,
    ) -> MarketRead<OrderBookInfo> {
        if let Some(entry) = self.orderbooks.get(key) {
            let age_ms = now_ms.saturating_sub(entry.received_at_ms);
            if age_ms <= ORDERBOOK_STALE_MS {
                self.record_cache_stale_for_feed(FEED_ORDERBOOK, entry.source);
                return MarketRead::stale(
                    entry.data.clone(),
                    age_ms,
                    retry_after_ms.or_else(|| cached_retry_after_ms(&entry, now_ms)),
                    last_error.or_else(|| entry.last_error.clone()),
                );
            }
        }
        MarketRead::unavailable(quality, retry_after_ms, last_error)
    }

    pub(super) fn read_cached_orderbook_or_missing(
        &self,
        key: &MarketKey,
        now_ms: i64,
    ) -> MarketRead<OrderBookInfo> {
        self.read_cached_or_missing(FEED_ORDERBOOK, &self.orderbooks, key, now_ms)
    }

    pub(super) fn read_index_composition_after_blocked_fetch(
        &self,
        key: &MarketKey,
        now_ms: i64,
        retry_after_ms: Option<i64>,
        quality: MarketQuality,
        last_error: Option<String>,
    ) -> MarketRead<IndexCompositionSnapshot> {
        if let Some(entry) = self.index_compositions.get(key) {
            let age_ms = now_ms.saturating_sub(entry.received_at_ms);
            if age_ms <= INDEX_COMPOSITION_STALE_MS {
                self.record_cache_stale_for_feed(FEED_INDEX_COMPOSITION, entry.source);
                return MarketRead::stale(
                    entry.data.clone(),
                    age_ms,
                    retry_after_ms.or_else(|| cached_retry_after_ms(&entry, now_ms)),
                    last_error.or_else(|| entry.last_error.clone()),
                );
            }
        }
        MarketRead::unavailable(quality, retry_after_ms, last_error)
    }

    pub(super) fn read_cached_spot_orderbook_or_missing(
        &self,
        key: &MarketKey,
        now_ms: i64,
    ) -> MarketRead<OrderBookInfo> {
        self.read_cached_or_missing(FEED_SPOT_ORDERBOOK, &self.spot_orderbooks, key, now_ms)
    }

    pub(super) fn read_cached_or_missing<T: Clone>(
        &self,
        feed: &'static str,
        entries: &DashMap<MarketKey, CachedEntry<T>>,
        key: &MarketKey,
        now_ms: i64,
    ) -> MarketRead<T> {
        if let Some(entry) = entries.get(key) {
            let age_ms = now_ms.saturating_sub(entry.received_at_ms);
            if age_ms <= ORDERBOOK_STALE_MS {
                self.record_cache_stale_for_feed(feed, entry.source);
                return MarketRead::stale(
                    entry.data.clone(),
                    age_ms,
                    cached_retry_after_ms(&entry, now_ms),
                    entry.last_error.clone(),
                );
            }
        }
        self.record_cache_miss_for_feed(feed);
        MarketRead::unavailable(MarketQuality::Missing, None, None)
    }

    pub(super) fn read_spot_orderbook_after_blocked_fetch(
        &self,
        key: &MarketKey,
        now_ms: i64,
        retry_after_ms: Option<i64>,
        quality: MarketQuality,
        last_error: Option<String>,
    ) -> MarketRead<OrderBookInfo> {
        if let Some(entry) = self.spot_orderbooks.get(key) {
            let age_ms = now_ms.saturating_sub(entry.received_at_ms);
            if age_ms <= ORDERBOOK_STALE_MS {
                self.record_cache_stale_for_feed(FEED_SPOT_ORDERBOOK, entry.source);
                return MarketRead::stale(
                    entry.data.clone(),
                    age_ms,
                    retry_after_ms.or_else(|| cached_retry_after_ms(&entry, now_ms)),
                    last_error.or_else(|| entry.last_error.clone()),
                );
            }
        }
        MarketRead::unavailable(quality, retry_after_ms, last_error)
    }

    pub(super) fn record_orderbook_error(
        &self,
        key: &MarketKey,
        now_ms: i64,
        quality: MarketQuality,
        backoff_ms: i64,
        last_error: &str,
    ) {
        let until_ms = now_ms.saturating_add(backoff_ms);
        self.orderbook_fetch_backoff.insert(
            key.clone(),
            MarketFetchBackoff {
                until_ms,
                quality,
                last_error: Some(last_error.to_owned()),
            },
        );
        if let Some(mut entry) = self.orderbooks.get_mut(key) {
            entry.retry_after_until_ms = Some(until_ms);
            entry.last_error = Some(last_error.to_owned());
        }
    }

    pub(super) fn record_spot_orderbook_error(
        &self,
        key: &MarketKey,
        now_ms: i64,
        quality: MarketQuality,
        backoff_ms: i64,
        last_error: &str,
    ) {
        let until_ms = now_ms.saturating_add(backoff_ms);
        self.spot_orderbook_fetch_backoff.insert(
            key.clone(),
            MarketFetchBackoff {
                until_ms,
                quality,
                last_error: Some(last_error.to_owned()),
            },
        );
        if let Some(mut entry) = self.spot_orderbooks.get_mut(key) {
            entry.retry_after_until_ms = Some(until_ms);
            entry.last_error = Some(last_error.to_owned());
        }
    }

    pub(super) fn record_index_composition_error(
        &self,
        backoff_key: &MarketKey,
        value_key: &MarketKey,
        backoff: MarketFetchBackoff,
    ) {
        let until_ms = backoff.until_ms;
        let last_error = backoff.last_error.clone();
        self.index_composition_fetch_backoff
            .insert(backoff_key.clone(), backoff);
        if let Some(mut entry) = self.index_compositions.get_mut(value_key) {
            entry.retry_after_until_ms = Some(until_ms);
            entry.last_error = last_error;
        }
    }
}
