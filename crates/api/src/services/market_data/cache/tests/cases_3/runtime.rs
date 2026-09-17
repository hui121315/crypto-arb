#![allow(clippy::panic)]

use super::*;

#[test]
fn runtime_health_records_errors() {
    let cache = MarketDataCache::default();
    cache.record_runtime_error(
        "bybit",
        MARKET_OP_WS_TICKER,
        MarketSource::WsPush,
        2,
        &exchange::ExchangeError::RateLimited {
            retry_after_secs: 3,
        },
    );

    let rows = cache.runtime_health_snapshot();

    let ticker = find_runtime_health(&rows, "bybit", MARKET_OP_WS_TICKER);
    assert_eq!(ticker.quality, MarketQuality::RateLimited);
    assert_eq!(ticker.retry_after_ms, Some(3_000));
    assert!(ticker
        .last_error
        .as_deref()
        .is_some_and(|msg| msg.contains("rate limited")));
}

#[test]
fn runtime_health_snapshot_degrades_old_fresh_market_sample() {
    let cache = MarketDataCache::default();
    let now_ms = common::time::now_ms();
    cache.upsert_runtime_health(MarketRuntimeHealth {
        venue: "binance".to_owned(),
        operation: MARKET_OP_WS_TICKER,
        quality: MarketQuality::Fresh,
        source: MarketSource::WsPush,
        requested: 1,
        rows: 1,
        retry_after_ms: None,
        last_error: None,
        problem: None,
        observed_at_ms: now_ms - TICKER_FRESH_MS - 1,
    });

    let rows = cache.runtime_health_snapshot();
    let health = find_runtime_health(&rows, "binance", MARKET_OP_WS_TICKER);

    assert_eq!(health.quality, MarketQuality::StaleAllowed);
    assert!(health
        .last_error
        .as_deref()
        .is_some_and(|message| message.contains("market runtime sample stale")));
}

#[test]
fn fresh_ws_feed_is_not_overwritten_by_rest_baseline_miss() {
    let cache = MarketDataCache::default();
    cache.record_runtime_success(
        "gate_crossex",
        MARKET_OP_PERP_TICKERS,
        MarketSource::WsPush,
        1,
        3,
    );
    cache.record_runtime_success(
        "gate_crossex",
        MARKET_OP_PERP_TICKERS,
        MarketSource::RestBaseline,
        1,
        0,
    );

    let rows = cache.runtime_health_snapshot();
    let health = find_runtime_health(&rows, "gate_crossex", MARKET_OP_PERP_TICKERS);

    assert_eq!(health.quality, MarketQuality::Fresh);
    assert_eq!(health.source, MarketSource::WsPush);
    assert_eq!(health.rows, 3);
}

#[test]
fn runtime_health_preserves_metadata_sample_beyond_default_ttl() {
    let cache = MarketDataCache::default();
    let now_ms = common::time::now_ms();
    cache.upsert_runtime_health(MarketRuntimeHealth {
        venue: "binance".to_owned(),
        operation: MARKET_OP_REST_METADATA,
        quality: MarketQuality::Fresh,
        source: MarketSource::RestColdStart,
        requested: 1,
        rows: 1,
        retry_after_ms: None,
        last_error: None,
        problem: None,
        observed_at_ms: now_ms.saturating_sub(MARKET_RUNTIME_HEALTH_TTL_MS + 1),
    });

    let rows = cache.runtime_health_snapshot();
    let health = find_runtime_health(&rows, "binance", MARKET_OP_REST_METADATA);

    assert_eq!(health.quality, MarketQuality::Fresh);
    assert_eq!(health.rows, 1);
}

#[test]
fn metadata_failure_surfaces_per_venue_snapshot_status_row() {
    let cache = MarketDataCache::default();
    let now_ms = common::time::now_ms();
    cache.record_runtime_error(
        "gate",
        MARKET_OP_REST_METADATA,
        MarketSource::RestColdStart,
        1,
        &exchange::error::ExchangeError::Timeout { seconds: 10 },
    );

    let status = cache.snapshot_status(now_ms);
    let row = status
        .rows
        .iter()
        .find(|row| row.venue == "gate" && row.operation == MarketDataSnapshotOperation::Metadata)
        .unwrap_or_else(|| panic!("missing per-venue metadata status row"));

    assert_eq!(row.health.quality, shared_types::MarketDataQuality::Missing);
    assert_eq!(
        row.health.source,
        shared_types::MarketDataSourceKind::RestColdStart
    );
    assert!(row
        .health
        .last_error
        .as_deref()
        .is_some_and(|message| message.contains("timeout after 10s")));
    assert!(row.health.problem.is_some());
}

#[test]
fn runtime_success_with_requested_empty_rows_is_missing() {
    let cache = MarketDataCache::default();
    cache.record_runtime_success(
        "hyperliquid:xyz",
        MARKET_OP_WS_TICKER,
        MarketSource::WsPush,
        1,
        0,
    );

    let rows = cache.runtime_health_snapshot();
    let health = find_runtime_health(&rows, "hyperliquid:xyz", MARKET_OP_WS_TICKER);

    assert_eq!(health.quality, MarketQuality::Missing);
    assert_eq!(health.requested, 1);
    assert_eq!(health.rows, 0);
    assert!(health
        .last_error
        .as_deref()
        .is_some_and(|msg| msg.contains("produced partial rows: 0/1")));
}

#[test]
fn runtime_success_with_partial_rows_is_missing() {
    let cache = MarketDataCache::default();
    cache.record_runtime_success("bybit", MARKET_OP_WS_TICKER, MarketSource::WsPush, 2, 1);

    let rows = cache.runtime_health_snapshot();
    let health = find_runtime_health(&rows, "bybit", MARKET_OP_WS_TICKER);

    assert_eq!(health.quality, MarketQuality::Missing);
    assert_eq!(health.requested, 2);
    assert_eq!(health.rows, 1);
    assert!(health
        .last_error
        .as_deref()
        .is_some_and(|msg| msg.contains("produced partial rows: 1/2")));
}
