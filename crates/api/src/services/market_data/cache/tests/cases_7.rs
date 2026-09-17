use super::super::*;
use exchange::{Aggregator, ExchangeAdapter};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use super::fixtures::{
    find_cache_access, funding, index_composition, orderbook, spot_tick, ticker, OrderbookAdapter,
};

#[test]
fn position_mark_cache_accepts_only_fresh_ws_rows() {
    let cache = MarketDataCache::default();
    let now_ms = common::time::now_ms();
    let mut row = MarkIndexInfo {
        symbol: "SOL".to_owned(),
        exchange: "binance".to_owned(),
        mark_price: 150.0,
        index_price: Some(149.9),
        open_interest: None,
        open_interest_value: None,
        timestamp: now_ms,
    };

    assert_eq!(
        cache.store_mark_index_rows(std::slice::from_ref(&row), MarketSource::WsPush),
        1
    );
    assert_eq!(
        cache
            .fresh_ws_mark_index(" Binance ", "SOLUSDT", now_ms)
            .map(|(row, _)| row.mark_price),
        Some(150.0)
    );

    row.mark_price = 151.0;
    row.timestamp += 1;
    assert_eq!(
        cache.store_mark_index_rows(std::slice::from_ref(&row), MarketSource::RestBaseline),
        0
    );
    assert_eq!(
        cache
            .fresh_ws_mark_index("binance", "SOL", now_ms)
            .map(|(row, _)| row.mark_price),
        Some(150.0)
    );
}

#[test]
fn market_projection_reuses_unchanged_feeds_and_invalidates_only_the_written_feed() {
    let cache = MarketDataCache::default();
    cache.store_funding_rows(&[funding("BTC")], MarketSource::WsPush);
    cache.store_ticker_rows(&[ticker("BTC")], MarketSource::WsPush);
    cache.store_spot_ticks(&[spot_tick("BTC/USDT")], MarketSource::WsPush);
    cache.store_index_compositions(&[index_composition("BTC")], MarketSource::RestBaseline);

    let first = cache.market_snapshot_cached();
    let unchanged = cache.market_snapshot_cached();

    assert!(Arc::ptr_eq(&first.funding, &unchanged.funding));
    assert!(Arc::ptr_eq(&first.perp_tickers, &unchanged.perp_tickers));
    assert!(Arc::ptr_eq(&first.spot_ticks, &unchanged.spot_ticks));
    assert!(Arc::ptr_eq(
        &first.index_compositions,
        &unchanged.index_compositions
    ));

    let mut changed = ticker("BTC");
    changed.bid = 2.0;
    changed.timestamp = 2_000;
    assert_eq!(cache.store_ticker_rows(&[changed], MarketSource::WsPush), 1);
    let updated = cache.market_snapshot_cached();

    assert!(!Arc::ptr_eq(&first.perp_tickers, &updated.perp_tickers));
    assert!(Arc::ptr_eq(&first.funding, &updated.funding));
    assert!(Arc::ptr_eq(&first.spot_ticks, &updated.spot_ticks));
    assert!(Arc::ptr_eq(
        &first.index_compositions,
        &updated.index_compositions
    ));
    assert_eq!(updated.perp_tickers[0].bid, 2.0);
}

#[test]
fn market_projection_tracks_freshness_heartbeats_without_triggering_an_economic_scan() {
    let cache = MarketDataCache::default();
    let initial = ticker("BTC");
    cache.store_ticker_rows(std::slice::from_ref(&initial), MarketSource::WsPush);
    let first = cache.market_snapshot_cached();

    let mut heartbeat = initial;
    heartbeat.timestamp = 2_000;
    assert_eq!(
        cache.store_ticker_rows(&[heartbeat], MarketSource::WsPush),
        0
    );
    let refreshed = cache.market_snapshot_cached();

    assert!(!Arc::ptr_eq(&first.perp_tickers, &refreshed.perp_tickers));
    assert_eq!(refreshed.perp_tickers[0].timestamp, 2_000);
}

#[test]
fn opportunity_snapshot_excludes_on_demand_execution_depth_health() {
    let cache = MarketDataCache::default();
    cache.store_orderbook(orderbook("BTC"), MarketSource::WsPush);

    assert!(cache
        .snapshot_status(common::time::now_ms())
        .rows
        .iter()
        .any(|row| row.operation == MarketDataSnapshotOperation::Orderbooks));

    let opportunity_status = cache.market_snapshot_cached().status.unwrap_or_default();
    assert!(!opportunity_status
        .rows
        .iter()
        .any(|row| row.operation == MarketDataSnapshotOperation::Orderbooks));
}

#[tokio::test]
async fn execution_orderbooks_wait_for_bounded_cold_ws_startup() {
    let cache = MarketDataCache::default();
    let aggregator = Aggregator::new();
    let adapter = Arc::new(OrderbookAdapter {
        perp_ws_ready_after_calls: 3,
        spot_ws_ready_after_calls: 4,
        ..OrderbookAdapter::default()
    });
    let cloned: Arc<OrderbookAdapter> = Arc::clone(&adapter);
    let registered: Arc<dyn ExchangeAdapter> = cloned;
    aggregator.register(registered);

    let (perp, spot) = tokio::join!(
        cache.refresh_orderbook_from_ws(&aggregator, "mock", "BTC", 20, common::time::now_ms()),
        cache.refresh_spot_orderbook_from_ws(
            &aggregator,
            "mock",
            "BTC/USDT",
            20,
            common::time::now_ms()
        )
    );

    assert_eq!(perp.source, MarketSource::WsPush);
    assert_eq!(spot.source, MarketSource::WsPush);
    assert_eq!(adapter.perp_ws_calls.load(Ordering::SeqCst), 3);
    assert_eq!(adapter.spot_ws_calls.load(Ordering::SeqCst), 4);
    assert_eq!(adapter.orderbook_calls.load(Ordering::SeqCst), 0);
    assert_eq!(adapter.spot_orderbook_calls.load(Ordering::SeqCst), 0);
    let access = cache.cache_access_metrics_snapshot();
    assert_eq!(
        find_cache_access(
            &access,
            FEED_ORDERBOOK,
            CACHE_OUTCOME_REFRESH,
            MarketSource::WsPush,
            MarketQuality::Fresh
        )
        .count,
        1
    );
    assert_eq!(
        find_cache_access(
            &access,
            FEED_SPOT_ORDERBOOK,
            CACHE_OUTCOME_REFRESH,
            MarketSource::WsPush,
            MarketQuality::Fresh
        )
        .count,
        1
    );
}
