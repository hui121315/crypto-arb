use super::*;
use dashmap::mapref::entry::Entry;

impl MarketDataCache {
    pub(crate) fn store_funding_rows(
        &self,
        rows: &[FundingRateData],
        source: MarketSource,
    ) -> usize {
        let mut content_changed = 0;
        let mut projection_changed = false;
        for row in rows {
            let outcome = self.store_funding_row(row, source);
            content_changed += usize::from(outcome.content_changed);
            projection_changed |= outcome.projection_changed;
        }
        if projection_changed {
            self.mark_funding_projection_dirty();
        }
        content_changed
    }

    pub(crate) fn store_ticker_rows(&self, rows: &[TickerInfo], source: MarketSource) -> usize {
        self.store_tickers(rows, source)
    }

    pub(super) fn store_spot_tick(
        &self,
        row: &SpotTick,
        source: MarketSource,
    ) -> MarketStoreOutcome {
        let key = MarketKey::spot(&row.venue, &row.symbol);
        match self.spot_ticks.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(CachedEntry::new(
                    row.clone(),
                    common::time::now_ms(),
                    source,
                ));
                MarketStoreOutcome::stored(true)
            }
            Entry::Occupied(mut entry) => {
                let current = entry.get();
                if should_keep_fresh_ws(current, source, TICKER_FRESH_MS)
                    || row.best_timestamp_ms() < current.data.best_timestamp_ms()
                {
                    return MarketStoreOutcome::default();
                }
                let changed = spot_tick_content_changed(&current.data, row);
                let advanced = row.best_timestamp_ms() > current.data.best_timestamp_ms()
                    || row.received_at_ms > current.data.received_at_ms;
                let projection_changed = changed || advanced || current.source != source;
                if projection_changed {
                    entry.insert(CachedEntry::new(
                        row.clone(),
                        common::time::now_ms(),
                        source,
                    ));
                }
                MarketStoreOutcome {
                    content_changed: changed,
                    projection_changed,
                }
            }
        }
    }

    pub(super) fn store_ticker_row(
        &self,
        row: &TickerInfo,
        source: MarketSource,
    ) -> MarketStoreOutcome {
        let key = MarketKey::new(&row.exchange, &row.symbol);
        match self.tickers.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(CachedEntry::new(
                    row.clone(),
                    common::time::now_ms(),
                    source,
                ));
                MarketStoreOutcome::stored(true)
            }
            Entry::Occupied(mut entry) => {
                let current = entry.get();
                if should_keep_fresh_ws(current, source, TICKER_FRESH_MS)
                    || row.timestamp < current.data.timestamp
                {
                    return MarketStoreOutcome::default();
                }
                let changed = ticker_content_changed(&current.data, row);
                let advanced = row.timestamp > current.data.timestamp;
                let projection_changed = changed || advanced || current.source != source;
                if projection_changed {
                    entry.insert(CachedEntry::new(
                        row.clone(),
                        common::time::now_ms(),
                        source,
                    ));
                }
                MarketStoreOutcome {
                    content_changed: changed,
                    projection_changed,
                }
            }
        }
    }

    fn store_funding_row(&self, row: &FundingRateData, source: MarketSource) -> MarketStoreOutcome {
        let key = MarketKey::new(&row.exchange, &row.symbol);
        match self.funding.entry(key) {
            Entry::Vacant(entry) => {
                entry.insert(CachedEntry::new(
                    row.clone(),
                    common::time::now_ms(),
                    source,
                ));
                MarketStoreOutcome::stored(true)
            }
            Entry::Occupied(mut entry) => {
                let current = entry.get();
                if should_keep_fresh_ws(current, source, FUNDING_FRESH_MS)
                    || row.timestamp < current.data.timestamp
                {
                    return MarketStoreOutcome::default();
                }
                let changed = funding_content_changed(&current.data, row);
                // A successful observation is also a freshness heartbeat. Funding
                // values often remain identical for several refresh cycles, so
                // tying received_at_ms to an economic change makes live rows
                // disappear at the cache TTL despite continuous confirmation.
                entry.insert(CachedEntry::new(
                    row.clone(),
                    common::time::now_ms(),
                    source,
                ));
                MarketStoreOutcome::stored(changed)
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn store_orderbook(&self, book: OrderBookInfo, source: MarketSource) {
        self.orderbooks.insert(
            MarketKey::new(&book.exchange, &book.symbol),
            CachedEntry::new(book, common::time::now_ms(), source),
        );
    }

    #[cfg(test)]
    pub(crate) fn store_spot_orderbook(
        &self,
        venue: &str,
        book: OrderBookInfo,
        source: MarketSource,
    ) {
        self.spot_orderbooks.insert(
            MarketKey::spot(venue, &book.symbol),
            CachedEntry::new(book, common::time::now_ms(), source),
        );
    }

    pub(crate) fn funding_rows_snapshot(&self) -> Vec<FundingRateData> {
        self.funding_rows_snapshot_with_evidence().rows
    }

    pub(crate) fn funding_rows_snapshot_with_evidence(
        &self,
    ) -> MarketRowsSnapshot<FundingRateData> {
        fresh_rows_snapshot(
            &self.funding,
            common::time::now_ms(),
            FUNDING_FRESH_MS,
            MarketDataSnapshotOperation::FundingRates,
            |row| (row.exchange.clone(), row.symbol.clone()),
        )
    }

    #[cfg(test)]
    pub(crate) fn funding_row_evidence(
        &self,
        rows: &[FundingRateData],
    ) -> Vec<MarketDataRowEvidence> {
        cached_row_evidence_for(
            &self.funding,
            rows,
            common::time::now_ms(),
            FUNDING_FRESH_MS,
            MarketDataSnapshotOperation::FundingRates,
            |row| {
                (
                    row.exchange.clone(),
                    row.symbol.clone(),
                    MarketKey::new(&row.exchange, &row.symbol),
                )
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn perp_ticker_row_evidence(
        &self,
        rows: &[TickerInfo],
    ) -> Vec<MarketDataRowEvidence> {
        cached_row_evidence_for(
            &self.tickers,
            rows,
            common::time::now_ms(),
            TICKER_FRESH_MS,
            MarketDataSnapshotOperation::PerpTickers,
            |row| {
                (
                    row.exchange.clone(),
                    row.symbol.clone(),
                    MarketKey::new(&row.exchange, &row.symbol),
                )
            },
        )
    }

    pub(crate) fn store_index_compositions(
        &self,
        rows: &[IndexCompositionSnapshot],
        source: MarketSource,
    ) {
        let now_ms = common::time::now_ms();
        for row in rows {
            self.index_compositions.insert(
                MarketKey::new(&row.venue, &row.symbol),
                CachedEntry::new(row.clone(), now_ms, source),
            );
        }
        if !rows.is_empty() {
            self.mark_index_composition_projection_dirty();
        }
    }

    pub(crate) fn index_compositions_snapshot(&self) -> Vec<IndexCompositionSnapshot> {
        fresh_values(
            &self.index_compositions,
            common::time::now_ms(),
            INDEX_COMPOSITION_FRESH_MS,
        )
    }
}

pub(super) fn should_keep_fresh_ws<T>(
    current: &CachedEntry<T>,
    incoming: MarketSource,
    fresh_ms: i64,
) -> bool {
    current.source == MarketSource::WsPush
        && incoming != MarketSource::WsPush
        && common::time::now_ms().saturating_sub(current.received_at_ms) <= fresh_ms
}

fn ticker_content_changed(current: &TickerInfo, incoming: &TickerInfo) -> bool {
    current.bid.to_bits() != incoming.bid.to_bits()
        || current.ask.to_bits() != incoming.ask.to_bits()
        || current.last.to_bits() != incoming.last.to_bits()
        || current.volume_24h.to_bits() != incoming.volume_24h.to_bits()
}

fn funding_content_changed(current: &FundingRateData, incoming: &FundingRateData) -> bool {
    current.rate.to_bits() != incoming.rate.to_bits()
        || current.rate_8h.to_bits() != incoming.rate_8h.to_bits()
        || option_f64_changed(current.predicted_rate, incoming.predicted_rate)
        || current.next_funding_time != incoming.next_funding_time
        || current.funding_interval != incoming.funding_interval
        || current.volume_24h.to_bits() != incoming.volume_24h.to_bits()
        || option_f64_changed(current.smoothed_rate, incoming.smoothed_rate)
        || option_f64_changed(current.rate_std, incoming.rate_std)
        || current.is_outlier != incoming.is_outlier
}

fn spot_tick_content_changed(current: &SpotTick, incoming: &SpotTick) -> bool {
    current.bid != incoming.bid
        || current.ask != incoming.ask
        || current.last != incoming.last
        || current.bid_size != incoming.bid_size
        || current.ask_size != incoming.ask_size
        || current.volume_24h != incoming.volume_24h
}

pub(super) fn option_f64_changed(current: Option<f64>, incoming: Option<f64>) -> bool {
    match (current, incoming) {
        (Some(current), Some(incoming)) => current.to_bits() != incoming.to_bits(),
        (None, None) => false,
        _ => true,
    }
}
