use super::*;

#[test]
fn spot_tick_row_evidence_preserves_ws_source() {
    let cache = MarketDataCache::default();
    let row = spot_tick("BTC");
    cache.store_spot_ticks(std::slice::from_ref(&row), MarketSource::WsPush);

    let evidence = cache.spot_tick_row_evidence(&[row]);

    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].venue, "mock");
    assert_eq!(evidence[0].symbol, "BTC");
    assert_eq!(
        evidence[0].operation,
        MarketDataSnapshotOperation::SpotTicks
    );
    assert_eq!(
        evidence[0].health.source,
        shared_types::MarketDataSourceKind::WsPush
    );
}
#[test]
fn perp_ticker_row_evidence_preserves_ws_source() {
    let cache = MarketDataCache::default();
    let row = ticker("BTC");
    cache.store_ticker_rows(std::slice::from_ref(&row), MarketSource::WsPush);

    let evidence = cache.perp_ticker_row_evidence(&[row]);

    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].venue, "mock");
    assert_eq!(evidence[0].symbol, "BTC");
    assert_eq!(
        evidence[0].operation,
        MarketDataSnapshotOperation::PerpTickers
    );
    assert_eq!(
        evidence[0].health.source,
        shared_types::MarketDataSourceKind::WsPush
    );
}
