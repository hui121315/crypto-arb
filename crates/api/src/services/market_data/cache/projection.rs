use super::*;
use std::collections::HashMap;
use std::sync::Arc;

pub(super) struct FundingProjection {
    pub(super) grouped: Arc<HashMap<String, HashMap<String, FundingRateData>>>,
    pub(super) keys: Arc<Vec<ProjectedRowKey>>,
}

pub(super) struct MarketRowsProjection<T> {
    pub(super) rows: Arc<Vec<T>>,
    pub(super) keys: Arc<Vec<ProjectedRowKey>>,
}

pub(super) struct ProjectedRowKey {
    pub(super) cache_key: MarketKey,
    pub(super) venue: String,
    pub(super) symbol: String,
}

#[derive(Default)]
pub(super) struct MarketProjectionCache {
    funding: ProjectionSlot<FundingProjection>,
    perp_tickers: ProjectionSlot<MarketRowsProjection<TickerInfo>>,
    spot_ticks: ProjectionSlot<MarketRowsProjection<SpotTick>>,
    index_compositions: ProjectionSlot<Vec<IndexCompositionSnapshot>>,
}

struct VersionedProjection<T> {
    version: u64,
    valid_until_ms: i64,
    value: Arc<T>,
}

struct ProjectionSlot<T> {
    version: AtomicU64,
    current: ArcSwapOption<VersionedProjection<T>>,
}

impl<T> Default for ProjectionSlot<T> {
    fn default() -> Self {
        Self {
            version: AtomicU64::new(0),
            current: ArcSwapOption::empty(),
        }
    }
}

impl<T> ProjectionSlot<T> {
    fn mark_dirty(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    fn get_or_build(&self, now_ms: i64, build: impl FnOnce() -> (T, i64)) -> Arc<T> {
        let version = self.version.load(Ordering::Acquire);
        if let Some(current) = self
            .current
            .load_full()
            .filter(|current| current.version == version && now_ms <= current.valid_until_ms)
        {
            return Arc::clone(&current.value);
        }

        let (value, valid_until_ms) = build();
        let value = Arc::new(value);
        if self.version.load(Ordering::Acquire) == version {
            self.current.store(Some(Arc::new(VersionedProjection {
                version,
                valid_until_ms,
                value: Arc::clone(&value),
            })));
        }
        value
    }
}

impl MarketDataCache {
    pub(super) fn mark_funding_projection_dirty(&self) {
        self.projections.funding.mark_dirty();
    }

    pub(super) fn mark_perp_ticker_projection_dirty(&self) {
        self.projections.perp_tickers.mark_dirty();
    }

    pub(super) fn mark_spot_tick_projection_dirty(&self) {
        self.projections.spot_ticks.mark_dirty();
    }

    pub(super) fn mark_index_composition_projection_dirty(&self) {
        self.projections.index_compositions.mark_dirty();
    }

    pub(super) fn funding_projection(&self, now_ms: i64) -> Arc<FundingProjection> {
        self.projections.funding.get_or_build(now_ms, || {
            let (pairs, valid_until_ms) = projected_pairs(&self.funding, now_ms, FUNDING_FRESH_MS);
            let (keys, rows) =
                split_market_pairs(pairs, |row| (row.exchange.clone(), row.symbol.clone()));
            let grouped = Arc::new(group_funding(rows));
            (
                FundingProjection {
                    grouped,
                    keys: Arc::new(keys),
                },
                valid_until_ms,
            )
        })
    }

    pub(super) fn perp_ticker_projection(
        &self,
        now_ms: i64,
    ) -> Arc<MarketRowsProjection<TickerInfo>> {
        self.projections.perp_tickers.get_or_build(now_ms, || {
            projected_market_rows(&self.tickers, now_ms, TICKER_DISCOVERY_MAX_AGE_MS, |row| {
                (row.exchange.clone(), row.symbol.clone())
            })
        })
    }

    pub(super) fn spot_tick_projection(&self, now_ms: i64) -> Arc<MarketRowsProjection<SpotTick>> {
        self.projections.spot_ticks.get_or_build(now_ms, || {
            projected_market_rows(
                &self.spot_ticks,
                now_ms,
                TICKER_DISCOVERY_MAX_AGE_MS,
                |row| (row.venue.clone(), row.symbol.clone()),
            )
        })
    }

    pub(super) fn index_composition_projection(
        &self,
        now_ms: i64,
    ) -> Arc<Vec<IndexCompositionSnapshot>> {
        self.projections
            .index_compositions
            .get_or_build(now_ms, || {
                projected_rows(&self.index_compositions, now_ms, INDEX_COMPOSITION_FRESH_MS)
            })
    }
}

fn projected_market_rows<T: Clone>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    now_ms: i64,
    max_age_ms: i64,
    row_identity: impl Fn(&T) -> (String, String),
) -> (MarketRowsProjection<T>, i64) {
    let (pairs, valid_until_ms) = projected_pairs(entries, now_ms, max_age_ms);
    let (keys, rows) = split_market_pairs(pairs, row_identity);
    (
        MarketRowsProjection {
            rows: Arc::new(rows),
            keys: Arc::new(keys),
        },
        valid_until_ms,
    )
}

fn projected_rows<T: Clone>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    now_ms: i64,
    max_age_ms: i64,
) -> (Vec<T>, i64) {
    let (pairs, valid_until_ms) = projected_pairs(entries, now_ms, max_age_ms);
    (
        pairs.into_iter().map(|(_, row)| row).collect(),
        valid_until_ms,
    )
}

fn projected_pairs<T: Clone>(
    entries: &DashMap<MarketKey, CachedEntry<T>>,
    now_ms: i64,
    max_age_ms: i64,
) -> (Vec<(MarketKey, T)>, i64) {
    let mut valid_until_ms = i64::MAX;
    let mut rows = entries
        .iter()
        .filter_map(|entry| {
            let expires_at_ms = entry.received_at_ms.saturating_add(max_age_ms);
            (now_ms <= expires_at_ms).then(|| {
                valid_until_ms = valid_until_ms.min(expires_at_ms);
                (entry.key().clone(), entry.data.clone())
            })
        })
        .collect::<Vec<_>>();
    rows.sort_unstable_by(|left, right| left.0.cmp(&right.0));
    (rows, valid_until_ms)
}

fn split_market_pairs<T>(
    pairs: Vec<(MarketKey, T)>,
    row_identity: impl Fn(&T) -> (String, String),
) -> (Vec<ProjectedRowKey>, Vec<T>) {
    let mut keys = Vec::with_capacity(pairs.len());
    let mut rows = Vec::with_capacity(pairs.len());
    for (cache_key, row) in pairs {
        let (venue, symbol) = row_identity(&row);
        keys.push(ProjectedRowKey {
            cache_key,
            venue,
            symbol,
        });
        rows.push(row);
    }
    (keys, rows)
}

pub(super) fn group_funding(
    rows: Vec<FundingRateData>,
) -> HashMap<String, HashMap<String, FundingRateData>> {
    let mut grouped = HashMap::new();
    for row in rows {
        grouped
            .entry(row.symbol.clone())
            .or_insert_with(HashMap::new)
            .insert(row.exchange.clone(), row);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn projection_slot_reuses_through_the_inclusive_expiry_boundary() {
        let slot = ProjectionSlot::<Vec<u8>>::default();
        let builds = AtomicUsize::new(0);
        let first = slot.get_or_build(100, || {
            builds.fetch_add(1, Ordering::Relaxed);
            (vec![1], 110)
        });
        let boundary = slot.get_or_build(110, || {
            builds.fetch_add(1, Ordering::Relaxed);
            (vec![2], 120)
        });
        let expired = slot.get_or_build(111, || {
            builds.fetch_add(1, Ordering::Relaxed);
            (vec![3], 120)
        });

        assert!(Arc::ptr_eq(&first, &boundary));
        assert!(!Arc::ptr_eq(&first, &expired));
        assert_eq!(&*expired, &[3]);
        assert_eq!(builds.load(Ordering::Relaxed), 2);
    }
}
