use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PublicBaselineStats {
    pub perp_tickers: usize,
    pub spot_ticks: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct MarketDataStats {
    pub cache_hit_total: u64,
    pub cache_miss_total: u64,
    pub cache_stale_total: u64,
    pub cache_hit_ratio: f64,
    pub rest_baseline_orderbook_guard_keys: usize,
    pub rest_baseline_orderbook_in_flight: usize,
    pub rest_baseline_orderbook_wait_count_total: u64,
    pub rest_baseline_orderbook_wait_ms_total: u64,
    pub rest_baseline_orderbook_guard_evicted_total: u64,
    pub rest_baseline_orderbook_guard_oldest_idle_ms: u64,
    pub rest_baseline_snapshot_feed_keys: usize,
    pub rest_baseline_snapshot_feed_in_flight: usize,
    pub rest_baseline_snapshot_wait_count_total: u64,
    pub rest_baseline_snapshot_wait_ms_total: u64,
    pub perp_ticker_snapshot_served_stale_total: u64,
    pub spot_tick_snapshot_served_stale_total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MarketSource {
    WsPush,
    RestColdStart,
    RestBaseline,
    LocalCache,
}

impl MarketSource {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::WsPush => "ws_push",
            Self::RestColdStart => "rest_cold_start",
            Self::RestBaseline => "rest_baseline",
            Self::LocalCache => "local_cache",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MarketQuality {
    Fresh,
    Warming,
    StaleAllowed,
    Missing,
    RateLimited,
    CircuitOpen,
    Unsupported,
}

impl MarketQuality {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Warming => "warming",
            Self::StaleAllowed => "stale_allowed",
            Self::Missing => "missing",
            Self::RateLimited => "rate_limited",
            Self::CircuitOpen => "circuit_open",
            Self::Unsupported => "unsupported",
        }
    }

    pub(crate) fn problem_code(self) -> &'static str {
        match self {
            Self::Fresh => "MARKET_DATA_FRESH",
            Self::Warming => "MARKET_DATA_WARMING",
            Self::StaleAllowed => "MARKET_DATA_STALE_ALLOWED",
            Self::Missing => "MARKET_DATA_MISSING",
            Self::RateLimited => "MARKET_DATA_RATE_LIMITED",
            Self::CircuitOpen => "MARKET_DATA_CIRCUIT_OPEN",
            Self::Unsupported => "MARKET_DATA_UNSUPPORTED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarketRuntimeHealth {
    pub(crate) venue: String,
    pub(crate) operation: &'static str,
    pub(crate) quality: MarketQuality,
    pub(crate) source: MarketSource,
    pub(crate) requested: u64,
    pub(crate) rows: u64,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) last_error: Option<String>,
    pub(crate) problem: Option<ExchangeProblem>,
    pub(crate) observed_at_ms: i64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct CacheStatusSpec {
    pub(super) fresh_ms: i64,
    pub(super) operation: MarketDataSnapshotOperation,
    pub(super) runtime_operation: &'static str,
    pub(super) missing_message: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct MarketRuntimeKey {
    pub(super) venue: String,
    pub(super) operation: &'static str,
}

#[derive(Debug, Clone, Default)]
pub(super) struct WsWarmupState {
    pub(super) missing_since_ms: std::collections::HashMap<String, i64>,
    pub(super) last_seen_ms: i64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct WsRuntimeSample<'a> {
    pub(crate) venue: &'a str,
    pub(crate) operation: &'static str,
    pub(crate) source: MarketSource,
    pub(crate) requested_symbols: &'a [String],
    pub(crate) rows: usize,
    pub(crate) missing_symbols: &'a [String],
    pub(crate) grace_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MarketFetchBackoff {
    pub(super) until_ms: i64,
    pub(super) quality: MarketQuality,
    pub(super) last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ActiveFetchBackoff {
    pub(super) wait_ms: i64,
    pub(super) quality: MarketQuality,
    pub(super) last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct MarketCacheAccessKey {
    pub feed: &'static str,
    pub outcome: &'static str,
    pub source: MarketSource,
    pub quality: MarketQuality,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MarketCacheAccessMetric {
    pub key: MarketCacheAccessKey,
    pub count: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct MarketRead<T> {
    pub value: Option<T>,
    pub quality: MarketQuality,
    pub freshness_ms: Option<i64>,
    pub source: MarketSource,
    pub retry_after_ms: Option<i64>,
    pub last_error: Option<String>,
}

impl<T> MarketRead<T> {
    pub(super) fn fresh(value: T, freshness_ms: i64, source: MarketSource) -> Self {
        Self {
            value: Some(value),
            quality: MarketQuality::Fresh,
            freshness_ms: Some(freshness_ms.max(0)),
            source,
            retry_after_ms: None,
            last_error: None,
        }
    }

    pub(super) fn stale(
        value: T,
        freshness_ms: i64,
        retry_after_ms: Option<i64>,
        last_error: Option<String>,
    ) -> Self {
        Self {
            value: Some(value),
            quality: MarketQuality::StaleAllowed,
            freshness_ms: Some(freshness_ms.max(0)),
            source: MarketSource::LocalCache,
            retry_after_ms,
            last_error,
        }
    }

    pub(super) fn unavailable(
        quality: MarketQuality,
        retry_after_ms: Option<i64>,
        last_error: Option<String>,
    ) -> Self {
        Self {
            value: None,
            quality,
            freshness_ms: None,
            source: MarketSource::LocalCache,
            retry_after_ms,
            last_error,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct MarketRowsSnapshot<T> {
    pub(crate) rows: Vec<T>,
    pub(crate) row_evidence: Vec<MarketDataRowEvidence>,
}

impl<T> Default for MarketRowsSnapshot<T> {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            row_evidence: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct CachedEntry<T> {
    pub(super) data: T,
    pub(super) received_at_ms: i64,
    pub(super) source: MarketSource,
    pub(super) retry_after_until_ms: Option<i64>,
    pub(super) last_error: Option<String>,
}

#[derive(Default)]
pub(crate) struct MarketDataCache {
    pub(super) funding: DashMap<MarketKey, CachedEntry<FundingRateData>>,
    pub(super) mark_indexes: DashMap<MarketKey, CachedEntry<MarkIndexInfo>>,
    pub(super) tickers: DashMap<MarketKey, CachedEntry<TickerInfo>>,
    pub(super) spot_ticks: DashMap<MarketKey, CachedEntry<SpotTick>>,
    pub(super) orderbooks: DashMap<MarketKey, CachedEntry<OrderBookInfo>>,
    pub(super) spot_orderbooks: DashMap<MarketKey, CachedEntry<OrderBookInfo>>,
    pub(super) index_compositions: DashMap<MarketKey, CachedEntry<IndexCompositionSnapshot>>,
    pub(super) index_composition_fetch_backoff: DashMap<MarketKey, MarketFetchBackoff>,
    pub(super) orderbook_fetch_backoff: DashMap<MarketKey, MarketFetchBackoff>,
    pub(super) spot_orderbook_fetch_backoff: DashMap<MarketKey, MarketFetchBackoff>,
    pub(super) runtime_health: DashMap<MarketRuntimeKey, MarketRuntimeHealth>,
    pub(super) ws_warmups: DashMap<MarketRuntimeKey, WsWarmupState>,
    pub(super) cache_access: DashMap<MarketCacheAccessKey, AtomicU64>,
    pub(super) rest_baseline: RestBaselineCoordinator,
    pub(super) cache_hit_total: AtomicU64,
    pub(super) cache_miss_total: AtomicU64,
    pub(super) cache_stale_total: AtomicU64,
    pub(super) perp_ticker_snapshot_served_stale_total: AtomicU64,
    pub(super) spot_tick_snapshot_served_stale_total: AtomicU64,
    pub(super) projections: MarketProjectionCache,
}

impl<T> CachedEntry<T> {
    pub(super) fn new(data: T, received_at_ms: i64, source: MarketSource) -> Self {
        Self {
            data,
            received_at_ms,
            source,
            retry_after_until_ms: None,
            last_error: None,
        }
    }
}
