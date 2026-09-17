use super::*;

impl MarketDataCache {
    pub(crate) async fn index_composition_or_fetch(
        &self,
        producer: &dyn MarketDataSource,
        venue: &str,
        symbol: &str,
        now_ms: i64,
    ) -> MarketRead<IndexCompositionSnapshot> {
        let key = MarketKey::new(venue, symbol);
        if let Some(read) = read_entry(
            &self.index_compositions,
            &key,
            now_ms,
            INDEX_COMPOSITION_FRESH_MS,
        ) {
            self.record_cache_hit_for_read(FEED_INDEX_COMPOSITION, &read);
            return read;
        }

        let guard_key = index_composition_guard_key(venue);
        if let Some(backoff) =
            fetch_backoff_wait(&self.index_composition_fetch_backoff, &guard_key, now_ms)
        {
            return self.read_index_composition_after_blocked_fetch(
                &key,
                now_ms,
                Some(backoff.wait_ms),
                backoff.quality,
                backoff.last_error,
            );
        }
        let _guard = self.rest_baseline.orderbook_guard(&guard_key).await;
        let refresh_ms = common::time::now_ms();
        if let Some(read) = read_entry(
            &self.index_compositions,
            &key,
            refresh_ms,
            INDEX_COMPOSITION_FRESH_MS,
        ) {
            self.record_cache_hit_for_read(FEED_INDEX_COMPOSITION, &read);
            return read;
        }
        if let Some(backoff) = fetch_backoff_wait(
            &self.index_composition_fetch_backoff,
            &guard_key,
            refresh_ms,
        ) {
            return self.read_index_composition_after_blocked_fetch(
                &key,
                refresh_ms,
                Some(backoff.wait_ms),
                backoff.quality,
                backoff.last_error,
            );
        }

        self.record_cache_miss_for_feed(FEED_INDEX_COMPOSITION);
        match producer.fetch_index_composition(venue, symbol).await {
            Ok(snapshot) => {
                self.index_composition_fetch_backoff.remove(&guard_key);
                self.store_index_compositions(
                    std::slice::from_ref(&snapshot),
                    MarketSource::RestBaseline,
                );
                MarketRead::fresh(snapshot, 0, MarketSource::RestBaseline)
            }
            Err(error) => {
                let backoff_ms = index_composition_backoff_ms(&error);
                let error_text = error.to_string();
                let quality = error_quality(&error);
                self.record_runtime_symbol_error(
                    venue,
                    MARKET_OP_REST_INDEX_COMPOSITIONS,
                    MarketSource::RestBaseline,
                    symbol,
                    &error,
                );
                self.record_index_composition_error(
                    &guard_key,
                    &key,
                    MarketFetchBackoff {
                        until_ms: refresh_ms.saturating_add(backoff_ms),
                        quality,
                        last_error: Some(error_text.clone()),
                    },
                );
                self.read_index_composition_after_blocked_fetch(
                    &key,
                    refresh_ms,
                    Some(backoff_ms),
                    quality,
                    Some(error_text),
                )
            }
        }
    }

    pub(crate) async fn prewarm_public_perp_tickers_for_venue(
        &self,
        producer: &dyn MarketDataSource,
        source: MarketSource,
        venue: &str,
    ) -> usize {
        let _guard = self
            .rest_baseline
            .snapshot_guard(SnapshotFeed::PerpTickers)
            .await;
        let report = producer.fetch_perp_tickers_for_venue_report(venue).await;
        self.record_fanout_outcomes(source, report.venues);
        let rows = report.rows;
        self.store_tickers(&rows, source);
        rows.len()
    }

    pub(crate) async fn prewarm_public_spot_ticks_for_venue(
        &self,
        producer: &dyn MarketDataSource,
        source: MarketSource,
        venue: &str,
    ) -> usize {
        let _guard = self
            .rest_baseline
            .snapshot_guard(SnapshotFeed::SpotTicks)
            .await;
        let report = producer.fetch_spot_ticks_for_venue_report(venue).await;
        self.record_fanout_outcomes(source, report.venues);
        let rows = report.rows;
        self.store_spot_ticks(&rows, source);
        rows.len()
    }

    pub(crate) async fn spot_ticks_snapshot(
        &self,
        producer: &dyn MarketDataSource,
    ) -> Vec<SpotTick> {
        self.load_spot_ticks(producer, MarketSource::RestBaseline)
            .await
    }

    pub(crate) fn spot_tick_row_evidence(&self, rows: &[SpotTick]) -> Vec<MarketDataRowEvidence> {
        cached_row_evidence_for(
            &self.spot_ticks,
            rows,
            common::time::now_ms(),
            TICKER_FRESH_MS,
            MarketDataSnapshotOperation::SpotTicks,
            |row| {
                (
                    row.venue.clone(),
                    row.symbol.clone(),
                    MarketKey::spot(&row.venue, &row.symbol),
                )
            },
        )
    }
}
