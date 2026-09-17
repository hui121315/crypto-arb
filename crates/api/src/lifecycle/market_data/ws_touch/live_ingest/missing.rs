use super::*;

pub(super) fn record_missing_ws_adapter(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested: usize,
) {
    for operation in [MARKET_OP_WS_TICKER_SUBSCRIBE, MARKET_OP_WS_TICKER_SNAPSHOT] {
        runtime.market_data.record_runtime_unsupported(
            venue,
            operation,
            MarketSource::WsPush,
            requested,
            "adapter is not registered",
        );
    }
}

pub(super) fn record_missing_funding_ws_adapter(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested: usize,
) {
    for operation in [
        MARKET_OP_WS_FUNDING_SUBSCRIBE,
        MARKET_OP_WS_FUNDING_SNAPSHOT,
    ] {
        runtime.market_data.record_runtime_unsupported(
            venue,
            operation,
            MarketSource::WsPush,
            requested,
            "adapter is not registered",
        );
    }
}

pub(super) fn record_missing_mark_ws_adapter(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested: usize,
) {
    for operation in [
        MARKET_OP_WS_MARK_INDEX_SUBSCRIBE,
        MARKET_OP_WS_MARK_INDEX_SNAPSHOT,
    ] {
        runtime.market_data.record_runtime_unsupported(
            venue,
            operation,
            MarketSource::WsPush,
            requested,
            "adapter is not registered",
        );
    }
}

pub(super) fn record_missing_spot_ws_adapter(
    runtime: &MarketDataRuntime,
    venue: &str,
    requested: usize,
) {
    runtime.market_data.record_runtime_unsupported(
        venue,
        MARKET_OP_WS_SPOT_SNAPSHOT,
        MarketSource::WsPush,
        requested,
        "adapter is not registered",
    );
}
