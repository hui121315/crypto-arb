#![allow(clippy::panic)]

use super::super::*;

use super::fixtures::*;
use shared_types::VenueMarketSubscription;

#[test]
fn disabling_a_feed_evicts_only_that_venue_from_executable_cache() {
    let cache = MarketDataCache::default();
    let mut binance_perp = ticker("BTC");
    binance_perp.exchange = "binance".to_owned();
    let mut kraken_perp = ticker("BTC");
    kraken_perp.exchange = "kraken".to_owned();
    cache.store_ticker_rows(&[binance_perp, kraken_perp], MarketSource::WsPush);

    let mut binance_spot = spot_tick("BTC/USDT");
    binance_spot.venue = "binance".to_owned();
    let mut kraken_spot = spot_tick("BTC/USD");
    kraken_spot.venue = "kraken".to_owned();
    cache.store_spot_ticks(&[binance_spot, kraken_spot], MarketSource::WsPush);

    cache.apply_market_subscription(&VenueMarketSubscription {
        venue: "binance".to_owned(),
        spot_enabled: false,
        perp_enabled: true,
        funding_enabled: true,
    });

    assert!(!cache
        .spot_ticks
        .iter()
        .any(|entry| entry.key().belongs_to_venue("binance")));
    assert!(cache
        .spot_ticks
        .iter()
        .any(|entry| entry.key().belongs_to_venue("kraken")));
    assert!(cache
        .tickers
        .iter()
        .any(|entry| entry.key().belongs_to_venue("binance")));
}

#[test]
fn fresh_values_ignores_stale_rows() {
    let cache = DashMap::new();
    cache.insert(
        MarketKey::new("binance", "BTCUSDT"),
        CachedEntry::new(ticker("BTCUSDT"), 1_000, MarketSource::RestBaseline),
    );

    assert_eq!(fresh_values(&cache, 2_000, TICKER_FRESH_MS).len(), 1);
    assert!(fresh_values(&cache, 70_000, TICKER_FRESH_MS).is_empty());
}

#[test]
fn selected_spot_bbo_uses_ws_tick_without_opening_depth() {
    let cache = MarketDataCache::default();
    let mut row = spot_tick("PUPS/USD");
    row.venue = "kraken".to_owned();
    row.received_at_ms = 10_000;
    cache.store_spot_ticks(&[row], MarketSource::WsPush);

    let read = cache.spot_bbo_read("kraken", "PUPS/USD", common::time::now_ms(), 10_000);

    assert_eq!(read.quality, MarketQuality::Fresh);
    assert_eq!(read.source, MarketSource::WsPush);
    let book = read.value.expect("fresh BBO");
    assert_eq!(book.best_bid(), Some(100.0));
    assert_eq!(book.best_ask(), Some(101.0));
    assert_eq!(book.bids[0][1], 1.0);
    assert_eq!(book.asks[0][1], 1.0);
    assert!(cache.spot_orderbooks.is_empty());
}

#[test]
fn spot_bbo_cache_keeps_same_base_markets_with_different_quotes_separate() {
    let cache = MarketDataCache::default();
    let mut usd = spot_tick("SOL/USD");
    usd.venue = "kraken".to_owned();
    let mut usdc = spot_tick("SOL/USDC");
    usdc.venue = "kraken".to_owned();
    cache.store_spot_ticks(&[usd, usdc], MarketSource::WsPush);

    assert_eq!(cache.spot_ticks.len(), 2);
    let usd_book = cache
        .spot_bbo_read("kraken", "SOL/USD", common::time::now_ms(), 10_000)
        .value
        .expect("USD BBO");
    let usdc_book = cache
        .spot_bbo_read("kraken", "SOL/USDC", common::time::now_ms(), 10_000)
        .value
        .expect("USDC BBO");
    assert_eq!(usd_book.symbol, "SOL/USD");
    assert_eq!(usdc_book.symbol, "SOL/USDC");
}

#[test]
fn row_evidence_keeps_the_actual_cache_receive_time() {
    let entries = DashMap::new();
    entries.insert(
        MarketKey::new("binance", "BTCUSDT"),
        CachedEntry::new(ticker("BTCUSDT"), 1_000, MarketSource::WsPush),
    );

    let snapshot = fresh_rows_snapshot(
        &entries,
        1_250,
        TICKER_FRESH_MS,
        MarketDataSnapshotOperation::PerpTickers,
        |row| (row.exchange.clone(), row.symbol.clone()),
    );

    assert_eq!(snapshot.row_evidence[0].health.observed_at_ms, 1_000);
    assert_eq!(snapshot.row_evidence[0].health.freshness_ms, Some(250));
}

#[test]
fn cached_snapshot_retains_stale_discovery_rows_without_fresh_evidence() {
    let cache = MarketDataCache::default();
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

    let snapshot = cache.market_snapshot_cached();

    assert_eq!(snapshot.perp_tickers.len(), 1);
    assert_eq!(snapshot.spot_ticks.len(), 1);
    assert_eq!(
        snapshot.perp_ticker_row_evidence[0].health.quality,
        shared_types::MarketDataQuality::StaleAllowed
    );
    assert_eq!(
        snapshot.spot_tick_row_evidence[0].health.quality,
        shared_types::MarketDataQuality::StaleAllowed
    );
}

#[test]
fn cached_snapshot_drops_expired_discovery_rows() {
    let cache = MarketDataCache::default();
    cache.tickers.insert(
        MarketKey::new("mock", "BTC"),
        CachedEntry::new(
            ticker("BTC"),
            common::time::now_ms().saturating_sub(TICKER_DISCOVERY_MAX_AGE_MS + 1),
            MarketSource::RestBaseline,
        ),
    );

    let snapshot = cache.market_snapshot_cached();

    assert!(snapshot.perp_tickers.is_empty());
    assert!(snapshot.perp_ticker_row_evidence.is_empty());
}

#[test]
fn fetch_backoff_wait_preserves_quality_and_expires() {
    let backoffs = DashMap::new();
    let key = MarketKey::new("hyperliquid:xyz", "MU");
    backoffs.insert(
        key.clone(),
        MarketFetchBackoff {
            until_ms: 3_000,
            quality: MarketQuality::Missing,
            last_error: Some("network error: reset".to_owned()),
        },
    );

    let active = fetch_backoff_wait(&backoffs, &key, 1_500)
        .unwrap_or_else(|| panic!("missing active fetch backoff"));
    assert_eq!(active.wait_ms, 1_500);
    assert_eq!(active.quality, MarketQuality::Missing);
    assert_eq!(active.last_error.as_deref(), Some("network error: reset"));
    assert!(fetch_backoff_wait(&backoffs, &key, 3_001).is_none());
}
#[test]
fn funding_rows_snapshot_ignores_stale_rows() {
    let cache = MarketDataCache::default();
    cache.store_funding_rows(&[funding("BTC")], MarketSource::RestBaseline);
    assert_eq!(cache.funding_rows_snapshot().len(), 1);

    let stale_key = MarketKey::new("mock", "ETH");
    cache.funding.insert(
        stale_key,
        CachedEntry::new(
            funding("ETH"),
            common::time::now_ms().saturating_sub(FUNDING_FRESH_MS + 1),
            MarketSource::RestBaseline,
        ),
    );
    assert_eq!(cache.funding_rows_snapshot().len(), 1);
}

#[test]
fn ticker_store_reports_only_economic_changes() {
    let cache = MarketDataCache::default();
    let mut heartbeat = ticker("BTC");
    assert_eq!(
        cache.store_ticker_rows(std::slice::from_ref(&heartbeat), MarketSource::WsPush),
        1
    );
    assert_eq!(
        cache.store_ticker_rows(std::slice::from_ref(&heartbeat), MarketSource::WsPush),
        0
    );

    heartbeat.timestamp += 1;
    assert_eq!(
        cache.store_ticker_rows(std::slice::from_ref(&heartbeat), MarketSource::WsPush),
        0
    );

    let mut changed = heartbeat;
    changed.bid += 1.0;
    changed.timestamp += 1;
    assert_eq!(
        cache.store_ticker_rows(std::slice::from_ref(&changed), MarketSource::WsPush),
        1
    );
}

#[test]
fn funding_store_reports_only_economic_changes() {
    let cache = MarketDataCache::default();
    let first = funding("BTC");
    assert_eq!(
        cache.store_funding_rows(std::slice::from_ref(&first), MarketSource::WsPush),
        1
    );
    assert_eq!(
        cache.store_funding_rows(std::slice::from_ref(&first), MarketSource::WsPush),
        0
    );

    let mut changed = first;
    changed.rate += 0.0001;
    changed.timestamp += 1;
    assert_eq!(
        cache.store_funding_rows(std::slice::from_ref(&changed), MarketSource::WsPush),
        1
    );
}

#[test]
fn unchanged_funding_observation_renews_cache_freshness() {
    let cache = MarketDataCache::default();
    let row = funding("BTC");
    cache.store_funding_rows(std::slice::from_ref(&row), MarketSource::WsPush);

    let key = MarketKey::new("mock", "BTC");
    cache
        .funding
        .get_mut(&key)
        .unwrap_or_else(|| panic!("missing seeded funding row"))
        .received_at_ms = common::time::now_ms().saturating_sub(FUNDING_FRESH_MS + 1);
    assert!(cache.funding_rows_snapshot().is_empty());

    assert_eq!(
        cache.store_funding_rows(std::slice::from_ref(&row), MarketSource::WsPush),
        0
    );
    let refreshed = cache.funding_rows_snapshot_with_evidence();
    assert_eq!(refreshed.rows.len(), 1);
    assert_eq!(refreshed.rows[0].symbol, row.symbol);
    assert!(refreshed.row_evidence[0]
        .health
        .freshness_ms
        .is_some_and(|age_ms| age_ms < FUNDING_FRESH_MS));
}

#[test]
fn fresh_ws_ticker_is_not_downgraded_by_rest_baseline() {
    let cache = MarketDataCache::default();
    let ws = ticker("BTC");
    cache.store_ticker_rows(std::slice::from_ref(&ws), MarketSource::WsPush);

    let mut rest = ws;
    rest.bid += 10.0;
    rest.timestamp += 1;
    assert_eq!(
        cache.store_ticker_rows(std::slice::from_ref(&rest), MarketSource::RestBaseline),
        0
    );

    let snapshot = cache.market_snapshot_cached();
    assert_eq!(snapshot.perp_tickers[0].bid, 1.0);
    assert_eq!(
        snapshot.perp_ticker_row_evidence[0].health.source,
        shared_types::MarketDataSourceKind::WsPush
    );
}

#[test]
fn hyperliquid_builder_rest_and_ws_symbols_share_one_ticker_row() {
    let cache = MarketDataCache::default();
    let mut rest = ticker("ZHIPU");
    rest.exchange = "hyperliquid:xyz".to_owned();
    cache.store_ticker_rows(std::slice::from_ref(&rest), MarketSource::RestBaseline);

    let mut ws = rest;
    ws.symbol = "xyz:ZHIPU".to_owned();
    ws.bid += 1.0;
    ws.timestamp += 1;
    cache.store_ticker_rows(std::slice::from_ref(&ws), MarketSource::WsPush);

    let snapshot = cache.market_snapshot_cached();
    assert_eq!(snapshot.perp_tickers.len(), 1);
    assert_eq!(snapshot.perp_tickers[0].symbol, "xyz:ZHIPU");
    assert_eq!(snapshot.perp_ticker_row_evidence.len(), 1);
    assert_eq!(
        snapshot.perp_ticker_row_evidence[0].health.source,
        shared_types::MarketDataSourceKind::WsPush
    );
}
#[test]
fn funding_rows_snapshot_includes_row_evidence() {
    let cache = MarketDataCache::default();
    cache.store_funding_rows(&[funding("BTC")], MarketSource::RestBaseline);

    let snapshot = cache.funding_rows_snapshot_with_evidence();

    assert_eq!(snapshot.rows.len(), 1);
    assert_eq!(snapshot.row_evidence.len(), 1);
    assert_eq!(snapshot.row_evidence[0].venue, "mock");
    assert_eq!(snapshot.row_evidence[0].symbol, "BTC");
    assert_eq!(
        snapshot.row_evidence[0].operation,
        MarketDataSnapshotOperation::FundingRates
    );
    assert_eq!(
        snapshot.row_evidence[0].health.source,
        shared_types::MarketDataSourceKind::RestBaseline
    );
    assert_eq!(
        snapshot.row_evidence[0].health.coverage,
        Some(coverage(1, 1))
    );
}
#[test]
fn market_snapshot_cached_carries_funding_row_evidence() {
    let cache = MarketDataCache::default();
    cache.store_funding_rows(&[funding("BTC")], MarketSource::WsPush);

    let snapshot = cache.market_snapshot_cached();

    assert_eq!(snapshot.funding_row_evidence.len(), 1);
    assert_eq!(snapshot.funding_row_evidence[0].venue, "mock");
    assert_eq!(
        snapshot.funding_row_evidence[0].operation,
        MarketDataSnapshotOperation::FundingRates
    );
    assert_eq!(
        snapshot.funding_row_evidence[0].health.source,
        shared_types::MarketDataSourceKind::WsPush
    );
}
mod evidence;
