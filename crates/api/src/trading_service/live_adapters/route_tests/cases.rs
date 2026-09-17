use super::*;

mod funding_payments;
mod order_ids;
mod positions;

#[test]
fn live_router_routes_exact_builder_venue_and_family_fallback() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert("binance".into(), named("binance"));
    routes.insert("hyperliquid".into(), named("hyperliquid"));
    routes.insert("hyperliquid:xyz".into(), named("hyperliquid:xyz"));
    let router = LiveVenueRouter::new(routes);

    assert_eq!(
        route_name(&router, "hyperliquid:xyz").ok(),
        Some("hyperliquid:xyz")
    );
    assert_eq!(route_name(&router, "binance:um").ok(), Some("binance"));
    assert!(matches!(
        route_name(&router, "kucoin"),
        Err(ExchangeError::UnsupportedSymbol(_))
    ));
    assert!(matches!(
        route_name(&router, "hyperliquid:unknown"),
        Err(ExchangeError::UnsupportedSymbol(_))
    ));
    assert!(matches!(
        route_name(&router, "hyperliquid:spot"),
        Err(ExchangeError::UnsupportedSymbol(_))
    ));
}

#[test]
fn live_router_exchange_capabilities_are_route_scoped() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert("binance".into(), adapter_with_market("binance", true));
    routes.insert("gate".into(), adapter_with_market("gate", false));
    let router = LiveVenueRouter::new(routes);

    let aggregate = router.capabilities();
    let gate = router.exchange_capabilities("gate").ok();
    let binance_um = router.exchange_capabilities("binance:um").ok();

    assert!(aggregate.supports_market_orders);
    assert_eq!(
        gate.map(|capabilities| capabilities.supports_market_orders),
        Some(false)
    );
    assert_eq!(
        binance_um.map(|capabilities| capabilities.supports_market_orders),
        Some(true)
    );
}

#[test]
fn settings_capability_rows_cover_all_venues_without_credentials() {
    let rows = capability_rows_from_credentials(&AdapterCredentials::default());

    assert_eq!(rows.len(), exchange::LIVE_VENUE_FAMILIES.len());
    assert!(rows.iter().all(|row| !row.credentials_available));
    assert!(rows.iter().all(|row| row.matrix.orders.len() == 3));
    assert!(rows
        .iter()
        .all(|row| row.matrix.finality.has_confirmed_path()));
}

#[tokio::test]
async fn live_router_account_mode_is_route_scoped() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert("kucoin".into(), account_mode_adapter("kucoin", "hedge"));
    routes.insert("gate".into(), account_mode_adapter("gate", "one_way"));
    let router = LiveVenueRouter::new(routes);

    let kucoin = router
        .get_exchange_account_mode("kucoin:um")
        .await
        .unwrap_or(None)
        .unwrap_or_else(|| account_mode_info("missing", "missing"));

    assert_eq!(kucoin.venue, "kucoin");
    assert_eq!(kucoin.mode, "hedge");
}

#[tokio::test]
async fn live_router_order_preflight_is_route_scoped() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert("kucoin".into(), named("kucoin"));
    routes.insert("gate".into(), named("gate"));
    let router = LiveVenueRouter::new(routes);
    let mut intent = order_intent("kucoin");

    assert!(router.preflight_order("kucoin", &intent).await.is_ok());

    intent.exchange = "gate".into();
    let result = router.preflight_order("kucoin", &intent).await;
    assert!(result.is_err());
    assert!(result
        .err()
        .map(|error| error.to_string().contains("cannot preflight gate"))
        .unwrap_or(false));
}

#[tokio::test]
async fn live_router_open_orders_record_failed_routes_to_sink() {
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
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let rows = router.get_open_orders(None).await.unwrap_or_default();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "binance");

    let failures = sink.take("open_orders");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].venue, "gate");
    assert_eq!(failures[0].operation, "open_orders");
    assert!(sink.take("open_orders").is_empty());
}

#[tokio::test]
async fn live_router_open_orders_clear_sink_when_all_routes_succeed() {
    let reads = Arc::new(AtomicUsize::new(0));
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        order_adapter("binance", OrderRead::One, Arc::clone(&reads)),
    );
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let _ = router.get_open_orders(None).await;

    assert!(sink.take("open_orders").is_empty());
}

#[tokio::test]
async fn live_router_scoped_open_orders_skip_unrequested_routes() {
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
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let rows = router
        .open_orders_for_venues(&["binance".to_owned()], None)
        .await
        .unwrap_or_default();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "binance");
    assert!(sink.take("open_orders").is_empty());
}

#[tokio::test]
async fn live_router_balances_record_failed_routes_to_sink() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        balance_adapter("binance", BalanceRead::One),
    );
    routes.insert("gate".into(), balance_adapter("gate", BalanceRead::Error));
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let rows = router.get_balances(None).await.unwrap_or_default();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].venue, "binance");

    let failures = sink.take("balances");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].venue, "gate");
    assert_eq!(failures[0].operation, "balances");
    assert!(sink.take("balances").is_empty());
}

#[tokio::test]
async fn live_router_balances_clear_sink_when_all_routes_succeed() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        balance_adapter("binance", BalanceRead::One),
    );
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let _ = router.get_balances(None).await;

    assert!(sink.take("balances").is_empty());
}

#[tokio::test]
async fn live_router_scoped_account_read_skips_unrequested_routes() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        balance_adapter("binance", BalanceRead::One),
    );
    routes.insert("gate".into(), balance_adapter("gate", BalanceRead::Error));
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let read = router
        .account_read_for_venues(&["binance".to_owned()], None)
        .await
        .unwrap_or_default();

    assert_eq!(read.balances.len(), 1);
    assert_eq!(read.balances[0].venue, "binance");
    assert!(sink.take("balances").is_empty());
}

#[tokio::test]
async fn live_router_scoped_positions_skip_unrequested_routes() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "binance".into(),
        position_adapter("binance", PositionRead::One),
    );
    routes.insert("gate".into(), position_adapter("gate", PositionRead::Error));
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let rows = router
        .positions_for_venues(&["binance".to_owned()], None)
        .await
        .unwrap_or_default();

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "binance");
    assert!(sink.take("positions").is_empty());
}

#[tokio::test]
async fn live_router_balances_keep_rows_and_record_inner_source_issue() {
    let mut routes: LiveRouteMap = BTreeMap::new();
    routes.insert(
        "hyperliquid".into(),
        balance_adapter("hyperliquid", BalanceRead::Partial),
    );
    let sink = Arc::new(RouteFailureSink::default());
    let router = LiveVenueRouter::with_failure_sink(routes, Arc::clone(&sink));

    let rows = router.get_balances(None).await.unwrap_or_default();
    let failures = sink.take("balances");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].venue, "hyperliquid");
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].venue, "hyperliquid:spot");
    assert_eq!(failures[0].operation, "spot_truth");
}
