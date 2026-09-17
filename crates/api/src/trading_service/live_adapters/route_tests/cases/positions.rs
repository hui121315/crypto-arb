use super::*;

#[tokio::test]
async fn live_router_positions_are_partial_tolerant() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        position_adapter("binance", PositionRead::One),
    );
    routes.insert("gate".into(), position_adapter("gate", PositionRead::Error));
    let router = LiveVenueRouter::new(routes);

    let rows = router.get_positions(None).await.unwrap_or_default();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "binance");
}

#[tokio::test]
async fn live_router_positions_return_empty_when_all_routes_fail() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert("gate".into(), position_adapter("gate", PositionRead::Error));
    let router = LiveVenueRouter::new(routes);

    let rows = router.get_positions(None).await.unwrap_or_default();

    assert!(rows.is_empty());
}

#[tokio::test]
async fn live_router_positions_record_failed_routes_to_sink() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        position_adapter("binance", PositionRead::One),
    );
    routes.insert("gate".into(), position_adapter("gate", PositionRead::Error));
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let rows = router.get_positions(None).await.unwrap_or_default();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "binance");
    assert_eq!(sink.venues("positions"), vec!["gate"]);

    let failures = sink.take("positions");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].venue, "gate");
    assert_eq!(failures[0].operation, "positions");
    assert!(sink.take("positions").is_empty());
}

#[tokio::test]
async fn live_router_positions_clear_sink_when_all_routes_succeed() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        position_adapter("binance", PositionRead::One),
    );
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let _ = router.get_positions(None).await;

    assert!(sink.take("positions").is_empty());
}
