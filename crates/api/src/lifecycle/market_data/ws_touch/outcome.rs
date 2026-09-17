use super::*;
use exchange::{PublicWsIngestOutcome, PublicWsSubscribeOutcome};

mod mark;

pub(super) use mark::record_mark_index_ready;

const WS_UNSUPPORTED: &str = "public ws operation is not implemented by this adapter";
const DEFAULT_WS_WARMUP_GRACE_MS: i64 = 10_000;
const GATE_MARKET_WARMUP_GRACE_MS: i64 = 30_000;
const KUCOIN_SPOT_WARMUP_GRACE_MS: i64 = 8_000;
const KUCOIN_TICKER_WARMUP_GRACE_MS: i64 = 7_000;
const KUCOIN_FUNDING_WARMUP_GRACE_MS: i64 = 65_000;

pub(super) fn record_ws_ready(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested_symbols: &[String],
    rows: &[shared_types::TickerInfo],
    missing_symbols: &[String],
    outcomes: (PublicWsSubscribeOutcome, PublicWsIngestOutcome),
) -> usize {
    let (subscribe_outcome, ingest_outcome) = outcomes;
    let changed_rows = runtime
        .market_data
        .store_ticker_rows(rows, MarketSource::WsPush);
    runtime.market_data.record_runtime_success(
        venue,
        MARKET_OP_PERP_TICKERS,
        MarketSource::WsPush,
        1,
        rows.len(),
    );
    record_subscribe_outcome(
        runtime,
        venue,
        MARKET_OP_WS_TICKER_SUBSCRIBE,
        requested_symbols,
        rows.len(),
        subscribe_outcome,
    );
    record_ingest_outcome(
        runtime,
        ws_runtime_sample(
            venue,
            MARKET_OP_WS_TICKER_SNAPSHOT,
            requested_symbols,
            rows.len(),
            missing_symbols,
        ),
        ingest_outcome,
    );
    log_touch(
        venue,
        "ticker",
        "ws_snapshot",
        requested_symbols.len(),
        rows.len(),
    );
    changed_rows
}

pub(super) fn record_funding_ready(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested_symbols: &[String],
    rows: &[shared_types::FundingRateData],
    missing_symbols: &[String],
    outcomes: (PublicWsSubscribeOutcome, PublicWsIngestOutcome),
) -> usize {
    let (subscribe_outcome, ingest_outcome) = outcomes;
    let changed_rows = runtime
        .market_data
        .store_funding_rows(rows, MarketSource::WsPush);
    runtime.market_data.record_runtime_success(
        venue,
        MARKET_OP_FUNDING_RATES,
        MarketSource::WsPush,
        1,
        rows.len(),
    );
    record_subscribe_outcome(
        runtime,
        venue,
        MARKET_OP_WS_FUNDING_SUBSCRIBE,
        requested_symbols,
        rows.len(),
        subscribe_outcome,
    );
    record_ingest_outcome(
        runtime,
        ws_runtime_sample(
            venue,
            MARKET_OP_WS_FUNDING_SNAPSHOT,
            requested_symbols,
            rows.len(),
            missing_symbols,
        ),
        ingest_outcome,
    );
    log_touch(
        venue,
        "funding",
        "ws_snapshot",
        requested_symbols.len(),
        rows.len(),
    );
    changed_rows
}

pub(super) fn record_ws_pending(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested_symbols: &[String],
    operations: (&'static str, &'static str),
    outcomes: (PublicWsSubscribeOutcome, PublicWsIngestOutcome),
) {
    let (subscribe_operation, snapshot_operation) = operations;
    let (subscribe_outcome, ingest_outcome) = outcomes;
    record_subscribe_outcome(
        runtime,
        venue,
        subscribe_operation,
        requested_symbols,
        0,
        subscribe_outcome,
    );
    record_ingest_outcome(
        runtime,
        ws_runtime_sample(
            venue,
            snapshot_operation,
            requested_symbols,
            0,
            requested_symbols,
        ),
        ingest_outcome,
    );
}

pub(super) fn record_ws_unsupported(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested_symbols: &[String],
    operations: (&'static str, &'static str),
    outcomes: (PublicWsSubscribeOutcome, PublicWsIngestOutcome),
) {
    let (subscribe_operation, snapshot_operation) = operations;
    let (subscribe_outcome, ingest_outcome) = outcomes;
    record_subscribe_outcome(
        runtime,
        venue,
        subscribe_operation,
        requested_symbols,
        0,
        subscribe_outcome,
    );
    record_ingest_outcome(
        runtime,
        ws_runtime_sample(
            venue,
            snapshot_operation,
            requested_symbols,
            0,
            requested_symbols,
        ),
        ingest_outcome,
    );
}

fn record_ingest_outcome(
    runtime: &MarketDataRuntime,
    sample: WsRuntimeSample<'_>,
    outcome: PublicWsIngestOutcome,
) {
    match outcome {
        PublicWsIngestOutcome::Ingested | PublicWsIngestOutcome::AwaitingFirstEvent => {
            runtime.market_data.record_ws_runtime(sample)
        }
        PublicWsIngestOutcome::Unsupported => runtime.market_data.record_runtime_unsupported(
            sample.venue,
            sample.operation,
            MarketSource::WsPush,
            sample.requested_symbols.len(),
            WS_UNSUPPORTED,
        ),
    }
}

fn record_subscribe_outcome(
    runtime: &MarketDataRuntime,
    venue: &str,
    operation: &'static str,
    requested_symbols: &[String],
    rows: usize,
    outcome: PublicWsSubscribeOutcome,
) {
    match outcome {
        PublicWsSubscribeOutcome::Confirmed => runtime.market_data.record_runtime_success(
            venue,
            operation,
            MarketSource::WsPush,
            requested_symbols.len(),
            requested_symbols.len(),
        ),
        PublicWsSubscribeOutcome::Requested => runtime.market_data.record_ws_runtime(
            ws_runtime_sample(venue, operation, requested_symbols, rows, requested_symbols),
        ),
        PublicWsSubscribeOutcome::Unsupported => runtime.market_data.record_runtime_unsupported(
            venue,
            operation,
            MarketSource::WsPush,
            requested_symbols.len(),
            WS_UNSUPPORTED,
        ),
    }
}

pub(super) fn ws_runtime_sample<'a>(
    venue: &'a str,
    operation: &'static str,
    requested_symbols: &'a [String],
    rows: usize,
    missing_symbols: &'a [String],
) -> WsRuntimeSample<'a> {
    WsRuntimeSample {
        venue,
        operation,
        source: MarketSource::WsPush,
        requested_symbols,
        rows,
        missing_symbols,
        grace_ms: ws_warmup_grace_ms(venue, operation),
    }
}

pub(super) fn ws_warmup_grace_ms(venue: &str, operation: &str) -> i64 {
    if venue == "gate"
        && matches!(
            operation,
            MARKET_OP_WS_FUNDING_SNAPSHOT | MARKET_OP_WS_TICKER_SNAPSHOT
        )
    {
        return GATE_MARKET_WARMUP_GRACE_MS;
    }
    if venue == "kucoin" {
        return match operation {
            MARKET_OP_WS_FUNDING_SUBSCRIBE | MARKET_OP_WS_FUNDING_SNAPSHOT => {
                KUCOIN_FUNDING_WARMUP_GRACE_MS
            }
            MARKET_OP_WS_TICKER_SUBSCRIBE | MARKET_OP_WS_TICKER_SNAPSHOT => {
                KUCOIN_TICKER_WARMUP_GRACE_MS
            }
            MARKET_OP_WS_SPOT_SNAPSHOT => KUCOIN_SPOT_WARMUP_GRACE_MS,
            _ => DEFAULT_WS_WARMUP_GRACE_MS,
        };
    }
    DEFAULT_WS_WARMUP_GRACE_MS
}

pub(super) fn record_ws_error(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested: usize,
    subscribe_operation: &'static str,
    snapshot_operation: &'static str,
    error: &exchange::ExchangeError,
) {
    for operation in [subscribe_operation, snapshot_operation] {
        runtime.market_data.record_runtime_error(
            venue,
            operation,
            MarketSource::WsPush,
            requested,
            error,
        );
    }
}

#[cfg(test)]
#[path = "outcome_tests.rs"]
mod tests;
