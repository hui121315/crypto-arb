use super::*;

#[test]
fn credential_invalidation_advances_epoch_and_clears_account_backoff() {
    let service = TradingService::new_mock();
    let initial_epoch = service.account_cache_epoch();
    service.record_balance_fetch_error(
        &["binance".to_owned()],
        &exchange::ExchangeError::RateLimited {
            retry_after_secs: 2,
        },
        common::time::now_ms(),
    );
    assert!(service
        .balance_backoff_error("binance", common::time::now_ms())
        .is_some());
    service.open_order_fetch_backoffs.insert(
        "binance".to_owned(),
        BalanceFetchBackoff {
            retry_until_ms: common::time::now_ms() + 10_000,
            error: CachedExchangeError::Timeout { seconds: 3 },
        },
    );

    service.invalidate_account_credentials();

    assert_eq!(service.account_cache_epoch(), initial_epoch + 1);
    assert!(service.balance_cache_health().is_empty());
    assert!(service.position_cache_health().is_empty());
    assert!(service
        .balance_backoff_error("binance", common::time::now_ms())
        .is_none());
    assert!(service.open_order_fetch_backoffs.is_empty());
}

#[test]
fn account_reader_refresh_replaces_routes_and_invalidates_account_state() -> anyhow::Result<()> {
    let service = TradingService::new_mock();
    let initial_epoch = service.account_cache_epoch();
    service.asset_valuations.insert(
        ("bitget".to_owned(), "BTC".to_owned()),
        VenueAssetValuation {
            venue: "bitget".to_owned(),
            currency: "BTC".to_owned(),
            usd_value: 1.0,
            source: "test".to_owned(),
            observed_at_ms: 1,
        },
    );

    service.refresh_account_reader(AdapterCredentials {
        binance_live: Some(("key".to_owned(), "secret".to_owned())),
        ..AdapterCredentials::default()
    })?;

    assert_eq!(service.account_reader_venues(), vec!["binance"]);
    assert_eq!(service.account_cache_epoch(), initial_epoch + 1);
    assert!(service.asset_valuations.is_empty());
    Ok(())
}

#[test]
fn live_credential_refresh_replaces_read_and_write_routes_together() -> anyhow::Result<()> {
    let service = TradingService::new_mock();
    service.try_select_adapter(
        LIVE_ROUTER_ADAPTER_ID,
        AdapterCredentials {
            binance_live: Some(("old-key".to_owned(), "old-secret".to_owned())),
            bitget_live: Some((
                "bitget-key".to_owned(),
                "bitget-secret".to_owned(),
                "bitget-pass".to_owned(),
            )),
            ..AdapterCredentials::default()
        },
    )?;

    service.refresh_account_reader(AdapterCredentials {
        binance_live: Some(("new-key".to_owned(), "new-secret".to_owned())),
        ..AdapterCredentials::default()
    })?;

    assert_eq!(service.account_reader_venues(), vec!["binance"]);
    assert_eq!(
        service.risk_config().allowed_exchanges,
        BTreeSet::from(["binance".to_owned()])
    );
    assert_eq!(service.adapter_name(), LIVE_ROUTER_ADAPTER_ID);
    Ok(())
}

#[test]
fn live_credential_refresh_does_not_expand_operator_execution_scope() -> anyhow::Result<()> {
    let service = TradingService::new_mock();
    let credentials = AdapterCredentials {
        binance_live: Some(("binance-key".to_owned(), "binance-secret".to_owned())),
        bitget_live: Some((
            "bitget-key".to_owned(),
            "bitget-secret".to_owned(),
            "bitget-pass".to_owned(),
        )),
        ..AdapterCredentials::default()
    };
    service.try_select_adapter(LIVE_ROUTER_ADAPTER_ID, credentials.clone())?;
    service.update_risk_config(|risk| {
        risk.allowed_exchanges = BTreeSet::from(["bitget".to_owned()]);
    });

    service.refresh_account_reader(credentials)?;

    assert_eq!(
        service.risk_config().allowed_exchanges,
        BTreeSet::from(["bitget".to_owned()])
    );
    assert_eq!(service.adapter_name(), LIVE_ROUTER_ADAPTER_ID);
    Ok(())
}

#[test]
fn removing_all_credentials_disables_active_live_router() -> anyhow::Result<()> {
    let service = TradingService::new_mock();
    service.try_select_adapter(
        LIVE_ROUTER_ADAPTER_ID,
        AdapterCredentials {
            binance_live: Some(("key".to_owned(), "secret".to_owned())),
            ..AdapterCredentials::default()
        },
    )?;

    service.refresh_account_reader(AdapterCredentials::default())?;

    assert_eq!(service.adapter_name(), "mock");
    assert!(service.account_reader_venues().is_empty());
    assert!(!service.risk_config().live_trading_enabled);
    assert!(service.risk_config().allowed_exchanges.is_empty());
    Ok(())
}

#[test]
fn order_ack_invalidates_only_open_orders() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service
        .position_cache
        .replace("binance", epoch, vec![position("binance", "SOL")]);
    service
        .position_cache
        .replace("bitget", epoch, vec![position("bitget", "ETH")]);
    service
        .balance_cache
        .replace("binance", epoch, vec![balance("binance", "USDT")]);

    service.mark_open_order_cache_stale("binance");

    let snapshots = service.position_cache_health();
    assert!(snapshots.iter().any(|row| {
        row.venue == "binance" && row.quality == AccountCacheQuality::Fresh && row.rows == 1
    }));
    assert!(snapshots.iter().any(|row| {
        row.venue == "bitget" && row.quality == AccountCacheQuality::Fresh && row.rows == 1
    }));
    assert!(service.balance_cache_health().iter().any(|row| {
        row.venue == "binance" && row.quality == AccountCacheQuality::Fresh && row.rows == 1
    }));
}

#[test]
fn fill_invalidates_only_its_venue_and_retains_bounded_rows() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service
        .position_cache
        .replace("binance", epoch, vec![position("binance", "SOL")]);
    service
        .position_cache
        .replace("bitget", epoch, vec![position("bitget", "ETH")]);

    service.mark_filled_account_cache_stale("binance", "fill_event");

    let snapshots = service.position_cache_health();
    assert!(snapshots.iter().any(|row| {
        row.venue == "binance" && row.quality == AccountCacheQuality::Stale && row.rows == 1
    }));
    assert!(snapshots.iter().any(|row| {
        row.venue == "bitget" && row.quality == AccountCacheQuality::Fresh && row.rows == 1
    }));
}

#[test]
fn private_account_recovery_requests_only_the_still_missing_scope() {
    let service = TradingService::new_mock();
    let now_ms = common::time::now_ms();
    let epoch = service.account_cache_epoch();
    service
        .position_cache
        .replace("binance", epoch, vec![position("binance", "SOL")]);

    assert_eq!(
        service.unresolved_private_account_scope("binance", PrivateAccountScope::All, now_ms),
        Some(PrivateAccountScope::Balances)
    );

    service
        .balance_cache
        .replace("binance", epoch, vec![balance("binance", "USDT")]);
    assert_eq!(
        service.unresolved_private_account_scope("binance", PrivateAccountScope::All, now_ms),
        None
    );

    service.balance_cache.invalidate("binance");
    assert_eq!(
        service.unresolved_private_account_scope("binance", PrivateAccountScope::All, now_ms),
        Some(PrivateAccountScope::Balances)
    );
}

#[test]
fn account_invalidation_requires_fill_evidence() {
    assert!(!filled_quantity_changes_account_state(None));
    assert!(!filled_quantity_changes_account_state(Some(0.0)));
    assert!(!filled_quantity_changes_account_state(Some(f64::NAN)));
    assert!(filled_quantity_changes_account_state(Some(0.25)));
}

#[test]
fn hyperliquid_root_private_ws_activity_covers_builder_cache_routes() {
    let venues = expand_private_ws_cache_venues(
        "hyperliquid",
        vec![
            "binance".to_owned(),
            "hyperliquid:xyz".to_owned(),
            "hyperliquid:vntl".to_owned(),
        ],
    );

    assert_eq!(
        venues,
        vec![
            "hyperliquid".to_owned(),
            "hyperliquid:vntl".to_owned(),
            "hyperliquid:xyz".to_owned(),
        ]
    );
    assert_eq!(
        expand_private_ws_cache_venues("hyperliquid:xyz", vec!["hyperliquid".to_owned()]),
        vec!["hyperliquid:xyz".to_owned()]
    );
}

fn position(venue: &str, symbol: &str) -> PositionInfo {
    PositionInfo {
        symbol: symbol.to_owned(),
        exchange: venue.to_owned(),
        side: "long".to_owned(),
        quantity: 1.0,
        entry_price: 1.0,
        mark_price: 1.0,
        unrealized_pnl: 0.0,
        leverage: 1.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        paired_with: None,
        margin: 1.0,
        maintenance_margin_ratio: 0.0,
        position_mode: None,
        margin_mode: None,
        risk_rate: None,
        available_position: None,
        frozen_position: None,
    }
}

fn balance(venue: &str, currency: &str) -> VenueBalanceInfo {
    VenueBalanceInfo {
        venue: venue.to_owned(),
        currency: currency.to_owned(),
        total: 100.0,
        available: 100.0,
        frozen: 0.0,
        unrealized_pnl: 0.0,
    }
}
