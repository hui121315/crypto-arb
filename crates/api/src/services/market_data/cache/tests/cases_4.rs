#![allow(clippy::panic)]

use super::super::*;
use exchange::{Aggregator, ExchangeAdapter};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::fixtures::*;

#[tokio::test]
async fn orderbook_fetch_is_singleflight_per_market_key() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter {
        perp_ws_ready: true,
        ..OrderbookAdapter::default()
    });
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);

    let (first, second) = tokio::join!(
        cache.refresh_orderbook_from_ws(&aggregator, "mock", "BTC", 20, 1_000),
        cache.refresh_orderbook_from_ws(&aggregator, "mock", "BTC", 20, 1_000)
    );

    assert_eq!(adapter.perp_ws_calls.load(Ordering::SeqCst), 1);
    assert_eq!(adapter.orderbook_calls.load(Ordering::SeqCst), 0);
    assert_eq!(first.quality, MarketQuality::Fresh);
    assert_eq!(second.quality, MarketQuality::Fresh);
}

#[test]
fn index_compositions_snapshot_ignores_stale_rows() {
    let cache = MarketDataCache::default();
    cache.store_index_compositions(&[index_composition("MU")], MarketSource::RestBaseline);
    assert_eq!(cache.index_compositions_snapshot().len(), 1);

    cache.index_compositions.insert(
        MarketKey::new("mock", "OLD"),
        CachedEntry::new(
            index_composition("OLD"),
            common::time::now_ms().saturating_sub(INDEX_COMPOSITION_FRESH_MS + 1),
            MarketSource::RestBaseline,
        ),
    );
    assert_eq!(cache.index_compositions_snapshot().len(), 1);
}

#[test]
fn snapshot_status_fresh_cache_replaces_stale_aggregate_missing() {
    let cache = MarketDataCache::default();
    cache.record_runtime_success(
        MARKET_AGGREGATE_VENUE,
        MARKET_OP_REST_PERP_TICKERS,
        MarketSource::RestBaseline,
        8,
        0,
    );
    cache.store_ticker_rows(&[ticker("BTC")], MarketSource::WsPush);

    let status = cache.snapshot_status(common::time::now_ms());
    let row = find_status_row(&status.rows, MarketDataSnapshotOperation::PerpTickers);

    assert_eq!(row.health.quality, shared_types::MarketDataQuality::Fresh);
    assert_eq!(
        row.health.source,
        shared_types::MarketDataSourceKind::WsPush
    );
    assert_eq!(
        row.health
            .coverage
            .as_ref()
            .map(|coverage| coverage.received),
        Some(1)
    );
}
#[test]
fn runtime_health_records_fanout_venue_outcomes() {
    let cache = MarketDataCache::default();
    let outcomes = vec![
        exchange::FanoutVenueResult {
            venue: "binance".to_owned(),
            operation: MARKET_OP_REST_FUNDING_RATES,
            rows: 4,
            latency_ms: 12,
            error: None,
            problem: None,
        },
        exchange::FanoutVenueResult {
            venue: "okx".to_owned(),
            operation: MARKET_OP_REST_FUNDING_RATES,
            rows: 0,
            latency_ms: 8,
            error: None,
            problem: None,
        },
        exchange::FanoutVenueResult {
            venue: "bybit".to_owned(),
            operation: MARKET_OP_REST_FUNDING_RATES,
            rows: 0,
            latency_ms: 5,
            error: Some(exchange::ExchangeError::RateLimited {
                retry_after_secs: 3,
            }),
            problem: Some(
                exchange::ExchangeError::RateLimited {
                    retry_after_secs: 3,
                }
                .to_problem("bybit", MARKET_OP_REST_FUNDING_RATES),
            ),
        },
    ];
    cache.record_aggregate_fanout_outcome(
        MARKET_OP_REST_FUNDING_RATES,
        MarketSource::RestBaseline,
        &outcomes,
    );
    cache.record_fanout_outcomes(MarketSource::RestBaseline, outcomes);

    let rows = cache.runtime_health_snapshot();

    let aggregate =
        find_runtime_health(&rows, MARKET_AGGREGATE_VENUE, MARKET_OP_REST_FUNDING_RATES);
    assert_eq!(aggregate.quality, MarketQuality::RateLimited);
    assert_eq!(aggregate.requested, 3);
    assert_eq!(aggregate.rows, 1);
    assert_eq!(aggregate.retry_after_ms, Some(3_000));
    assert_eq!(
        aggregate
            .problem
            .as_ref()
            .map(|problem| (problem.venue.as_str(), problem.operation.as_str())),
        Some(("bybit", MARKET_OP_REST_FUNDING_RATES))
    );

    let binance = find_runtime_health(&rows, "binance", MARKET_OP_REST_FUNDING_RATES);
    assert_eq!(binance.quality, MarketQuality::Fresh);
    assert_eq!(binance.requested, 1);
    assert_eq!(binance.rows, 4);

    let okx = find_runtime_health(&rows, "okx", MARKET_OP_REST_FUNDING_RATES);
    assert_eq!(okx.quality, MarketQuality::Missing);
    assert_eq!(okx.rows, 0);
    assert!(okx
        .last_error
        .as_deref()
        .is_some_and(|msg| msg.contains("no rows")));

    let bybit = find_runtime_health(&rows, "bybit", MARKET_OP_REST_FUNDING_RATES);
    assert_eq!(bybit.quality, MarketQuality::RateLimited);
    assert_eq!(bybit.retry_after_ms, Some(3_000));
    assert_fanout_problem_context(bybit);
}

fn assert_fanout_problem_context(health: &MarketRuntimeHealth) {
    assert_eq!(
        health
            .problem
            .as_ref()
            .map(|problem| (problem.venue.as_str(), problem.operation.as_str())),
        Some(("bybit", MARKET_OP_REST_FUNDING_RATES))
    );
    assert_eq!(
        health
            .problem
            .as_ref()
            .and_then(|problem| problem.latency_ms),
        Some(5)
    );
}

#[test]
fn aggregate_empty_fanout_is_missing_not_fresh() {
    let health = aggregate_fanout_health(
        MARKET_OP_REST_FUNDING_RATES,
        MarketSource::RestColdStart,
        &[],
    );

    assert_eq!(health.quality, MarketQuality::Missing);
    assert_eq!(health.requested, 0);
    assert_eq!(health.rows, 0);
    assert!(health
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("no registered venues")));
}
#[tokio::test]
async fn index_composition_fetch_is_cached_per_market_key() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter::default());
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);

    let first = cache
        .index_composition_or_fetch(&aggregator, "mock", "MU", common::time::now_ms())
        .await;
    let second = cache
        .index_composition_or_fetch(&aggregator, "mock", "MU", common::time::now_ms())
        .await;

    assert_eq!(adapter.index_composition_calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.quality, MarketQuality::Fresh);
    assert_eq!(second.quality, MarketQuality::Fresh);
    assert_eq!(
        first.value.as_ref().map(|row| row.symbol.as_str()),
        Some("MU")
    );
}

#[tokio::test]
async fn index_composition_rate_limit_uses_venue_wide_negative_cache() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter {
        fail_index_composition: true,
        ..Default::default()
    });
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);

    let first = cache
        .index_composition_or_fetch(&aggregator, "mock", "MU", common::time::now_ms())
        .await;
    let second = cache
        .index_composition_or_fetch(&aggregator, "mock", "BTC", common::time::now_ms())
        .await;

    assert_eq!(adapter.index_composition_calls.load(Ordering::SeqCst), 1);
    assert_eq!(first.quality, MarketQuality::RateLimited);
    assert_eq!(second.quality, MarketQuality::RateLimited);
    assert!(second.retry_after_ms.is_some_and(|wait| wait > 50_000));
}

#[test]
fn fee_schedule_evidence_surfaces_per_venue_snapshot_status_rows() {
    let cache = MarketDataCache::default();
    let now_ms = common::time::now_ms();

    let status = cache.snapshot_status(now_ms);
    let fee_rows: Vec<_> = status
        .rows
        .iter()
        .filter(|row| row.operation == MarketDataSnapshotOperation::FeeSchedule)
        .collect();
    assert_eq!(fee_rows.len(), 8, "one fee schedule row per venue family");

    for row in &fee_rows {
        assert_eq!(row.health.quality, shared_types::MarketDataQuality::Fresh);
        assert_eq!(
            row.health.source,
            shared_types::MarketDataSourceKind::LocalCache
        );
        assert!(row.health.freshness_ms.is_some_and(|value| value >= 0));
        let coverage = row
            .health
            .coverage
            .as_ref()
            .unwrap_or_else(|| panic!("missing fee coverage for {}", row.venue));
        assert_eq!(coverage.requested, 2, "{}", row.venue);
        assert_eq!(coverage.received, 2, "{}", row.venue);
        assert!(row.health.last_error.is_none(), "{}", row.venue);
    }
}
