#![allow(clippy::panic)]

use super::super::*;

use super::fixtures::*;

#[test]
fn snapshot_status_exposes_depth_only_after_an_on_demand_book_exists() {
    let cache = MarketDataCache::default();
    let before = cache.snapshot_status(common::time::now_ms());

    assert!(!before
        .rows
        .iter()
        .any(|row| row.operation == MarketDataSnapshotOperation::Orderbooks));

    cache.store_orderbook(orderbook("BTC"), MarketSource::WsPush);
    let after = cache.snapshot_status(common::time::now_ms());
    let row = find_status_row(&after.rows, MarketDataSnapshotOperation::Orderbooks);
    assert_eq!(row.health.quality, shared_types::MarketDataQuality::Fresh);
    assert_eq!(
        row.health
            .coverage
            .as_ref()
            .map(|coverage| (coverage.requested, coverage.received)),
        Some((1, 1))
    );
}

#[test]
fn snapshot_status_prefers_fresh_ws_over_failed_rest_baseline() {
    let cache = MarketDataCache::default();
    cache.record_runtime_success(
        "kraken",
        MARKET_OP_REST_SPOT_TICKS,
        MarketSource::RestBaseline,
        1,
        0,
    );
    cache.record_runtime_success(
        "kraken",
        MARKET_OP_WS_SPOT_SNAPSHOT,
        MarketSource::WsPush,
        1,
        1,
    );

    let status = cache.snapshot_status(common::time::now_ms());

    assert!(!status.rows.iter().any(|row| {
        row.venue == "kraken"
            && row.operation == MarketDataSnapshotOperation::SpotTicks
            && row.health.quality != shared_types::MarketDataQuality::Fresh
    }));
    assert!(status.rows.iter().any(|row| {
        row.venue == "kraken"
            && row.operation == MarketDataSnapshotOperation::WsSpotTicks
            && row.health.quality == shared_types::MarketDataQuality::Fresh
    }));
}
#[test]
fn snapshot_status_reports_core_cache_health() {
    let cache = MarketDataCache::default();
    let now_ms = common::time::now_ms();
    cache.store_funding_rows(&[funding("BTC")], MarketSource::RestBaseline);

    let status = cache.snapshot_status(now_ms);
    let funding = find_status_row(&status.rows, MarketDataSnapshotOperation::FundingRates);
    let perp = find_status_row(&status.rows, MarketDataSnapshotOperation::PerpTickers);
    let spot = find_status_row(&status.rows, MarketDataSnapshotOperation::SpotTicks);

    assert_eq!(
        funding.health.quality,
        shared_types::MarketDataQuality::Fresh
    );
    assert_eq!(
        funding
            .health
            .coverage
            .as_ref()
            .map(|coverage| coverage.received),
        Some(1)
    );
    assert_eq!(
        perp.health.quality,
        shared_types::MarketDataQuality::Missing
    );
    assert_eq!(
        spot.health.quality,
        shared_types::MarketDataQuality::Missing
    );
    assert_eq!(
        spot.health
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("MARKET_DATA_MISSING")
    );
}
#[test]
fn snapshot_status_preserves_runtime_rate_limit() {
    let cache = MarketDataCache::default();
    cache.record_runtime_error(
        "bybit",
        MARKET_OP_REST_PERP_TICKERS,
        MarketSource::RestBaseline,
        1,
        &exchange::ExchangeError::RateLimited {
            retry_after_secs: 2,
        },
    );

    let status = cache.snapshot_status(common::time::now_ms());
    let row = status
        .rows
        .iter()
        .find(|row| {
            row.venue == "bybit" && row.operation == MarketDataSnapshotOperation::PerpTickers
        })
        .unwrap_or_else(|| panic!("missing bybit perp ticker status"));

    assert_eq!(
        row.health.quality,
        shared_types::MarketDataQuality::RateLimited
    );
    assert_eq!(row.health.retry_after_ms, Some(2_000));
}

mod runtime;
mod status;
