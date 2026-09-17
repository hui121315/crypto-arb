use super::*;

impl MarketDataCache {
    #[cfg(test)]
    pub(super) async fn load_perp_tickers(
        &self,
        producer: &dyn MarketDataSource,
        source: MarketSource,
    ) -> Vec<TickerInfo> {
        let now_ms = common::time::now_ms();
        let cached = fresh_values(&self.tickers, now_ms, TICKER_FRESH_MS);
        if !cached.is_empty() {
            self.record_cache_hit_for_feed(FEED_PERP_TICKERS);
            return cached;
        }
        let stale = fresh_values(&self.tickers, now_ms, TICKER_STALE_MS);
        let _guard = match self
            .rest_baseline
            .try_snapshot_guard(SnapshotFeed::PerpTickers)
        {
            Some(guard) => guard,
            None if !stale.is_empty() => {
                self.record_snapshot_stale_served(SnapshotFeed::PerpTickers);
                return stale;
            }
            None => {
                self.rest_baseline
                    .snapshot_guard(SnapshotFeed::PerpTickers)
                    .await
            }
        };
        let refresh_ms = common::time::now_ms();
        let cached = fresh_values(&self.tickers, refresh_ms, TICKER_FRESH_MS);
        if !cached.is_empty() {
            self.record_cache_hit_for_feed(FEED_PERP_TICKERS);
            return cached;
        }
        self.record_cache_miss_for_feed(FEED_PERP_TICKERS);
        let report = producer.fetch_perp_tickers_report().await;
        self.record_fanout_outcomes(source, report.venues);
        let rows = report.rows;
        self.store_tickers(&rows, source);
        rows
    }

    pub(super) async fn load_spot_ticks(
        &self,
        producer: &dyn MarketDataSource,
        source: MarketSource,
    ) -> Vec<SpotTick> {
        let now_ms = common::time::now_ms();
        let cached = fresh_values(&self.spot_ticks, now_ms, TICKER_FRESH_MS);
        if !cached.is_empty() {
            self.record_cache_hit_for_feed(FEED_SPOT_TICKS);
            return cached;
        }
        let stale = fresh_values(&self.spot_ticks, now_ms, TICKER_STALE_MS);
        let _guard = match self
            .rest_baseline
            .try_snapshot_guard(SnapshotFeed::SpotTicks)
        {
            Some(guard) => guard,
            None if !stale.is_empty() => {
                self.record_snapshot_stale_served(SnapshotFeed::SpotTicks);
                return stale;
            }
            None => {
                self.rest_baseline
                    .snapshot_guard(SnapshotFeed::SpotTicks)
                    .await
            }
        };
        let refresh_ms = common::time::now_ms();
        let cached = fresh_values(&self.spot_ticks, refresh_ms, TICKER_FRESH_MS);
        if !cached.is_empty() {
            self.record_cache_hit_for_feed(FEED_SPOT_TICKS);
            return cached;
        }
        self.record_cache_miss_for_feed(FEED_SPOT_TICKS);
        let report = producer.fetch_spot_ticks_report().await;
        self.record_fanout_outcomes(source, report.venues);
        let rows = report.rows;
        self.store_spot_ticks(&rows, source);
        rows
    }
}
