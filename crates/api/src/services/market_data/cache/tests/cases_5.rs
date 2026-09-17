#![allow(clippy::panic)]

use super::super::*;
use exchange::{Aggregator, ExchangeAdapter};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::fixtures::*;

#[tokio::test]
async fn cache_stats_track_miss_and_fresh_hit() {
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
    let stats = cache.stats_snapshot();

    assert_eq!(first.quality, MarketQuality::Fresh);
    assert_eq!(second.quality, MarketQuality::Fresh);
    assert_eq!(stats.cache_miss_total, 1);
    assert_eq!(stats.cache_hit_total, 1);
    assert_eq!(stats.cache_stale_total, 0);
    assert!((stats.cache_hit_ratio - 0.5).abs() < f64::EPSILON);
    let access = cache.cache_access_metrics_snapshot();
    assert_eq!(
        find_cache_access(
            &access,
            FEED_ORDERBOOK,
            CACHE_OUTCOME_MISS,
            MarketSource::LocalCache,
            MarketQuality::Missing
        )
        .count,
        1
    );
    assert_eq!(
        find_cache_access(
            &access,
            FEED_ORDERBOOK,
            CACHE_OUTCOME_HIT,
            MarketSource::WsPush,
            MarketQuality::Fresh
        )
        .count,
        1
    );
}

#[tokio::test]
async fn high_frequency_spot_projection_bypasses_fresh_rest_cache_for_ws() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter {
        spot_ws_ready: true,
        ..OrderbookAdapter::default()
    });
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);
    cache.spot_orderbooks.insert(
        MarketKey::spot("mock", "BTC/USDT"),
        CachedEntry::new(
            orderbook("BTC/USDT"),
            common::time::now_ms(),
            MarketSource::RestBaseline,
        ),
    );

    let read = cache
        .refresh_spot_orderbook_from_ws(&aggregator, "mock", "BTC/USDT", 20, common::time::now_ms())
        .await;

    assert_eq!(read.source, MarketSource::WsPush);
    assert_eq!(adapter.spot_orderbook_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn non_rate_limit_orderbook_error_uses_short_negative_cache() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter {
        orderbook_error: Some(OrderbookFailure::Network),
        ..OrderbookAdapter::default()
    });
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);
    let now_ms = common::time::now_ms();

    let first = cache
        .refresh_orderbook_from_ws(&aggregator, "mock", "BTC", 20, now_ms)
        .await;
    let second = cache
        .refresh_orderbook_from_ws(&aggregator, "mock", "BTC", 20, common::time::now_ms())
        .await;

    assert_eq!(first.quality, MarketQuality::Missing);
    assert_eq!(second.quality, MarketQuality::Missing);
    assert!(second
        .retry_after_ms
        .is_some_and(|wait_ms| wait_ms > 0 && wait_ms <= ORDERBOOK_NEGATIVE_CACHE_MS));
    assert!(second
        .last_error
        .as_deref()
        .is_some_and(|msg| msg.contains("network error")));
    assert_eq!(adapter.perp_ws_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.orderbook_calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn unsupported_orderbook_symbol_uses_long_negative_cache() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter {
        orderbook_error: Some(OrderbookFailure::UnsupportedSymbol),
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

    assert_eq!(first.quality, MarketQuality::Unsupported);
    assert_eq!(second.quality, MarketQuality::Unsupported);
    assert!(second.retry_after_ms.is_some_and(|wait_ms| {
        wait_ms > ORDERBOOK_NEGATIVE_CACHE_MS && wait_ms <= ORDERBOOK_UNSUPPORTED_CACHE_MS
    }));
    assert_eq!(adapter.perp_ws_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.orderbook_calls.load(Ordering::SeqCst), 0);
}
