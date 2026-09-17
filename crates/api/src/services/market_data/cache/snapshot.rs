use super::*;
use rust_decimal::prelude::ToPrimitive;

impl MarketDataCache {
    /// Preserve optional sizes and source time for stock comparisons. No network request.
    pub(crate) fn spot_tick_read(&self, venue: &str, symbol: &str, now_ms: i64) -> MarketRead<SpotTick> {
        read_entry(&self.spot_ticks, &MarketKey::spot(venue,symbol), now_ms, TICKER_FRESH_MS)
            .unwrap_or_else(|| MarketRead::unavailable(MarketQuality::Missing,None,None))
    }
    #[cfg(test)]
    pub(crate) async fn market_snapshot(
        &self,
        producer: &dyn MarketDataSource,
        funding_rows: Vec<FundingRateData>,
    ) -> MarketDataSnapshot {
        let now_ms = common::time::now_ms();
        self.store_funding_rows(&funding_rows, MarketSource::RestBaseline);
        let (perp_tickers, spot_ticks) = tokio::join!(
            self.load_perp_tickers(producer, MarketSource::RestBaseline),
            self.load_spot_ticks(producer, MarketSource::RestBaseline)
        );
        let perp_ticker_row_evidence = self.perp_ticker_row_evidence(&perp_tickers);
        let spot_tick_row_evidence = self.spot_tick_row_evidence(&spot_ticks);
        let funding_row_evidence = self.funding_row_evidence(&funding_rows);
        MarketDataSnapshot {
            funding: Arc::new(super::projection::group_funding(funding_rows)),
            funding_row_evidence,
            perp_tickers: Arc::new(perp_tickers),
            perp_ticker_row_evidence,
            spot_ticks: Arc::new(spot_ticks),
            spot_tick_row_evidence,
            funding_diff_stats: Vec::new(),
            index_compositions: Arc::new(self.index_compositions_snapshot()),
            status: Some(self.opportunity_snapshot_status(now_ms)),
        }
    }

    pub(crate) fn market_snapshot_cached(&self) -> MarketDataSnapshot {
        let now_ms = common::time::now_ms();
        let funding = self.funding_projection(now_ms);
        let perp_tickers = self.perp_ticker_projection(now_ms);
        let spot_ticks = self.spot_tick_projection(now_ms);
        let funding_row_evidence = projected_row_evidence(
            &self.funding,
            &funding.keys,
            now_ms,
            FUNDING_FRESH_MS,
            FUNDING_FRESH_MS,
            MarketDataSnapshotOperation::FundingRates,
        );
        let perp_ticker_row_evidence = projected_row_evidence(
            &self.tickers,
            &perp_tickers.keys,
            now_ms,
            TICKER_FRESH_MS,
            TICKER_DISCOVERY_MAX_AGE_MS,
            MarketDataSnapshotOperation::PerpTickers,
        );
        let spot_tick_row_evidence = projected_row_evidence(
            &self.spot_ticks,
            &spot_ticks.keys,
            now_ms,
            TICKER_FRESH_MS,
            TICKER_DISCOVERY_MAX_AGE_MS,
            MarketDataSnapshotOperation::SpotTicks,
        );
        MarketDataSnapshot {
            funding: Arc::clone(&funding.grouped),
            funding_row_evidence,
            perp_tickers: Arc::clone(&perp_tickers.rows),
            perp_ticker_row_evidence,
            spot_ticks: Arc::clone(&spot_ticks.rows),
            spot_tick_row_evidence,
            funding_diff_stats: Vec::new(),
            index_compositions: self.index_composition_projection(now_ms),
            status: Some(self.opportunity_snapshot_status(now_ms)),
        }
    }

    pub(crate) fn snapshot_status(&self, now_ms: i64) -> MarketDataSnapshotStatus {
        self.build_snapshot_status(now_ms, true)
    }

    fn opportunity_snapshot_status(&self, now_ms: i64) -> MarketDataSnapshotStatus {
        self.build_snapshot_status(now_ms, false)
    }

    fn build_snapshot_status(
        &self,
        now_ms: i64,
        include_execution_orderbooks: bool,
    ) -> MarketDataSnapshotStatus {
        self.prune_runtime_health(now_ms);
        let mut rows = self
            .runtime_health
            .iter()
            .filter_map(|entry| runtime_status_row(entry.value().clone()))
            .collect::<Vec<_>>();
        push_cache_status_row(
            &mut rows,
            &self.funding,
            now_ms,
            CacheStatusSpec {
                fresh_ms: FUNDING_FRESH_MS,
                operation: MarketDataSnapshotOperation::FundingRates,
                runtime_operation: MARKET_OP_REST_FUNDING_RATES,
                missing_message: "funding rates cache has no fresh rows",
            },
        );
        push_cache_status_row(
            &mut rows,
            &self.tickers,
            now_ms,
            CacheStatusSpec {
                fresh_ms: TICKER_FRESH_MS,
                operation: MarketDataSnapshotOperation::PerpTickers,
                runtime_operation: MARKET_OP_REST_PERP_TICKERS,
                missing_message: "perp ticker cache has no fresh rows",
            },
        );
        push_cache_status_row(
            &mut rows,
            &self.spot_ticks,
            now_ms,
            CacheStatusSpec {
                fresh_ms: TICKER_FRESH_MS,
                operation: MarketDataSnapshotOperation::SpotTicks,
                runtime_operation: MARKET_OP_REST_SPOT_TICKS,
                missing_message: "spot tick cache has no fresh rows",
            },
        );
        if include_execution_orderbooks {
            push_cache_status_row(
                &mut rows,
                &self.orderbooks,
                now_ms,
                CacheStatusSpec {
                    fresh_ms: ORDERBOOK_FRESH_MS,
                    operation: MarketDataSnapshotOperation::Orderbooks,
                    runtime_operation: MARKET_OP_WS_ORDERBOOKS,
                    missing_message: "orderbook cache has no fresh rows",
                },
            );
        } else {
            rows.retain(|row| row.operation != MarketDataSnapshotOperation::Orderbooks);
        }
        push_cache_status_row(
            &mut rows,
            &self.index_compositions,
            now_ms,
            CacheStatusSpec {
                fresh_ms: INDEX_COMPOSITION_FRESH_MS,
                operation: MarketDataSnapshotOperation::IndexCompositions,
                runtime_operation: MARKET_OP_REST_INDEX_COMPOSITIONS,
                missing_message: "index composition cache has no fresh rows",
            },
        );
        push_fee_schedule_status_rows(&mut rows, now_ms);
        remove_scan_rows_superseded_by_fresh_ws(&mut rows);
        rows.sort_by(|left, right| {
            left.venue
                .cmp(&right.venue)
                .then_with(|| left.operation.as_str().cmp(right.operation.as_str()))
        });
        MarketDataSnapshotStatus {
            observed_at_ms: now_ms,
            rows,
        }
    }

    pub(crate) fn orderbook_read(
        &self,
        venue: &str,
        symbol: &str,
        now_ms: i64,
    ) -> MarketRead<OrderBookInfo> {
        let key = MarketKey::new(venue, symbol);
        if let Some(read) = read_entry(&self.orderbooks, &key, now_ms, ORDERBOOK_FRESH_MS) {
            self.record_cache_hit_for_read(FEED_ORDERBOOK, &read);
            return read;
        }
        if let Some(backoff) = fetch_backoff_wait(&self.orderbook_fetch_backoff, &key, now_ms) {
            return self.read_orderbook_after_blocked_fetch(
                &key,
                now_ms,
                Some(backoff.wait_ms),
                backoff.quality,
                backoff.last_error,
            );
        }
        self.read_cached_orderbook_or_missing(&key, now_ms)
    }

    pub(crate) fn ticker_read(
        &self,
        venue: &str,
        symbol: &str,
        now_ms: i64,
    ) -> MarketRead<TickerInfo> {
        let key = MarketKey::new(venue, symbol);
        read_entry(&self.tickers, &key, now_ms, TICKER_FRESH_MS)
            .unwrap_or_else(|| MarketRead::unavailable(MarketQuality::Missing, None, None))
    }

    /// Read the selected spot market's WS best bid/ask without opening a
    /// depth subscription. Execution paths still request a full order book.
    pub(crate) fn spot_bbo_read(
        &self,
        venue: &str,
        symbol: &str,
        now_ms: i64,
        max_age_ms: i64,
    ) -> MarketRead<OrderBookInfo> {
        let key = MarketKey::spot(venue, symbol);
        let Some(entry) = self.spot_ticks.get(&key) else {
            self.record_cache_miss_for_feed(FEED_SPOT_TICKS);
            return MarketRead::unavailable(
                MarketQuality::Missing,
                None,
                Some(format!("{venue} {symbol} spot WS BBO has no cached row")),
            );
        };
        let age_ms = now_ms.saturating_sub(entry.received_at_ms);
        let source = entry.source;
        let tick = entry.data.clone();
        drop(entry);

        let Some(book) = spot_tick_as_bbo_book(&tick) else {
            self.record_cache_miss_for_feed(FEED_SPOT_TICKS);
            return MarketRead::unavailable(
                MarketQuality::Missing,
                None,
                Some(format!(
                    "{venue} {symbol} spot WS BBO is missing a valid bid or ask"
                )),
            );
        };
        if age_ms <= max_age_ms.max(1) {
            let read = MarketRead::fresh(book, age_ms, source);
            self.record_cache_hit_for_read(FEED_SPOT_TICKS, &read);
            return read;
        }
        if age_ms <= TICKER_STALE_MS {
            self.record_cache_stale_for_feed(FEED_SPOT_TICKS, source);
            return MarketRead::stale(
                book,
                age_ms,
                None,
                Some(format!("{venue} {symbol} spot WS BBO is stale: {age_ms}ms")),
            );
        }
        self.record_cache_miss_for_feed(FEED_SPOT_TICKS);
        MarketRead::unavailable(
            MarketQuality::Missing,
            None,
            Some(format!("{venue} {symbol} spot WS BBO expired: {age_ms}ms")),
        )
    }

    pub(crate) fn spot_orderbook_read(
        &self,
        venue: &str,
        symbol: &str,
        now_ms: i64,
    ) -> MarketRead<OrderBookInfo> {
        let key = MarketKey::spot(venue, symbol);
        if let Some(read) = read_entry(&self.spot_orderbooks, &key, now_ms, ORDERBOOK_FRESH_MS) {
            self.record_cache_hit_for_read(FEED_SPOT_ORDERBOOK, &read);
            return read;
        }
        if let Some(backoff) = fetch_backoff_wait(&self.spot_orderbook_fetch_backoff, &key, now_ms)
        {
            return self.read_spot_orderbook_after_blocked_fetch(
                &key,
                now_ms,
                Some(backoff.wait_ms),
                backoff.quality,
                backoff.last_error,
            );
        }
        self.read_cached_spot_orderbook_or_missing(&key, now_ms)
    }
}

fn spot_tick_as_bbo_book(tick: &SpotTick) -> Option<OrderBookInfo> {
    let bid = tick.bid.to_f64()?;
    let ask = tick.ask.to_f64()?;
    if !bid.is_finite() || !ask.is_finite() || bid <= 0.0 || ask <= 0.0 || bid > ask {
        return None;
    }
    let bid_size = tick
        .bid_size
        .and_then(|value| value.to_f64())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(0.0);
    let ask_size = tick
        .ask_size
        .and_then(|value| value.to_f64())
        .filter(|value| value.is_finite() && *value > 0.0)
        .unwrap_or(0.0);
    Some(OrderBookInfo {
        symbol: tick.symbol.clone(),
        exchange: tick.venue.clone(),
        bids: vec![[bid, bid_size]],
        asks: vec![[ask, ask_size]],
        timestamp: tick.best_timestamp_ms(),
    })
}
