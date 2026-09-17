use super::*;

#[tokio::test]
async fn ws_touch_stores_ticker_and_funding_rows_in_market_cache() {
    let runtime = test_runtime();
    runtime.aggregator.register(Arc::new(TouchAdapter::ready()));
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

    assert_eq!(stats.changed_rows, 3);
    let snapshot = runtime.market_data.market_snapshot_cached();
    assert_eq!(snapshot.perp_tickers.len(), 1);
    assert_eq!(snapshot.perp_tickers[0].symbol, "MU");
    assert_eq!(snapshot.funding.len(), 1);
    assert_eq!(
        snapshot
            .funding
            .get("MU")
            .and_then(|rows| rows.get("bybit"))
            .map(|row| row.symbol.as_str()),
        Some("MU")
    );

    let health = runtime.market_data.runtime_health_snapshot();
    for operation in [MARKET_OP_PERP_TICKERS, MARKET_OP_FUNDING_RATES] {
        assert!(health.iter().any(|row| {
            row.venue == "bybit"
                && row.operation == operation
                && row.quality == MarketQuality::Fresh
                && row.source == MarketSource::WsPush
                && row.requested == 1
                && row.rows == 1
        }));
    }
    for operation in [
        MARKET_OP_WS_TICKER_SUBSCRIBE,
        MARKET_OP_WS_TICKER_SNAPSHOT,
        MARKET_OP_WS_FUNDING_SUBSCRIBE,
        MARKET_OP_WS_FUNDING_SNAPSHOT,
    ] {
        assert!(health.iter().any(|row| {
            row.venue == "bybit"
                && row.operation == operation
                && row.quality == MarketQuality::Fresh
                && row.source == MarketSource::WsPush
                && row.requested == 1
                && row.rows == 1
        }));
    }
}

#[tokio::test]
async fn position_mark_ingest_stays_on_the_public_ws_path() {
    let runtime = test_runtime();
    runtime.aggregator.register(Arc::new(TouchAdapter::ready()));
    let marks = BTreeMap::from([("bybit".to_owned(), vec!["MU".to_owned()])]);

    let stats = ingest_ws_market_updates(
        &runtime,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &marks,
        false,
    )
    .await;

    assert_eq!(stats.requested, 1);
    assert_eq!(stats.rows, 1);
    assert_eq!(stats.mark_changed_rows, 1);
    assert_eq!(
        runtime
            .market_data
            .fresh_ws_mark_index("bybit", "MUUSDT", common::time::now_ms())
            .map(|(row, _)| row.mark_price),
        Some(100.25)
    );
    let health = runtime.market_data.runtime_health_snapshot();
    for operation in [
        MARKET_OP_MARK_INDEX,
        MARKET_OP_WS_MARK_INDEX_SUBSCRIBE,
        MARKET_OP_WS_MARK_INDEX_SNAPSHOT,
    ] {
        assert!(health.iter().any(|row| {
            row.venue == "bybit"
                && row.operation == operation
                && row.quality == MarketQuality::Fresh
                && row.source == MarketSource::WsPush
        }));
    }
}
