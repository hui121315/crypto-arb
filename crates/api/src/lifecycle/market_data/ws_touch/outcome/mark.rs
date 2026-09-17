use super::*;

pub(in crate::lifecycle::market_data::ws_touch) fn record_mark_index_ready(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested_symbols: &[String],
    rows: &[shared_types::MarkIndexInfo],
    missing_symbols: &[String],
    outcomes: (PublicWsSubscribeOutcome, PublicWsIngestOutcome),
) -> usize {
    let (subscribe_outcome, ingest_outcome) = outcomes;
    let changed_rows = runtime
        .market_data
        .store_mark_index_rows(rows, MarketSource::WsPush);
    runtime.market_data.record_runtime_success(
        venue,
        MARKET_OP_MARK_INDEX,
        MarketSource::WsPush,
        requested_symbols.len(),
        rows.len(),
    );
    record_subscribe_outcome(
        runtime,
        venue,
        MARKET_OP_WS_MARK_INDEX_SUBSCRIBE,
        requested_symbols,
        rows.len(),
        subscribe_outcome,
    );
    record_ingest_outcome(
        runtime,
        ws_runtime_sample(
            venue,
            MARKET_OP_WS_MARK_INDEX_SNAPSHOT,
            requested_symbols,
            rows.len(),
            missing_symbols,
        ),
        ingest_outcome,
    );
    log_touch(
        venue,
        "mark_index",
        "ws_snapshot",
        requested_symbols.len(),
        rows.len(),
    );
    changed_rows
}
