use super::*;

#[test]
fn funding_payment_budget_is_background_scoped() {
    assert_eq!(
        FUNDING_PAYMENT_ROUTE_TIMEOUT,
        std::time::Duration::from_secs(10)
    );
    assert!(FUNDING_PAYMENT_ROUTE_TIMEOUT > OPEN_ORDER_ROUTE_TIMEOUT);
}

#[tokio::test]
async fn live_router_funding_payments_are_partial_tolerant_and_skip_unsupported() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        funding_payment_adapter("binance", FundingPaymentRead::One),
    );
    routes.insert(
        "gate".into(),
        funding_payment_adapter("gate", FundingPaymentRead::Error),
    );
    routes.insert(
        "hyperliquid".into(),
        funding_payment_adapter("hyperliquid", FundingPaymentRead::Unsupported),
    );
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let rows = router
        .get_funding_payments(None, Some(1), Some(20))
        .await
        .unwrap_or_default();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].venue, "binance");
    let failures = sink.take("funding_payments");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].venue, "gate");
    assert_eq!(failures[0].operation, "funding_payments");
}

#[tokio::test]
async fn configured_funding_routes_kucoin_per_distinct_symbol_and_others_account_wide() {
    let kucoin_calls = Arc::new(Mutex::new(Vec::new()));
    let binance_calls = Arc::new(Mutex::new(Vec::new()));
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "kucoin".into(),
        funding_payment_recording_adapter("kucoin", Arc::clone(&kucoin_calls)),
    );
    routes.insert(
        "binance".into(),
        funding_payment_recording_adapter("binance", Arc::clone(&binance_calls)),
    );
    let router = LiveVenueRouter::new(routes);

    let _ = router
        .get_configured_funding_payments(
            &["ETH".into(), "BTC".into(), "btc".into(), " ".into()],
            Some(1),
            Some(20),
        )
        .await;

    let mut kucoin = kucoin_calls.lock().clone();
    kucoin.sort();
    assert_eq!(kucoin, vec![Some("BTC".into()), Some("ETH".into())]);
    assert_eq!(*binance_calls.lock(), vec![None]);
}

#[tokio::test]
async fn configured_funding_skips_kucoin_when_no_candidate_symbols_exist() {
    let kucoin_calls = Arc::new(Mutex::new(Vec::new()));
    let binance_calls = Arc::new(Mutex::new(Vec::new()));
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "kucoin".into(),
        funding_payment_recording_adapter("kucoin", Arc::clone(&kucoin_calls)),
    );
    routes.insert(
        "binance".into(),
        funding_payment_recording_adapter("binance", Arc::clone(&binance_calls)),
    );
    let router = LiveVenueRouter::new(routes);

    let rows = router
        .get_configured_funding_payments(&[], Some(1), Some(20))
        .await
        .unwrap_or_default();

    assert!(rows.is_empty());
    assert!(kucoin_calls.lock().is_empty());
    assert_eq!(*binance_calls.lock(), vec![None]);
}
