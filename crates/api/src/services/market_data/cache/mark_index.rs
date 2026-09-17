use super::*;
use dashmap::mapref::entry::Entry;

impl MarketDataCache {
    pub(crate) fn store_mark_index_rows(
        &self,
        rows: &[MarkIndexInfo],
        source: MarketSource,
    ) -> usize {
        rows.iter()
            .filter(|row| self.store_mark_index_row(row, source).content_changed)
            .count()
    }

    pub(crate) fn fresh_ws_mark_index(
        &self,
        venue: &str,
        symbol: &str,
        now_ms: i64,
    ) -> Option<(MarkIndexInfo, i64)> {
        let entry = self.mark_indexes.get(&MarketKey::new(venue, symbol))?;
        let freshness_ms = now_ms.saturating_sub(entry.received_at_ms);
        (entry.source == MarketSource::WsPush && freshness_ms <= MARK_INDEX_FRESH_MS)
            .then(|| (entry.data.clone(), entry.received_at_ms))
    }

    fn store_mark_index_row(
        &self,
        row: &MarkIndexInfo,
        source: MarketSource,
    ) -> MarketStoreOutcome {
        if !row.mark_price.is_finite() || row.mark_price <= 0.0 {
            return MarketStoreOutcome::default();
        }
        let key = MarketKey::new(&row.exchange, &row.symbol);
        match self.mark_indexes.entry(key) {
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
                if super::store::should_keep_fresh_ws(current, source, MARK_INDEX_FRESH_MS)
                    || row.timestamp < current.data.timestamp
                {
                    return MarketStoreOutcome::default();
                }
                let changed = mark_index_content_changed(&current.data, row);
                let advanced = row.timestamp > current.data.timestamp;
                if changed || advanced || current.source != source {
                    entry.insert(CachedEntry::new(
                        row.clone(),
                        common::time::now_ms(),
                        source,
                    ));
                }
                MarketStoreOutcome {
                    content_changed: changed,
                    projection_changed: false,
                }
            }
        }
    }
}

fn mark_index_content_changed(current: &MarkIndexInfo, incoming: &MarkIndexInfo) -> bool {
    current.mark_price.to_bits() != incoming.mark_price.to_bits()
        || super::store::option_f64_changed(current.index_price, incoming.index_price)
        || super::store::option_f64_changed(current.open_interest, incoming.open_interest)
        || super::store::option_f64_changed(
            current.open_interest_value,
            incoming.open_interest_value,
        )
}
