#![allow(clippy::panic)]

use super::super::*;
use exchange::{Aggregator, ExchangeAdapter};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use super::fixtures::*;

#[tokio::test]
async fn execution_spot_orderbook_refresh_reuses_immediate_ws_snapshot() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter {
        spot_ws_ready: true,
        ..OrderbookAdapter::default()
    });
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);

    let first = cache
        .refresh_spot_orderbook_from_ws(&aggregator, "mock", "BTC/USDT", 20, common::time::now_ms())
        .await;
    let second = cache
        .refresh_spot_orderbook_from_ws(&aggregator, "mock", "BTC/USDT", 20, common::time::now_ms())
        .await;

    assert_eq!(first.source, MarketSource::WsPush);
    assert_eq!(second.source, MarketSource::WsPush);
    assert_eq!(adapter.spot_ws_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.spot_orderbook_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn execution_orderbook_refresh_preserves_ws_source_without_rest_fallback() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter {
        perp_ws_ready: true,
        ..OrderbookAdapter::default()
    });
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);

    let first = cache
        .refresh_orderbook_from_ws(&aggregator, "mock", "BTC", 20, common::time::now_ms())
        .await;
    let second = cache
        .refresh_orderbook_from_ws(&aggregator, "mock", "BTC", 20, common::time::now_ms())
        .await;

    assert_eq!(first.quality, MarketQuality::Fresh);
    assert_eq!(first.source, MarketSource::WsPush);
    assert_eq!(second.quality, MarketQuality::Fresh);
    assert_eq!(second.source, MarketSource::WsPush);
    assert_eq!(adapter.perp_ws_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.orderbook_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn market_snapshot_baseline_is_singleflight_per_feed() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter::default());
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);

    let (first, second) = tokio::join!(
        cache.market_snapshot(&aggregator, Vec::new()),
        cache.market_snapshot(&aggregator, Vec::new())
    );

    assert_eq!(adapter.ticker_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.spot_calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.perp_tickers.len(), 1);
    assert_eq!(second.perp_tickers.len(), 1);
    assert_eq!(first.spot_ticks.len(), 1);
    assert_eq!(second.spot_ticks.len(), 1);
}
#[tokio::test]
async fn market_snapshot_returns_bounded_stale_rows_when_baseline_refresh_is_running(
) -> Result<(), String> {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let now_ms = common::time::now_ms();
    cache.tickers.insert(
        MarketKey::new("mock", "BTC"),
        CachedEntry::new(
            ticker("BTC"),
            now_ms.saturating_sub(TICKER_FRESH_MS + 1),
            MarketSource::RestBaseline,
        ),
    );
    cache.spot_ticks.insert(
        MarketKey::spot("mock", "BTC"),
        CachedEntry::new(
            spot_tick("BTC"),
            now_ms.saturating_sub(TICKER_FRESH_MS + 1),
            MarketSource::RestBaseline,
        ),
    );
    let _perp_guard = cache
        .rest_baseline
        .snapshot_guard(SnapshotFeed::PerpTickers)
        .await;
    let _spot_guard = cache
        .rest_baseline
        .snapshot_guard(SnapshotFeed::SpotTicks)
        .await;

    let snapshot = tokio::time::timeout(
        Duration::from_millis(20),
        cache.market_snapshot(&aggregator, Vec::new()),
    )
    .await
    .map_err(|_| "stale snapshot waited for active baseline refresh".to_owned())?;

    assert_eq!(snapshot.perp_tickers.len(), 1);
    assert_eq!(snapshot.spot_ticks.len(), 1);
    let stats = cache.stats_snapshot();
    assert_eq!(stats.cache_stale_total, 2);
    assert_eq!(stats.perp_ticker_snapshot_served_stale_total, 1);
    assert_eq!(stats.spot_tick_snapshot_served_stale_total, 1);
    Ok(())
}
