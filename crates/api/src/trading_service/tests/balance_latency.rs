use super::adapters::balance_row;
use super::*;

#[tokio::test]
async fn low_latency_balance_read_serves_stale_while_refresh_is_busy() -> anyhow::Result<()> {
    let service = Arc::new(TradingService::new_mock());
    let credentials = AdapterCredentials {
        binance_live: Some(("key".to_owned(), "secret".to_owned())),
        ..AdapterCredentials::default()
    };
    let epoch = service.account_cache_epoch();
    service
        .balance_cache
        .replace("binance", epoch, vec![balance_row("binance", 250.0)]);
    service.balance_cache.invalidate("binance");
    let refresh_lock = service.balance_fetch_lock("binance");
    let _guard = refresh_lock.lock_owned().await;

    let rows = tokio::time::timeout(
        std::time::Duration::from_millis(50),
        service.list_configured_balances_low_latency(credentials),
    )
    .await??;

    assert_eq!(rows.first().map(|row| row.available), Some(250.0));
    Ok(())
}
