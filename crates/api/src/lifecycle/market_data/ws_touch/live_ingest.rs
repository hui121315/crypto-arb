use super::outcome::{
    record_funding_ready, record_mark_index_ready, record_ws_error, record_ws_pending,
    record_ws_ready, record_ws_unsupported,
};
use super::*;
use crate::services::market_subscriptions::MarketSubscriptionFeed;
use exchange::PublicWsSnapshot;
use futures::{stream::FuturesUnordered, StreamExt};

mod mark;
mod missing;
mod types;

use mark::ingest_mark_venue;
use missing::{
    record_missing_funding_ws_adapter, record_missing_mark_ws_adapter,
    record_missing_spot_ws_adapter, record_missing_ws_adapter,
};
use types::WsFeed;
pub(in crate::lifecycle::market_data) use types::WsLiveIngestStats;

const WS_VENUE_INGEST_CONCURRENCY: usize = 4;

pub(in crate::lifecycle::market_data) async fn ingest_ws_market_updates(
    runtime: &MarketDataRuntime,
    perp_requests: &BTreeMap<String, Vec<String>>,
    spot_requests: &BTreeMap<String, Vec<String>>,
    funding_requests: &BTreeMap<String, Vec<String>>,
    mark_requests: &BTreeMap<String, Vec<String>>,
    include_funding: bool,
) -> WsLiveIngestStats {
    let mut stats = ingest_requests(runtime, perp_requests, WsFeed::Perp).await;
    stats.merge(ingest_requests(runtime, spot_requests, WsFeed::Spot).await);
    stats.merge(ingest_requests(runtime, mark_requests, WsFeed::Mark).await);
    if include_funding {
        stats.merge(ingest_requests(runtime, funding_requests, WsFeed::Funding).await);
    }
    stats
}

pub(in crate::lifecycle::market_data) async fn ingest_ws_spot_market_updates(
    runtime: &MarketDataRuntime,
    spot_requests: &BTreeMap<String, Vec<String>>,
) -> WsLiveIngestStats {
    ingest_requests(runtime, spot_requests, WsFeed::Spot).await
}

async fn ingest_requests(
    runtime: &MarketDataRuntime,
    requests: &BTreeMap<String, Vec<String>>,
    feed: WsFeed,
) -> WsLiveIngestStats {
    let mut source = requests.iter();
    let mut pending = FuturesUnordered::new();
    for _ in 0..WS_VENUE_INGEST_CONCURRENCY {
        let Some((venue, symbols)) = source.next() else {
            break;
        };
        pending.push(ingest_venue(runtime, feed, venue, symbols));
    }

    let mut total = WsLiveIngestStats::default();
    while let Some(next) = pending.next().await {
        total.merge(next);
        if let Some((venue, symbols)) = source.next() {
            pending.push(ingest_venue(runtime, feed, venue, symbols));
        }
    }
    total
}

async fn ingest_venue(
    runtime: &MarketDataRuntime,
    feed: WsFeed,
    venue: &str,
    symbols: &[String],
) -> WsLiveIngestStats {
    if !feed_enabled(runtime, venue, feed) {
        return WsLiveIngestStats::default();
    }
    match feed {
        WsFeed::Perp => ingest_perp_venue(runtime, venue, symbols).await,
        WsFeed::Spot => ingest_spot_venue(runtime, venue, symbols).await,
        WsFeed::Funding => ingest_funding_venue(runtime, venue, symbols).await,
        WsFeed::Mark => ingest_mark_venue(runtime, venue, symbols).await,
    }
}

fn feed_enabled(runtime: &MarketDataRuntime, venue: &str, feed: WsFeed) -> bool {
    let feed = match feed {
        WsFeed::Perp | WsFeed::Mark => MarketSubscriptionFeed::Perp,
        WsFeed::Spot => MarketSubscriptionFeed::Spot,
        WsFeed::Funding => MarketSubscriptionFeed::Funding,
    };
    runtime.market_subscriptions.enabled(venue, feed)
}

async fn ingest_perp_venue(
    runtime: &MarketDataRuntime,
    venue: &str,
    symbols: &[String],
) -> WsLiveIngestStats {
    let mut stats = requested_stats(symbols);
    if symbols.is_empty() {
        return stats;
    }
    let Some(adapter) = runtime.aggregator.get(venue) else {
        record_missing_ws_adapter(runtime, venue, symbols.len());
        return stats;
    };
    ingest_ws_ticker(runtime, venue, adapter.as_ref(), symbols, &mut stats).await;
    stats
}

async fn ingest_funding_venue(
    runtime: &MarketDataRuntime,
    venue: &str,
    symbols: &[String],
) -> WsLiveIngestStats {
    let mut stats = requested_stats(symbols);
    if symbols.is_empty() {
        return stats;
    }
    let Some(adapter) = runtime.aggregator.get(venue) else {
        record_missing_funding_ws_adapter(runtime, venue, symbols.len());
        return stats;
    };
    ingest_ws_funding(runtime, venue, adapter.as_ref(), symbols, &mut stats).await;
    stats
}

async fn ingest_spot_venue(
    runtime: &MarketDataRuntime,
    venue: &str,
    symbols: &[String],
) -> WsLiveIngestStats {
    let mut stats = requested_stats(symbols);
    if symbols.is_empty() {
        return stats;
    }
    let Some(adapter) = runtime.aggregator.get(venue) else {
        record_missing_spot_ws_adapter(runtime, venue, symbols.len());
        return stats;
    };
    ingest_ws_spot(runtime, venue, adapter.as_ref(), symbols, &mut stats).await;
    stats
}

fn requested_stats(symbols: &[String]) -> WsLiveIngestStats {
    WsLiveIngestStats {
        requested: symbols.len(),
        ..Default::default()
    }
}

async fn ingest_ws_spot(
    runtime: &MarketDataRuntime,
    venue: &str,
    adapter: &dyn exchange::ExchangeAdapter,
    symbols: &[String],
    stats: &mut WsLiveIngestStats,
) {
    let snapshot = adapter.public_ws_spot_snapshot(symbols).await;
    if !feed_enabled(runtime, venue, WsFeed::Spot) {
        return;
    }
    match snapshot {
        Ok(PublicWsSnapshot::Ready(rows)) if !rows.is_empty() => {
            let missing =
                super::missing_symbols(adapter, symbols, &rows, |row| row.symbol.as_str());
            stats.rows = stats.rows.saturating_add(rows.len());
            stats.changed_rows = stats.changed_rows.saturating_add(super::record_spot_ready(
                runtime, venue, symbols, &rows, &missing,
            ));
        }
        Ok(PublicWsSnapshot::Ready(_) | PublicWsSnapshot::Pending) => {
            super::record_spot_pending(runtime, venue, symbols);
        }
        Ok(PublicWsSnapshot::Unsupported) => {
            super::record_spot_unsupported(runtime, venue, symbols.len());
        }
        Err(error) => runtime.market_data.record_runtime_error(
            venue,
            MARKET_OP_WS_SPOT_SNAPSHOT,
            MarketSource::WsPush,
            symbols.len(),
            &error,
        ),
    }
}

async fn ingest_ws_ticker(
    runtime: &MarketDataRuntime,
    venue: &str,
    adapter: &dyn exchange::ExchangeAdapter,
    symbols: &[String],
    stats: &mut WsLiveIngestStats,
) {
    let snapshot = adapter.public_ws_ticker_snapshot(symbols).await;
    if !feed_enabled(runtime, venue, WsFeed::Perp) {
        return;
    }
    match snapshot {
        Ok(snapshot) => {
            let subscribe_outcome = snapshot.subscribe_outcome();
            let ingest_outcome = snapshot.ingest_outcome();
            match snapshot {
                PublicWsSnapshot::Ready(rows) if !rows.is_empty() => {
                    let missing =
                        super::missing_symbols(adapter, symbols, &rows, |row| row.symbol.as_str());
                    stats.rows = stats.rows.saturating_add(rows.len());
                    stats.changed_rows = stats.changed_rows.saturating_add(record_ws_ready(
                        runtime,
                        venue,
                        symbols,
                        &rows,
                        &missing,
                        (subscribe_outcome, ingest_outcome),
                    ));
                }
                PublicWsSnapshot::Ready(_) | PublicWsSnapshot::Pending => record_ws_pending(
                    runtime,
                    venue,
                    symbols,
                    (MARKET_OP_WS_TICKER_SUBSCRIBE, MARKET_OP_WS_TICKER_SNAPSHOT),
                    (subscribe_outcome, ingest_outcome),
                ),
                PublicWsSnapshot::Unsupported => record_ws_unsupported(
                    runtime,
                    venue,
                    symbols,
                    (MARKET_OP_WS_TICKER_SUBSCRIBE, MARKET_OP_WS_TICKER_SNAPSHOT),
                    (subscribe_outcome, ingest_outcome),
                ),
            }
        }
        Err(error) => record_ws_error(
            runtime,
            venue,
            symbols.len(),
            MARKET_OP_WS_TICKER_SUBSCRIBE,
            MARKET_OP_WS_TICKER_SNAPSHOT,
            &error,
        ),
    }
}

async fn ingest_ws_funding(
    runtime: &MarketDataRuntime,
    venue: &str,
    adapter: &dyn exchange::ExchangeAdapter,
    symbols: &[String],
    stats: &mut WsLiveIngestStats,
) {
    let snapshot = adapter.public_ws_funding_snapshot(symbols).await;
    if !feed_enabled(runtime, venue, WsFeed::Funding) {
        return;
    }
    match snapshot {
        Ok(snapshot) => {
            let subscribe_outcome = snapshot.subscribe_outcome();
            let ingest_outcome = snapshot.ingest_outcome();
            match snapshot {
                PublicWsSnapshot::Ready(rows) if !rows.is_empty() => {
                    let missing =
                        super::missing_symbols(adapter, symbols, &rows, |row| row.symbol.as_str());
                    stats.rows = stats.rows.saturating_add(rows.len());
                    stats.changed_rows = stats.changed_rows.saturating_add(record_funding_ready(
                        runtime,
                        venue,
                        symbols,
                        &rows,
                        &missing,
                        (subscribe_outcome, ingest_outcome),
                    ));
                }
                PublicWsSnapshot::Ready(_) | PublicWsSnapshot::Pending => record_ws_pending(
                    runtime,
                    venue,
                    symbols,
                    (
                        MARKET_OP_WS_FUNDING_SUBSCRIBE,
                        MARKET_OP_WS_FUNDING_SNAPSHOT,
                    ),
                    (subscribe_outcome, ingest_outcome),
                ),
                PublicWsSnapshot::Unsupported => record_ws_unsupported(
                    runtime,
                    venue,
                    symbols,
                    (
                        MARKET_OP_WS_FUNDING_SUBSCRIBE,
                        MARKET_OP_WS_FUNDING_SNAPSHOT,
                    ),
                    (subscribe_outcome, ingest_outcome),
                ),
            }
        }
        Err(error) => record_ws_error(
            runtime,
            venue,
            symbols.len(),
            MARKET_OP_WS_FUNDING_SUBSCRIBE,
            MARKET_OP_WS_FUNDING_SNAPSHOT,
            &error,
        ),
    }
}
