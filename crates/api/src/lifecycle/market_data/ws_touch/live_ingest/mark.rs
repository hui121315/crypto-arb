use super::*;

pub(super) async fn ingest_mark_venue(
    runtime: &MarketDataRuntime,
    venue: &str,
    symbols: &[String],
) -> WsLiveIngestStats {
    let mut stats = requested_stats(symbols);
    if symbols.is_empty() {
        return stats;
    }
    let Some(adapter) = runtime.aggregator.get(venue) else {
        record_missing_mark_ws_adapter(runtime, venue, symbols.len());
        return stats;
    };
    ingest_ws_mark_index(runtime, venue, adapter.as_ref(), symbols, &mut stats).await;
    stats
}

async fn ingest_ws_mark_index(
    runtime: &MarketDataRuntime,
    venue: &str,
    adapter: &dyn exchange::ExchangeAdapter,
    symbols: &[String],
    stats: &mut WsLiveIngestStats,
) {
    let snapshot = adapter.public_ws_mark_index_snapshot(symbols).await;
    if !feed_enabled(runtime, venue, WsFeed::Mark) {
        return;
    }
    match snapshot {
        Ok(snapshot) => {
            let subscribe_outcome = snapshot.subscribe_outcome();
            let ingest_outcome = snapshot.ingest_outcome();
            match snapshot {
                PublicWsSnapshot::Ready(rows) if !rows.is_empty() => {
                    let missing = super::super::missing_symbols(adapter, symbols, &rows, |row| {
                        row.symbol.as_str()
                    });
                    stats.rows = stats.rows.saturating_add(rows.len());
                    let changed = record_mark_index_ready(
                        runtime,
                        venue,
                        symbols,
                        &rows,
                        &missing,
                        (subscribe_outcome, ingest_outcome),
                    );
                    stats.changed_rows = stats.changed_rows.saturating_add(changed);
                    stats.mark_changed_rows = stats.mark_changed_rows.saturating_add(changed);
                }
                PublicWsSnapshot::Ready(_) | PublicWsSnapshot::Pending => record_ws_pending(
                    runtime,
                    venue,
                    symbols,
                    (
                        MARKET_OP_WS_MARK_INDEX_SUBSCRIBE,
                        MARKET_OP_WS_MARK_INDEX_SNAPSHOT,
                    ),
                    (subscribe_outcome, ingest_outcome),
                ),
                PublicWsSnapshot::Unsupported => record_ws_unsupported(
                    runtime,
                    venue,
                    symbols,
                    (
                        MARKET_OP_WS_MARK_INDEX_SUBSCRIBE,
                        MARKET_OP_WS_MARK_INDEX_SNAPSHOT,
                    ),
                    (subscribe_outcome, ingest_outcome),
                ),
            }
        }
        Err(error) => record_ws_error(
            runtime,
            venue,
            symbols.len(),
            MARKET_OP_WS_MARK_INDEX_SUBSCRIBE,
            MARKET_OP_WS_MARK_INDEX_SNAPSHOT,
            &error,
        ),
    }
}
