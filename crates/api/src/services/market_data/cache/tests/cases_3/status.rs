use super::*;

#[test]
fn snapshot_status_keeps_ws_subscription_health_out_of_data_coverage() {
    let cache = MarketDataCache::default();
    cache.record_runtime_success(
        "kucoin",
        MARKET_OP_WS_TICKER_SUBSCRIBE,
        MarketSource::WsPush,
        8,
        8,
    );
    cache.record_runtime_success(
        "kucoin",
        MARKET_OP_WS_TICKER_SNAPSHOT,
        MarketSource::WsPush,
        8,
        7,
    );

    let status = cache.snapshot_status(common::time::now_ms());
    let rows = status
        .rows
        .iter()
        .filter(|row| {
            row.venue == "kucoin" && row.operation == MarketDataSnapshotOperation::WsTicker
        })
        .collect::<Vec<_>>();

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].health.quality,
        shared_types::MarketDataQuality::Missing
    );
    assert_eq!(
        rows[0]
            .health
            .coverage
            .as_ref()
            .map(|coverage| (coverage.requested, coverage.received)),
        Some((8, 7))
    );
}
