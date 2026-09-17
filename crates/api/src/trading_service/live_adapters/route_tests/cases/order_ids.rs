use super::*;

#[tokio::test]
async fn live_router_exchange_order_id_lookup_hits_only_target_route() {
    let binance_reads = Arc::new(AtomicUsize::new(0));
    let gate_reads = Arc::new(AtomicUsize::new(0));
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        order_adapter("binance", OrderRead::Error, Arc::clone(&binance_reads)),
    );
    routes.insert(
        "gate".into(),
        order_adapter("gate", OrderRead::One, Arc::clone(&gate_reads)),
    );
    let router = LiveVenueRouter::new(routes);

    let order = router
        .get_exchange_order_by_exchange_order_id("gate", "BTCUSDT", "777")
        .await
        .ok()
        .flatten();

    assert_eq!(
        order.as_ref().map(|row| row.exchange.as_str()),
        Some("gate")
    );
    assert_eq!(binance_reads.load(Ordering::Relaxed), 0);
    assert_eq!(gate_reads.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn live_router_get_exchange_order_hits_only_target_route() {
    let binance_reads = Arc::new(AtomicUsize::new(0));
    let gate_reads = Arc::new(AtomicUsize::new(0));
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        order_adapter("binance", OrderRead::One, Arc::clone(&binance_reads)),
    );
    routes.insert(
        "gate".into(),
        order_adapter("gate", OrderRead::Error, Arc::clone(&gate_reads)),
    );
    let router = LiveVenueRouter::new(routes);

    let order = router
        .get_exchange_order("binance", "BTCUSDT", "client-1")
        .await
        .ok()
        .flatten();

    assert_eq!(
        order.as_ref().map(|row| row.exchange.as_str()),
        Some("binance")
    );
    assert_eq!(binance_reads.load(Ordering::Relaxed), 1);
    assert_eq!(gate_reads.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn live_router_generic_order_read_fails_without_scanning_routes() {
    let binance_reads = Arc::new(AtomicUsize::new(0));
    let gate_reads = Arc::new(AtomicUsize::new(0));
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        order_adapter("binance", OrderRead::One, Arc::clone(&binance_reads)),
    );
    routes.insert(
        "gate".into(),
        order_adapter("gate", OrderRead::One, Arc::clone(&gate_reads)),
    );
    let router = LiveVenueRouter::new(routes);

    let result = router.get_order("BTCUSDT", "client-1").await;

    assert!(matches!(result, Err(ExchangeError::NotImplemented(_))));
    assert_eq!(binance_reads.load(Ordering::Relaxed), 0);
    assert_eq!(gate_reads.load(Ordering::Relaxed), 0);
}
