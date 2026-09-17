use super::*;

#[tokio::test]
async fn ws_venue_ingest_does_not_serialize_slow_adapters(
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let runtime = Arc::new(test_runtime());
    let barrier = Arc::new(tokio::sync::Barrier::new(3));
    runtime
        .aggregator
        .register(Arc::new(CoordinatedTouchAdapter::new(
            "bybit",
            Arc::clone(&barrier),
        )));
    runtime
        .aggregator
        .register(Arc::new(CoordinatedTouchAdapter::new(
            "okx",
            Arc::clone(&barrier),
        )));
    let requests = BTreeMap::from([
        ("bybit".to_owned(), vec!["BTC".to_owned()]),
        ("okx".to_owned(), vec!["ETH".to_owned()]),
    ]);
    let task_runtime = Arc::clone(&runtime);
    let task = tokio::spawn(async move {
        let empty = BTreeMap::new();
        ingest_ws_market_updates(&task_runtime, &requests, &empty, &empty, &empty, false).await
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), barrier.wait()).await?;
    let stats = task.await?;

    assert_eq!(stats.requested, 2);
    assert_eq!(stats.rows, 0);
    Ok(())
}

#[tokio::test]
async fn ws_pending_records_only_ws_warming_state() {
    let runtime = test_runtime();
    runtime.aggregator.register(Arc::new(FailingTouchAdapter));
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
    assert!(!rows.iter().any(|row| matches!(
        row.operation,
        MARKET_OP_REST_TICKER_FALLBACK
            | MARKET_OP_REST_FUNDING_FALLBACK
            | MARKET_OP_REST_SPOT_TICKS
    )));
    for operation in [
        MARKET_OP_WS_TICKER_SUBSCRIBE,
        MARKET_OP_WS_FUNDING_SUBSCRIBE,
    ] {
        assert!(rows.iter().any(|row| {
            row.venue == "bybit"
                && row.operation == operation
                && row.quality == MarketQuality::Warming
                && row.source == MarketSource::WsPush
                && row.retry_after_ms == Some(10_000)
                && row.problem.is_none()
        }));
    }
    for operation in [MARKET_OP_WS_TICKER_SNAPSHOT, MARKET_OP_WS_FUNDING_SNAPSHOT] {
        assert!(rows.iter().any(|row| {
            row.venue == "bybit"
                && row.operation == operation
                && row.quality == MarketQuality::Warming
                && row.retry_after_ms == Some(10_000)
                && row.problem.is_none()
        }));
    }
}

#[tokio::test]
async fn partial_ws_snapshots_keep_missing_symbols_on_ws_warmup() {
    let runtime = test_runtime();
    runtime
        .aggregator
        .register(Arc::new(TouchAdapter::partial()));
    let requests = BTreeMap::from([("bybit".to_owned(), vec!["BTC".to_owned(), "ETH".to_owned()])]);

    let stats = ingest_ws_market_updates(
        &runtime,
        &requests,
        &requests,
        &requests,
        &BTreeMap::new(),
        true,
    )
    .await;

    assert_eq!(stats.requested, 6);
    assert_eq!(stats.rows, 3);
    assert_eq!(stats.changed_rows, 3);
    let funding = runtime.market_data.funding_rows_snapshot();
    assert_eq!(funding.len(), 1);
    assert!(funding.iter().any(|row| row.symbol == "BTC"));
    let rows = runtime.market_data.runtime_health_snapshot();
    for operation in [
        MARKET_OP_WS_TICKER_SNAPSHOT,
        MARKET_OP_WS_FUNDING_SNAPSHOT,
        MARKET_OP_WS_SPOT_SNAPSHOT,
    ] {
        assert!(rows.iter().any(|row| {
            row.venue == "bybit"
                && row.operation == operation
                && row.quality == MarketQuality::Warming
                && row.requested == 2
                && row.rows == 1
        }));
    }
    assert!(!rows.iter().any(|row| matches!(
        row.operation,
        MARKET_OP_REST_TICKER_FALLBACK
            | MARKET_OP_REST_FUNDING_FALLBACK
            | MARKET_OP_REST_SPOT_TICKS
    )));
}
