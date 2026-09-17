use super::adapters::balance_row;
use super::support::*;
use super::*;
use crate::trading_service::balances::BALANCE_ROUTE_OPERATION;

#[tokio::test]
async fn list_positions_under_mock_returns_empty() {
    let service = TradingService::new_mock();
    let positions = must_ok(
        service.list_positions().await,
        "mock adapter never errors on get_positions",
    );
    assert!(positions.is_empty());
}

#[tokio::test]
async fn list_balances_under_mock_returns_margin_cash() {
    let service = TradingService::new_mock();
    let balances = must_ok(
        service
            .list_configured_balances(AdapterCredentials::default())
            .await,
        "mock adapter always exposes a dry-run balance",
    );

    assert_eq!(balances.len(), 1);
    assert_eq!(balances[0].venue, "mock");
    assert_eq!(balances[0].currency, "USDT");
    assert!(balances[0].available > 0.0);
}

#[tokio::test]
async fn list_scoped_balances_fetches_only_requested_venues() {
    let (service, adapter) = service_with_balance_probe_adapter();

    let balances = must_ok(
        service
            .list_scoped_balances(&[
                "binance".to_owned(),
                "gate".to_owned(),
                "Binance".to_owned(),
            ])
            .await,
        "scoped balances",
    );

    assert_eq!(
        adapter.balance_queries(),
        vec!["binance".to_owned(), "gate".to_owned()]
    );
    assert_eq!(balances.len(), 2);
    assert!(balances.iter().any(|row| row.venue == "binance"));
    assert!(balances.iter().any(|row| row.venue == "gate"));
}

#[tokio::test]
async fn list_scoped_balances_coalesces_same_venue_miss() {
    let (service, adapter) = service_with_slow_balance_probe_adapter(25);

    let first_venues = ["binance".to_owned()];
    let second_venues = ["Binance".to_owned()];
    let first = service.list_scoped_balances(&first_venues);
    let second = service.list_scoped_balances(&second_venues);
    let (first, second) = tokio::join!(first, second);
    let first = must_ok(first, "first scoped balances");
    let second = must_ok(second, "second scoped balances");

    assert_eq!(adapter.balance_queries(), vec!["binance".to_owned()]);
    assert_eq!(first.first().map(|row| row.available), Some(1_000.0));
    assert_eq!(second.first().map(|row| row.available), Some(1_000.0));
}

#[tokio::test]
async fn list_scoped_balances_reuses_failed_miss_backoff() {
    let (service, adapter) = service_with_balance_probe_adapter();
    let venues = ["rate_limited".to_owned()];

    let first = must_err(
        service.list_scoped_balances(&venues).await,
        "first scoped balance should fail",
    );
    let second = must_err(
        service.list_scoped_balances(&venues).await,
        "second scoped balance should fail from backoff",
    );

    assert!(matches!(
        first,
        ExchangeError::RateLimited {
            retry_after_secs: 2
        }
    ));
    assert!(matches!(
        second,
        ExchangeError::RateLimited {
            retry_after_secs: 2
        }
    ));
    assert_eq!(adapter.balance_queries(), vec!["rate_limited".to_owned()]);
}

#[tokio::test]
async fn list_configured_balance_venues_coalesces_same_missing_set() {
    let (service, adapter) = service_with_slow_balance_probe_adapter(25);

    let first_venues = ["binance".to_owned(), "gate".to_owned()];
    let second_venues = ["Gate".to_owned(), "Binance".to_owned()];
    let first = service.list_configured_balance_venues_from_adapter_for_test(&first_venues);
    let second = service.list_configured_balance_venues_from_adapter_for_test(&second_venues);
    let (first, second) = tokio::join!(first, second);
    let first = must_ok(first, "first configured balances");
    let second = must_ok(second, "second configured balances");

    assert_eq!(adapter.balance_queries(), vec!["*".to_owned()]);
    assert_eq!(first.len(), 2);
    assert_eq!(second.len(), 2);
    assert!(first.iter().any(|row| row.venue == "binance"));
    assert!(second.iter().any(|row| row.venue == "gate"));
}

#[tokio::test]
async fn list_configured_balance_venues_reuses_failed_miss_backoff() {
    let (service, adapter) = service_with_full_rate_limited_balance_probe_adapter(25, 3);

    let venues = ["binance".to_owned(), "gate".to_owned()];
    let first = must_err(
        service
            .list_configured_balance_venues_from_adapter_for_test(&venues)
            .await,
        "first configured balances should fail",
    );
    let second = must_err(
        service
            .list_configured_balance_venues_from_adapter_for_test(&venues)
            .await,
        "second configured balances should fail from backoff",
    );

    assert!(matches!(
        first,
        ExchangeError::RateLimited {
            retry_after_secs: 3
        }
    ));
    assert!(matches!(
        second,
        ExchangeError::RateLimited {
            retry_after_secs: 3
        }
    ));
    assert_eq!(adapter.balance_queries(), vec!["*".to_owned()]);
}

#[tokio::test]
async fn list_configured_balances_includes_fresh_hyperliquid_child_cache() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    service.balance_cache.replace(
        "hyperliquid",
        epoch,
        vec![balance_row("hyperliquid", 100.0)],
    );
    service.balance_cache.replace(
        "hyperliquid:xyz",
        epoch,
        vec![balance_row("hyperliquid:xyz", 25.0)],
    );
    service.balance_cache.replace(
        "hyperliquid:spot",
        epoch,
        vec![balance_row("hyperliquid:spot", 5.0)],
    );
    let credentials = AdapterCredentials {
        hyperliquid_live: Some(HyperliquidAdapterCredentials {
            account_address: "0x0000000000000000000000000000000000000001".into(),
            private_key: "0101010101010101010101010101010101010101010101010101010101010101".into(),
            vault_address: None,
        }),
        ..AdapterCredentials::default()
    };

    let balances = must_ok(
        service.list_configured_balances(credentials).await,
        "fresh hyperliquid child cache should satisfy configured balance read",
    );

    assert_eq!(balances.len(), 3);
    assert!(balances.iter().any(|row| row.venue == "hyperliquid"));
    assert!(balances.iter().any(|row| row.venue == "hyperliquid:xyz"));
    assert!(balances.iter().any(|row| row.venue == "hyperliquid:spot"));
}

#[tokio::test]
async fn refresh_scoped_balances_preserves_unrelated_cache_entries() {
    let (service, adapter) = service_with_balance_probe_adapter();
    let epoch = service.account_cache_epoch();
    service
        .balance_cache
        .replace("okx", epoch, vec![balance_row("okx", 500.0)]);
    service
        .balance_cache
        .replace("binance", epoch, vec![balance_row("binance", 10.0)]);

    let balances = must_ok(
        service
            .refresh_scoped_balances(&["binance".to_owned()])
            .await,
        "refresh scoped balances",
    );
    let okx = must_some(
        service
            .balance_cache
            .fresh("okx", epoch, common::time::now_ms()),
        "okx cache should remain",
    );

    assert_eq!(adapter.balance_queries(), vec!["binance".to_owned()]);
    assert_eq!(balances.first().map(|row| row.available), Some(1_000.0));
    assert_eq!(okx.first().map(|row| row.available), Some(500.0));
}

#[test]
fn partial_balance_refresh_keeps_failed_venue_stale_and_seeds_success() {
    let service = TradingService::new_mock();
    let epoch = service.account_cache_epoch();
    let now_ms = common::time::now_ms();
    service.balance_cache.replace_at(
        "binance",
        epoch,
        vec![balance_row("binance", 250.0)],
        now_ms - BALANCE_CACHE_TTL_MS - 1,
    );
    let fetched = vec![balance_row("bitget", 500.0)];
    let missing = vec!["binance".to_owned(), "bitget".to_owned()];
    let failed = HashSet::from(["binance".to_owned()]);
    let mut merged = Vec::new();

    service.merge_failed_balance_stale(epoch, now_ms, &missing, &failed, &mut merged);
    service.seed_balance_cache_from_full_read(epoch, &missing, &fetched, &failed);

    assert_eq!(merged.first().map(|row| row.available), Some(250.0));
    assert!(service
        .balance_cache
        .fresh("binance", epoch, now_ms)
        .is_none());
    assert_eq!(
        service
            .balance_cache
            .stale("binance", epoch, now_ms)
            .and_then(|rows| rows.first().map(|row| row.available)),
        Some(250.0)
    );
    assert_eq!(
        service
            .balance_cache
            .fresh("bitget", epoch, common::time::now_ms())
            .and_then(|rows| rows.first().map(|row| row.available)),
        Some(500.0)
    );
}

mod backoff;
