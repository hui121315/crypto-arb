use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::services::market_data::cache::{
    WsRuntimeSample, MARKET_OP_FUNDING_RATES, MARKET_OP_MARK_INDEX, MARKET_OP_PERP_TICKERS,
    MARKET_OP_REST_METADATA, MARKET_OP_SPOT_TICKS, MARKET_OP_WS_FUNDING_SNAPSHOT,
    MARKET_OP_WS_FUNDING_SUBSCRIBE, MARKET_OP_WS_MARK_INDEX_SNAPSHOT,
    MARKET_OP_WS_MARK_INDEX_SUBSCRIBE, MARKET_OP_WS_SPOT_SNAPSHOT, MARKET_OP_WS_TICKER_SNAPSHOT,
    MARKET_OP_WS_TICKER_SUBSCRIBE,
};
#[cfg(test)]
use crate::services::market_data::cache::{
    MARKET_OP_REST_FUNDING_FALLBACK, MARKET_OP_REST_SPOT_TICKS, MARKET_OP_REST_TICKER_FALLBACK,
};
use crate::services::market_data::{MarketDataCache, MarketSource, PublicBaselineStats};
use crate::state::AppState;
use shared_types::{normalized_venue_name, VenueId};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

mod baseline;
mod live;
mod metadata;
mod prewarm;
mod requests;
mod watchlist_runtime;
mod ws_touch;

use baseline::{
    advance_baseline_cursor, baseline_shard_at, BaselineFeed, BaselineRefresh, BaselineShard,
};
use metadata::prewarm_metadata;
use prewarm::{prewarm_market_data, prewarm_request_count, prewarm_task_outcome};
use requests::watchlist_ticker_requests;
use watchlist_runtime::refresh_watchlist_runtime;

// Active rows stay on the independent 100ms WS projection path. The 15s task
// rotates one venue/feed discovery shard at a time instead of downloading all
// spot and perpetual markets from every venue in one burst.
const WATCHLIST_PREWARM_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);
const COLD_START_WS_SETTLE_DELAY: std::time::Duration = std::time::Duration::from_secs(12);
const WATCHLIST_TICKER_SYMBOLS_PER_VENUE: usize = 32;
const METADATA_REFRESH_EVERY_TICKS: u64 = 360;

/// Public WS adapters touched each cycle. Each adapter now reports ticker and
/// funding support through typed snapshot methods, so this list only controls
/// fanout and no longer claims operation support.
const WS_PREWARM_VENUES: &[VenueId] = &[
    VenueId::Bybit,
    VenueId::Gate,
    VenueId::Bitget,
    VenueId::Binance,
    VenueId::Hyperliquid,
    VenueId::Kucoin,
    VenueId::Okx,
    VenueId::Kraken,
    VenueId::GateCrossEx,
];

pub(super) fn spawn_prewarm(state: &AppState, tasks: &mut BackgroundTasks) {
    live::spawn_live_ingest(state, tasks);
    let state = state.clone();
    let shutdown = tasks.shutdown_token();
    tasks.supervise(
        "market_prewarm",
        WATCHLIST_PREWARM_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_prewarm_task(state, shutdown).await }
        },
    );

    info!(
        watchlist_period_secs = WATCHLIST_PREWARM_INTERVAL.as_secs(),
        baseline_mode = "rotating_venue_feed",
        metadata_every_ticks = METADATA_REFRESH_EVERY_TICKS,
        "public market baseline prewarm started"
    );
}

async fn run_prewarm_task(state: AppState, shutdown: ShutdownToken) {
    let ws_ready = tokio::select! {
        biased;
        () = shutdown.cancelled() => false,
        _ = tokio::time::sleep(COLD_START_WS_SETTLE_DELAY) => true,
    };
    if !ws_ready {
        return;
    }
    let runtime = MarketDataRuntime {
        aggregator: Arc::clone(state.aggregator_handle()),
        market_data: Arc::clone(state.market_data()),
        market_subscriptions: Arc::clone(state.market_subscriptions()),
        watchlist: Arc::clone(state.watchlist()),
        watchlist_alert_store: Arc::clone(state.watchlist_alert_store()),
        hub: state.ws_hub().clone(),
    };
    let registry = state.task_registry().clone();
    let started_at_ms = common::time::now_ms();
    let cold_start_result = run_once(
        &runtime,
        MarketSource::RestColdStart,
        true,
        &BaselineRefresh::None,
    )
    .await;
    registry.record_result_timed("market_prewarm", started_at_ms, cold_start_result);

    let mut tick = tokio::time::interval(WATCHLIST_PREWARM_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.tick().await;
    let mut baseline_cursor = 0usize;
    let mut ticks_since_metadata = 0u64;
    while tick_or_shutdown(&mut tick, &shutdown).await {
        ticks_since_metadata = ticks_since_metadata.saturating_add(1);
        let venue_names = runtime.aggregator.names();
        let baseline_refresh = baseline_shard_at(&venue_names, baseline_cursor)
            .map(BaselineRefresh::Shard)
            .unwrap_or(BaselineRefresh::None);
        baseline_cursor = advance_baseline_cursor(baseline_cursor, venue_names.len());
        let refresh_metadata = ticks_since_metadata >= METADATA_REFRESH_EVERY_TICKS;
        if refresh_metadata {
            ticks_since_metadata = 0;
        }
        let started_at_ms = common::time::now_ms();
        let metadata_result = if refresh_metadata {
            prewarm_metadata(&runtime, MarketSource::RestBaseline).await
        } else {
            Ok(())
        };
        let baseline_result = run_once(
            &runtime,
            MarketSource::RestBaseline,
            false,
            &baseline_refresh,
        )
        .await;
        let result = metadata_result.and(baseline_result);
        registry.record_result_timed("market_prewarm", started_at_ms, result);
    }
}

struct MarketDataRuntime {
    aggregator: Arc<exchange::Aggregator>,
    market_data: Arc<MarketDataCache>,
    market_subscriptions: Arc<crate::services::market_subscriptions::MarketSubscriptions>,
    watchlist: Arc<tokio::sync::RwLock<Vec<realtime::WatchlistItem>>>,
    watchlist_alert_store: Arc<realtime::WatchlistAlertStore>,
    hub: realtime::WsHub,
}

async fn run_once(
    runtime: &MarketDataRuntime,
    source: MarketSource,
    prewarm: bool,
    baseline_refresh: &BaselineRefresh,
) -> Result<(), String> {
    let started = std::time::Instant::now();
    let watchlist = runtime.watchlist.read().await.clone();
    let watchlist_ticker_requests = watchlist_ticker_requests(&watchlist);
    let requested = prewarm_request_count(&watchlist_ticker_requests);
    let outcome = prewarm_market_data(
        runtime,
        source,
        &watchlist_ticker_requests,
        baseline_refresh,
    )
    .await;
    if let Some(envelope) = refresh_watchlist_runtime(
        runtime,
        &watchlist,
        &watchlist_ticker_requests,
        common::time::now_ms(),
    )
    .await
    {
        publish_watchlist_runtime(runtime, envelope)?;
    }
    let elapsed_ms = started.elapsed().as_millis() as u64;
    log_prewarm(elapsed_ms, source, prewarm, baseline_refresh, outcome.stats);
    prewarm_task_outcome(&outcome, baseline_refresh, requested)
}

fn publish_watchlist_runtime(
    runtime: &MarketDataRuntime,
    envelope: shared_types::WatchlistEnvelope,
) -> Result<(), String> {
    let event = shared_types::WatchlistStreamEvent::WatchlistChanged {
        envelope,
        timestamp_ms: common::time::now_ms(),
    };
    let message = realtime::WsMessage::json(&event)
        .map_err(|error| format!("watchlist runtime serialization failed: {error}"))?;
    runtime.hub.publish(realtime::channels::WATCHLIST, message);
    Ok(())
}

fn log_prewarm(
    elapsed_ms: u64,
    source: MarketSource,
    prewarm: bool,
    baseline_refresh: &BaselineRefresh,
    stats: PublicBaselineStats,
) {
    if elapsed_ms >= WATCHLIST_PREWARM_INTERVAL.as_millis() as u64 {
        log_slow_prewarm(elapsed_ms, source, prewarm, baseline_refresh, stats);
    } else {
        log_done_prewarm(elapsed_ms, source, prewarm, baseline_refresh, stats);
    }
}

fn log_slow_prewarm(
    elapsed_ms: u64,
    source: MarketSource,
    prewarm: bool,
    baseline_refresh: &BaselineRefresh,
    stats: PublicBaselineStats,
) {
    warn!(
        elapsed_ms,
        prewarm,
        baseline = baseline_refresh.label(),
        baseline_venue = baseline_refresh.venue(),
        baseline_feed = baseline_refresh.feed(),
        source = ?source,
        perp_tickers = stats.perp_tickers,
        spot_ticks = stats.spot_ticks,
        "public market baseline prewarm slow"
    );
}

fn log_done_prewarm(
    elapsed_ms: u64,
    source: MarketSource,
    prewarm: bool,
    baseline_refresh: &BaselineRefresh,
    stats: PublicBaselineStats,
) {
    info!(
        elapsed_ms,
        prewarm,
        baseline = baseline_refresh.label(),
        baseline_venue = baseline_refresh.venue(),
        baseline_feed = baseline_refresh.feed(),
        source = ?source,
        perp_tickers = stats.perp_tickers,
        spot_ticks = stats.spot_ticks,
        "public market baseline prewarm done"
    );
}

#[cfg(test)]
#[path = "market_data_tests.rs"]
mod tests;
