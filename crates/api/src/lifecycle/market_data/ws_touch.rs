use super::*;

mod live_ingest;
mod outcome;

pub(in crate::lifecycle::market_data) use live_ingest::{
    ingest_ws_market_updates, ingest_ws_spot_market_updates,
};

pub(super) async fn prewarm_ws_tickers(
    runtime: &MarketDataRuntime,
    requests: &BTreeMap<String, Vec<String>>,
) -> usize {
    use crate::services::market_subscriptions::MarketSubscriptionFeed;

    let mut spot = requests.clone();
    spot.retain(|venue, _| {
        runtime
            .market_subscriptions
            .enabled(venue, MarketSubscriptionFeed::Spot)
    });
    let mut perp = requests.clone();
    perp.retain(|venue, _| {
        runtime
            .market_subscriptions
            .enabled(venue, MarketSubscriptionFeed::Perp)
    });
    let mut funding = requests.clone();
    funding.retain(|venue, _| {
        runtime
            .market_subscriptions
            .enabled(venue, MarketSubscriptionFeed::Funding)
    });
    let no_position_marks = BTreeMap::new();
    ingest_ws_market_updates(runtime, &perp, &spot, &funding, &no_position_marks, true)
        .await
        .changed_rows
}

pub(super) fn record_spot_ready(
    runtime: &MarketDataRuntime,
    venue: &str,
    symbols: &[String],
    rows: &[shared_types::SpotTick],
    missing_symbols: &[String],
) -> usize {
    // KuCoin's official ticker contract pushes only after a trade/BBO event.
    // A broad non-empty snapshot therefore proves the stream is ingesting;
    // quiet symbols remain absent until their first event and are still
    // checked individually before they can become executable.
    let health_missing_symbols = if venue == "kucoin" && !rows.is_empty() {
        &[]
    } else {
        missing_symbols
    };
    let changed_rows = runtime
        .market_data
        .store_spot_ticks(rows, MarketSource::WsPush);
    runtime.market_data.record_runtime_success(
        venue,
        MARKET_OP_SPOT_TICKS,
        MarketSource::WsPush,
        1,
        rows.len(),
    );
    runtime
        .market_data
        .record_ws_runtime(outcome::ws_runtime_sample(
            venue,
            MARKET_OP_WS_SPOT_SNAPSHOT,
            symbols,
            rows.len(),
            health_missing_symbols,
        ));
    log_touch(venue, "spot", "ws_snapshot", symbols.len(), rows.len());
    changed_rows
}

pub(super) fn record_spot_pending(runtime: &MarketDataRuntime, venue: &str, symbols: &[String]) {
    runtime
        .market_data
        .record_ws_runtime(outcome::ws_runtime_sample(
            venue,
            MARKET_OP_WS_SPOT_SNAPSHOT,
            symbols,
            0,
            symbols,
        ));
    debug!(venue, symbols = symbols.len(), "spot ws snapshot pending");
}

pub(super) fn record_spot_unsupported(
    runtime: &MarketDataRuntime,
    venue: &str,
    symbol_count: usize,
) {
    runtime.market_data.record_runtime_unsupported(
        venue,
        MARKET_OP_WS_SPOT_SNAPSHOT,
        MarketSource::WsPush,
        symbol_count,
        "public spot ws operation is not implemented by this adapter",
    );
}

pub(super) fn missing_symbols<T>(
    adapter: &dyn exchange::ExchangeAdapter,
    requested: &[String],
    rows: &[T],
    row_symbol: fn(&T) -> &str,
) -> Vec<String> {
    let present = rows
        .iter()
        .map(|row| adapter.normalize_symbol(row_symbol(row)))
        .collect::<std::collections::HashSet<_>>();
    requested
        .iter()
        .filter(|symbol| !present.contains(&adapter.normalize_symbol(symbol)))
        .cloned()
        .collect()
}

fn log_touch(venue: &str, kind: &str, source: &str, symbols: usize, rows: usize) {
    debug!(
        venue,
        kind, source, symbols, rows, "public market ws snapshot ingested"
    );
}
