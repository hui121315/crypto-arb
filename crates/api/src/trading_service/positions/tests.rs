use super::*;

#[test]
fn partial_refresh_updates_success_and_keeps_failed_venue_stale() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service
        .position_cache
        .replace("binance", epoch, vec![position("binance", 1.0)]);
    service
        .position_cache
        .replace("gate", epoch, vec![position("gate", 2.0)]);
    service.route_failures.record(
        POSITION_OPERATION,
        vec![RouteFailure::new(
            "gate".to_owned(),
            POSITION_OPERATION,
            exchange::ExchangeError::RateLimited {
                retry_after_secs: 3,
            },
        )],
    );

    let rows = service.merge_position_refresh(
        epoch,
        &["binance".to_owned(), "gate".to_owned()],
        common::time::now_ms(),
        vec![position("binance", 3.0)],
    );

    assert!(rows
        .iter()
        .any(|row| row.exchange == "binance" && row.quantity == 3.0));
    assert!(rows
        .iter()
        .any(|row| row.exchange == "gate" && row.quantity == 2.0));
    let snapshots = service.position_cache_health();
    assert!(snapshots
        .iter()
        .any(|row| { row.venue == "binance" && row.quality == AccountCacheQuality::Fresh }));
    assert!(snapshots
        .iter()
        .any(|row| row.venue == "gate" && row.quality == AccountCacheQuality::Stale));
    assert_eq!(service.take_route_failures(POSITION_OPERATION).len(), 1);
}

#[test]
fn failed_venue_without_stale_rows_is_not_seeded_as_fresh_empty() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service.route_failures.record(
        POSITION_OPERATION,
        vec![RouteFailure::new(
            "gate".to_owned(),
            POSITION_OPERATION,
            exchange::ExchangeError::Parse("gate schema drift".to_owned()),
        )],
    );

    let rows = service.merge_position_refresh(
        epoch,
        &["binance".to_owned(), "gate".to_owned()],
        common::time::now_ms(),
        vec![position("binance", 3.0)],
    );

    assert_eq!(rows.len(), 1);
    assert!(!service
        .position_cache_health()
        .iter()
        .any(|row| row.venue == "gate"));
    assert!(service
        .position_cache
        .fresh_all(
            &["binance".to_owned(), "gate".to_owned()],
            epoch,
            common::time::now_ms(),
        )
        .is_none());
}

#[test]
fn paper_mode_reads_fresh_private_ws_position_venues() {
    let service = TradingService::new_mock();
    service.position_cache.replace(
        "bitget",
        service.account_cache_epoch(),
        vec![position("bitget", 1.0)],
    );

    assert_eq!(service.position_cache_venues(), vec!["bitget"]);
}

#[test]
fn credential_reader_venues_override_paper_risk_routes() -> anyhow::Result<()> {
    let service = TradingService::new_mock();
    service.initialize_account_reader(AdapterCredentials {
        binance_live: Some(("key".to_owned(), "secret".to_owned())),
        ..AdapterCredentials::default()
    })?;

    assert_eq!(service.position_cache_venues(), vec!["binance"]);
    Ok(())
}

#[test]
fn position_failure_backoff_is_scoped_and_expires() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    service
        .position_fetch_backoffs
        .insert("gate".to_owned(), timeout_backoff(now_ms + 10_000));

    assert!(service.position_backoff_error("gate", now_ms).is_some());
    assert!(service.position_backoff_error("binance", now_ms).is_none());
    assert!(service
        .position_backoff_error("gate", now_ms + 10_001)
        .is_none());
    assert!(!service.position_fetch_backoffs.contains_key("gate"));
}

#[test]
fn position_backoff_serves_stale_and_blocks_missing_evidence() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    let now_ms = common::time::now_ms();
    service
        .position_cache
        .replace("bitget", epoch, vec![position("bitget", 1.0)]);
    service
        .position_fetch_backoffs
        .insert("bitget".to_owned(), timeout_backoff(now_ms + 10_000));
    service
        .position_fetch_backoffs
        .insert("gate".to_owned(), timeout_backoff(now_ms + 10_000));

    let (refresh, deferred, blocked_error) = service.partition_position_refresh(
        &["bitget".to_owned(), "gate".to_owned()],
        epoch,
        now_ms,
    );

    assert!(refresh.is_empty());
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].exchange, "bitget");
    assert!(matches!(
        blocked_error,
        Some(exchange::ExchangeError::Timeout { seconds: 3 })
    ));
}

#[test]
fn position_backoff_preserves_exact_route_error() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    let error = exchange::ExchangeError::Parse("schema drift".to_owned());

    service.record_position_fetch_error(&["gate".to_owned()], &error, now_ms);

    assert!(matches!(
        service.position_backoff_error("gate", now_ms),
        Some(exchange::ExchangeError::Parse(message)) if message == "schema drift"
    ));
}

#[test]
fn position_cache_read_identifies_only_missing_venues() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service
        .position_cache
        .replace("bitget", epoch, vec![position("bitget", 1.0)]);

    let (rows, missing) = service.read_position_cache(
        &["bitget".to_owned(), "gate".to_owned()],
        epoch,
        common::time::now_ms(),
    );

    assert_eq!(rows.len(), 1);
    assert_eq!(missing, vec!["gate"]);
}

#[test]
fn scoped_route_failures_leave_unrequested_venues_for_global_diagnostics() {
    let service = TradingService::new_mock();
    service.route_failures.record(
        POSITION_OPERATION,
        vec![
            RouteFailure::new(
                "bitget".to_owned(),
                POSITION_OPERATION,
                exchange::ExchangeError::Timeout { seconds: 3 },
            ),
            RouteFailure::new(
                "gate".to_owned(),
                POSITION_OPERATION,
                exchange::ExchangeError::Timeout { seconds: 3 },
            ),
        ],
    );

    let scoped = service.take_route_failures_for_venues(POSITION_OPERATION, &["bitget".to_owned()]);

    assert_eq!(scoped.len(), 1);
    assert_eq!(scoped[0].venue, "bitget");
    let retained = service.take_route_failures(POSITION_OPERATION);
    assert_eq!(retained.len(), 1);
    assert_eq!(retained[0].venue, "gate");
}

#[tokio::test]
async fn scoped_low_latency_positions_read_only_requested_cache() -> anyhow::Result<()> {
    let service = Arc::new(TradingService::new_mock());
    let epoch = service.account_cache_epoch();
    service
        .position_cache
        .replace("bitget", epoch, vec![position("bitget", 1.0)]);
    service
        .position_cache
        .replace("gate", epoch, vec![position("gate", 2.0)]);

    let rows = service
        .list_scoped_positions_low_latency(&["bitget".to_owned()])
        .await?;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "bitget");
    Ok(())
}

#[tokio::test]
async fn concurrent_refresh_serves_bounded_stale_without_waiting() -> anyhow::Result<()> {
    let service = Arc::new(TradingService::new_mock());
    service.initialize_account_reader(AdapterCredentials {
        binance_live: Some(("key".to_owned(), "secret".to_owned())),
        ..AdapterCredentials::default()
    })?;
    let epoch = service.account_cache_epoch();
    service
        .position_cache
        .replace("binance", epoch, vec![position("binance", 1.0)]);
    service.position_cache.invalidate("binance");
    let _guard = service.position_fetch_lock.lock().await;

    let rows = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        service.list_positions_low_latency(),
    )
    .await??;

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].exchange, "binance");
    Ok(())
}

fn position(venue: &str, quantity: f64) -> PositionInfo {
    PositionInfo {
        exchange: venue.to_owned(),
        symbol: "BTCUSDT".to_owned(),
        side: "long".to_owned(),
        quantity,
        entry_price: 100.0,
        mark_price: 101.0,
        unrealized_pnl: 1.0,
        leverage: 2.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 50.0,
        maintenance_margin_ratio: 0.01,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

fn timeout_backoff(retry_until_ms: i64) -> BalanceFetchBackoff {
    BalanceFetchBackoff {
        retry_until_ms,
        error: CachedExchangeError::Timeout { seconds: 3 },
    }
}
