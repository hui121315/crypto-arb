use arbitrage::MarketDataSnapshot;
use arc_swap::ArcSwapOption;
use dashmap::DashMap;
use exchange::{ExchangeError, FanoutVenueResult};
use shared_types::{
    ExchangeProblem, FundingRateData, IndexCompositionSnapshot, MarkIndexInfo, MarketDataHealth,
    MarketDataQuality as SharedMarketDataQuality, MarketDataRowEvidence,
    MarketDataSnapshotOperation, MarketDataSnapshotStatus, MarketDataSnapshotStatusRow,
    OrderBookInfo, SpotTick, TickerInfo, OP_REST_FUNDING_FALLBACK, OP_REST_FUNDING_RATES,
    OP_REST_INDEX_COMPOSITIONS, OP_REST_METADATA, OP_REST_PERP_TICKERS, OP_REST_SPOT_TICKS,
    OP_REST_TICKER_FALLBACK, OP_WS_FUNDING, OP_WS_FUNDING_SNAPSHOT, OP_WS_FUNDING_SUBSCRIBE,
    OP_WS_SPOT_SNAPSHOT, OP_WS_TICKER, OP_WS_TICKER_SNAPSHOT, OP_WS_TICKER_SUBSCRIBE,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::envelope::{coverage, health_from_runtime};
use super::key::MarketKey;
use super::rest_baseline::{RestBaselineCoordinator, SnapshotFeed};
use super::source::MarketDataSource;

pub(crate) const TICKER_FRESH_MS: i64 = 30_000;
pub(crate) const TICKER_DISCOVERY_MAX_AGE_MS: i64 = 10 * 60_000;
pub(crate) const TICKER_STALE_MS: i64 = 60_000;
const FUNDING_FRESH_MS: i64 = 90_000;
const MARK_INDEX_FRESH_MS: i64 = 3_000;
const ORDERBOOK_FRESH_MS: i64 = 30_000;
const ORDERBOOK_STALE_MS: i64 = 120_000;
const ORDERBOOK_WS_REUSE_MS: i64 = 1_000;
const INDEX_COMPOSITION_FRESH_MS: i64 = 6 * 60 * 60 * 1_000;
const INDEX_COMPOSITION_STALE_MS: i64 = 24 * 60 * 60 * 1_000;
const INDEX_COMPOSITION_NEGATIVE_CACHE_MS: i64 = 60_000;
const METADATA_FRESH_MS: i64 = 24 * 60 * 60 * 1_000;
const RETRY_AFTER_FLOOR_MS: i64 = 1_000;
const ORDERBOOK_NEGATIVE_CACHE_MS: i64 = 2_000;
const ORDERBOOK_UNSUPPORTED_CACHE_MS: i64 = 5 * 60_000;
const WS_ORDERBOOK_ATTEMPTS: usize = 100;
const WS_ORDERBOOK_RETRY_MS: u64 = 50;
const MARKET_RUNTIME_HEALTH_TTL_MS: i64 = 120_000;
pub(crate) const MARKET_AGGREGATE_VENUE: &str = "all";
pub(crate) const MARKET_OP_WS_ORDERBOOKS: &str = "ws_orderbooks";
pub(crate) const MARKET_OP_FUNDING_RATES: &str = "funding_rates";
pub(crate) const MARKET_OP_MARK_INDEX: &str = "mark_index";
pub(crate) const MARKET_OP_PERP_TICKERS: &str = FEED_PERP_TICKERS;
pub(crate) const MARKET_OP_SPOT_TICKS: &str = FEED_SPOT_TICKS;
pub(crate) const MARKET_OP_REST_FUNDING_RATES: &str = OP_REST_FUNDING_RATES;
pub(crate) const MARKET_OP_REST_INDEX_COMPOSITIONS: &str = OP_REST_INDEX_COMPOSITIONS;
pub(crate) const MARKET_OP_REST_METADATA: &str = OP_REST_METADATA;
pub(crate) const MARKET_OP_REST_PERP_TICKERS: &str = OP_REST_PERP_TICKERS;
pub(crate) const MARKET_OP_REST_SPOT_TICKS: &str = OP_REST_SPOT_TICKS;
pub(crate) const MARKET_OP_WS_FUNDING: &str = OP_WS_FUNDING;
pub(crate) const MARKET_OP_WS_TICKER: &str = OP_WS_TICKER;
pub(crate) const MARKET_OP_WS_FUNDING_SUBSCRIBE: &str = OP_WS_FUNDING_SUBSCRIBE;
pub(crate) const MARKET_OP_WS_FUNDING_SNAPSHOT: &str = OP_WS_FUNDING_SNAPSHOT;
pub(crate) const MARKET_OP_WS_MARK_INDEX_SUBSCRIBE: &str = "ws_mark_index_subscribe";
pub(crate) const MARKET_OP_WS_MARK_INDEX_SNAPSHOT: &str = "ws_mark_index_snapshot";
pub(crate) const MARKET_OP_REST_FUNDING_FALLBACK: &str = OP_REST_FUNDING_FALLBACK;
pub(crate) const MARKET_OP_WS_TICKER_SUBSCRIBE: &str = OP_WS_TICKER_SUBSCRIBE;
pub(crate) const MARKET_OP_WS_TICKER_SNAPSHOT: &str = OP_WS_TICKER_SNAPSHOT;
pub(crate) const MARKET_OP_WS_SPOT_SNAPSHOT: &str = OP_WS_SPOT_SNAPSHOT;
pub(crate) const MARKET_OP_REST_TICKER_FALLBACK: &str = OP_REST_TICKER_FALLBACK;
const FEED_ORDERBOOK: &str = "orderbook";
const FEED_SPOT_ORDERBOOK: &str = "spot_orderbook";
const FEED_INDEX_COMPOSITION: &str = "index_composition";
const FEED_PERP_TICKERS: &str = "perp_tickers";
const FEED_SPOT_TICKS: &str = "spot_ticks";
const CACHE_OUTCOME_HIT: &str = "hit";
const CACHE_OUTCOME_MISS: &str = "miss";
const CACHE_OUTCOME_REFRESH: &str = "refresh";
const CACHE_OUTCOME_STALE: &str = "stale";

#[derive(Debug, Clone, Copy, Default)]
struct MarketStoreOutcome {
    content_changed: bool,
    projection_changed: bool,
}

impl MarketStoreOutcome {
    const fn stored(content_changed: bool) -> Self {
        Self {
            content_changed,
            projection_changed: true,
        }
    }
}

mod fee_status;
mod fetch;
mod helpers;
mod helpers2;
mod helpers3;
mod mark_index;
mod metrics;
mod orderbook_ops;
mod orderbook_ws;
mod projection;
mod reads;
mod runtime;
mod runtime_warmup;
mod snapshot;
mod spot_orderbook_ws;
mod stats;
mod store;
mod subscriptions;
#[cfg(test)]
mod tests;
mod types;

use fee_status::*;
use helpers::*;
use helpers2::*;
use helpers3::*;
use projection::{MarketProjectionCache, ProjectedRowKey};
use types::*;
pub(crate) use types::{
    MarketCacheAccessKey, MarketCacheAccessMetric, MarketDataCache, MarketDataStats, MarketQuality,
    MarketRead, MarketRowsSnapshot, MarketRuntimeHealth, MarketSource, PublicBaselineStats,
    WsRuntimeSample,
};
