use super::metadata::record_metadata_outcomes;
use super::ws_touch::ingest_ws_market_updates;
use super::*;
use crate::services::market_data::cache::{TICKER_DISCOVERY_MAX_AGE_MS, TICKER_FRESH_MS};
use crate::services::market_data::MarketQuality;

#[path = "market_data_tests/baseline.rs"]
mod baseline;
#[path = "market_data_tests/recovery.rs"]
mod recovery;
#[path = "market_data_tests/support.rs"]
mod support;
#[path = "market_data_tests/watchlist_runtime.rs"]
mod watchlist_runtime;
#[path = "market_data_tests/ws_ingest.rs"]
mod ws_ingest;

use support::*;

#[tokio::test]
async fn empty_ws_ready_stays_warming_without_rest_fallback() {
    let runtime = test_runtime();
    runtime.aggregator.register(Arc::new(TouchAdapter::empty()));
    let requests = BTreeMap::from([("bybit".to_owned(), vec!["MU".to_owned()])]);

    let stats = ingest_ws_market_updates(
        &runtime,
        &requests,
        &requests,
        &requests,
        &BTreeMap::new(),
        true,
    )
    .await;

    assert_eq!(stats.changed_rows, 0);
    let rows = runtime.market_data.runtime_health_snapshot();
    for operation in [
        MARKET_OP_WS_TICKER_SUBSCRIBE,
        MARKET_OP_WS_TICKER_SNAPSHOT,
        MARKET_OP_WS_FUNDING_SUBSCRIBE,
        MARKET_OP_WS_FUNDING_SNAPSHOT,
    ] {
        assert!(rows.iter().any(|row| {
            row.venue == "bybit"
                && row.operation == operation
                && row.quality == MarketQuality::Warming
                && row.source == MarketSource::WsPush
                && row.rows == 0
                && row.retry_after_ms == Some(10_000)
                && row.problem.is_none()
        }));
    }
    assert!(!rows.iter().any(|row| matches!(
        row.operation,
        MARKET_OP_REST_TICKER_FALLBACK
            | MARKET_OP_REST_FUNDING_FALLBACK
            | MARKET_OP_REST_SPOT_TICKS
    )));
}

#[tokio::test]
async fn unsupported_ws_remains_explicit_without_rest_fallback() {
    let runtime = test_runtime();
    runtime
        .aggregator
        .register(Arc::new(UnsupportedTouchAdapter));
    let requests = BTreeMap::from([("rest-only".to_owned(), vec!["MU".to_owned()])]);

    let stats = ingest_ws_market_updates(
        &runtime,
        &requests,
        &requests,
        &requests,
        &BTreeMap::new(),
        true,
    )
    .await;

    assert_eq!(stats.changed_rows, 0);
    let rows = runtime.market_data.runtime_health_snapshot();
    for operation in [
        MARKET_OP_WS_TICKER_SUBSCRIBE,
        MARKET_OP_WS_TICKER_SNAPSHOT,
        MARKET_OP_WS_FUNDING_SUBSCRIBE,
        MARKET_OP_WS_FUNDING_SNAPSHOT,
    ] {
        assert!(rows.iter().any(|row| {
            row.venue == "rest-only"
                && row.operation == operation
                && row.quality == MarketQuality::Unsupported
                && row.rows == 0
        }));
    }
    assert!(!rows.iter().any(|row| matches!(
        row.operation,
        MARKET_OP_REST_TICKER_FALLBACK
            | MARKET_OP_REST_FUNDING_FALLBACK
            | MARKET_OP_REST_SPOT_TICKS
    )));
    let snapshot = runtime.market_data.market_snapshot_cached();
    assert!(snapshot.perp_tickers.is_empty());
    assert_eq!(
        snapshot
            .funding
            .values()
            .map(|rows| rows.len())
            .sum::<usize>(),
        0
    );
}

#[tokio::test]
async fn live_ws_ingest_reports_changes_without_rest_fallback() {
    let runtime = test_runtime();
    let adapter: Arc<dyn exchange::ExchangeAdapter> = Arc::new(TouchAdapter::ready());
    runtime.aggregator.register(adapter);
    let requests = BTreeMap::from([("bybit".to_owned(), vec!["MU".to_owned()])]);
    let spot_requests = requests.clone();

    let first = ingest_ws_market_updates(
        &runtime,
        &requests,
        &spot_requests,
        &requests,
        &BTreeMap::new(),
        true,
    )
    .await;
    let second = ingest_ws_market_updates(
        &runtime,
        &requests,
        &spot_requests,
        &requests,
        &BTreeMap::new(),
        true,
    )
    .await;

    assert_eq!(first.requested, 3);
    assert_eq!(first.rows, 3);
    assert_eq!(first.changed_rows, 3);
    assert_eq!(second.changed_rows, 0);
    assert_eq!(
        runtime
            .market_data
            .market_snapshot_cached()
            .perp_tickers
            .len(),
        1
    );
    assert!(runtime
        .market_data
        .runtime_health_snapshot()
        .iter()
        .any(|row| row.operation == MARKET_OP_WS_SPOT_SNAPSHOT));
}

#[tokio::test]
async fn live_ws_ingest_never_falls_back_to_rest() {
    let runtime = test_runtime();
    let adapter: Arc<dyn exchange::ExchangeAdapter> = Arc::new(UnsupportedTouchAdapter);
    runtime.aggregator.register(adapter);
    let requests = BTreeMap::from([("rest-only".to_owned(), vec!["MU".to_owned()])]);
    let spot_requests = requests.clone();

    let stats = ingest_ws_market_updates(
        &runtime,
        &requests,
        &spot_requests,
        &requests,
        &BTreeMap::new(),
        true,
    )
    .await;

    assert_eq!(stats.changed_rows, 0);
    assert!(runtime
        .market_data
        .market_snapshot_cached()
        .perp_tickers
        .is_empty());
    let health = runtime.market_data.runtime_health_snapshot();
    assert!(!health.iter().any(|row| matches!(
        row.operation,
        MARKET_OP_REST_TICKER_FALLBACK
            | MARKET_OP_REST_FUNDING_FALLBACK
            | MARKET_OP_REST_SPOT_TICKS
    )));
}

#[test]
fn metadata_prewarm_records_market_runtime_health() {
    let runtime = test_runtime();
    let (refreshed, skipped, failed) = record_metadata_outcomes(
        &runtime,
        vec![
            (
                "binance".to_owned(),
                Ok(exchange::MetadataRefreshOutcome::Refreshed),
            ),
            (
                "bybit".to_owned(),
                Ok(exchange::MetadataRefreshOutcome::NotRequired),
            ),
            (
                "okx".to_owned(),
                Err(exchange::ExchangeError::RateLimited {
                    retry_after_secs: 2,
                }),
            ),
        ],
        MarketSource::RestColdStart,
    );

    assert_eq!((refreshed, skipped, failed), (1, 1, 1));
    let rows = runtime.market_data.runtime_health_snapshot();
    assert!(rows.iter().any(|row| {
        row.venue == "bybit"
            && row.operation == MARKET_OP_REST_METADATA
            && row.quality == MarketQuality::Unsupported
            && row.rows == 0
    }));
    assert!(rows.iter().any(|row| {
        row.venue == "binance"
            && row.operation == MARKET_OP_REST_METADATA
            && row.quality == MarketQuality::Fresh
            && row.source == MarketSource::RestColdStart
            && row.requested == 1
            && row.rows == 1
    }));
    assert!(rows.iter().any(|row| {
        row.venue == "okx"
            && row.operation == MARKET_OP_REST_METADATA
            && row.quality == MarketQuality::RateLimited
            && row.retry_after_ms == Some(2_000)
            && row
                .problem
                .as_ref()
                .is_some_and(|problem| problem.operation == MARKET_OP_REST_METADATA)
    }));
}

fn item(
    symbol: &str,
    venue_long: Option<&str>,
    venue_short: Option<&str>,
) -> realtime::WatchlistItem {
    realtime::WatchlistItem {
        id: 0,
        symbol: symbol.to_owned(),
        venue_long: venue_long.map(str::to_owned),
        venue_short: venue_short.map(str::to_owned),
        min_net_yield: None,
        min_volume_24h: None,
        enabled: true,
        created_at_ms: 0,
        persistence: shared_types::WatchlistPersistence::default(),
        runtime: shared_types::WatchlistItemRuntime::default(),
    }
}

fn disabled_item(
    symbol: &str,
    venue_long: Option<&str>,
    venue_short: Option<&str>,
) -> realtime::WatchlistItem {
    let mut row = item(symbol, venue_long, venue_short);
    row.enabled = false;
    row
}
