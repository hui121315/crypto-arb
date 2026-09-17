use super::*;
use std::time::Duration;

#[tokio::test]
async fn orderbook_guard_serializes_same_market_key() {
    let coordinator = Arc::new(RestBaselineCoordinator::default());
    let key = MarketKey::new("hyperliquid:xyz", "MU");
    let first = coordinator.orderbook_guard(&key).await;
    let cloned = Arc::clone(&coordinator);
    let same_key = key.clone();
    let mut waiter = tokio::spawn(async move { cloned.orderbook_guard(&same_key).await });

    let early = tokio::time::timeout(Duration::from_millis(20), &mut waiter).await;
    assert!(early.is_err());

    drop(first);
    let second = waiter.await.map_err(|err| err.to_string());
    assert!(second.is_ok());
    let Ok(second) = second else {
        return;
    };
    drop(second);
}

#[tokio::test]
async fn snapshot_guard_serializes_same_feed() {
    let coordinator = Arc::new(RestBaselineCoordinator::default());
    let first = coordinator.snapshot_guard(SnapshotFeed::PerpTickers).await;
    let cloned = Arc::clone(&coordinator);
    let mut waiter =
        tokio::spawn(async move { cloned.snapshot_guard(SnapshotFeed::PerpTickers).await });

    let early = tokio::time::timeout(Duration::from_millis(20), &mut waiter).await;
    assert!(early.is_err());

    drop(first);
    let second = waiter.await.map_err(|err| err.to_string());
    assert!(second.is_ok());
}

#[tokio::test]
async fn try_snapshot_guard_returns_none_when_feed_refresh_is_running() {
    let coordinator = RestBaselineCoordinator::default();
    let first = coordinator.snapshot_guard(SnapshotFeed::PerpTickers).await;

    assert!(coordinator
        .try_snapshot_guard(SnapshotFeed::PerpTickers)
        .is_none());

    drop(first);
    assert!(coordinator
        .try_snapshot_guard(SnapshotFeed::PerpTickers)
        .is_some());
}

#[tokio::test]
async fn stats_snapshot_reports_guard_keys_and_in_flight_counts() {
    let coordinator = RestBaselineCoordinator::default();
    let active_key = MarketKey::new("hyperliquid:xyz", "MU");
    let idle_key = MarketKey::new("binance", "BTCUSDT");
    let active_orderbook = coordinator.orderbook_guard(&active_key).await;
    let active_snapshot = coordinator.snapshot_guard(SnapshotFeed::PerpTickers).await;
    let idle_orderbook = coordinator.orderbook_guard(&idle_key).await;
    drop(idle_orderbook);

    let stats = coordinator.stats_snapshot();

    assert_eq!(stats.orderbook_keys, 2);
    assert_eq!(stats.orderbook_in_flight, 1);
    assert_eq!(stats.orderbook_wait_count_total, 0);
    assert_eq!(stats.orderbook_wait_ms_total, 0);
    assert_eq!(stats.orderbook_guard_evicted_total, 0);
    assert_eq!(stats.snapshot_feed_keys, 1);
    assert_eq!(stats.snapshot_feed_in_flight, 1);
    assert_eq!(stats.snapshot_wait_count_total, 0);
    assert_eq!(stats.snapshot_wait_ms_total, 0);

    drop(active_orderbook);
    drop(active_snapshot);
    let released = coordinator.stats_snapshot();
    assert_eq!(released.orderbook_in_flight, 0);
    assert_eq!(released.snapshot_feed_in_flight, 0);
}

#[tokio::test]
async fn stats_snapshot_records_wait_counters() -> Result<(), String> {
    let coordinator = Arc::new(RestBaselineCoordinator::default());
    let orderbook_key = MarketKey::new("hyperliquid:xyz", "MU");
    let orderbook_guard = coordinator.orderbook_guard(&orderbook_key).await;
    let mut orderbook_waiter = {
        let cloned = Arc::clone(&coordinator);
        let key = orderbook_key.clone();
        tokio::spawn(async move { cloned.orderbook_guard(&key).await })
    };

    let snapshot_guard = coordinator.snapshot_guard(SnapshotFeed::PerpTickers).await;
    let mut snapshot_waiter = {
        let cloned = Arc::clone(&coordinator);
        tokio::spawn(async move { cloned.snapshot_guard(SnapshotFeed::PerpTickers).await })
    };

    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut orderbook_waiter)
            .await
            .is_err()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), &mut snapshot_waiter)
            .await
            .is_err()
    );
    drop(orderbook_guard);
    drop(snapshot_guard);
    let waited_orderbook = orderbook_waiter.await.map_err(|err| err.to_string())?;
    let waited_snapshot = snapshot_waiter.await.map_err(|err| err.to_string())?;
    drop(waited_orderbook);
    drop(waited_snapshot);

    let stats = coordinator.stats_snapshot();
    assert_eq!(stats.orderbook_wait_count_total, 1);
    assert!(stats.orderbook_wait_ms_total > 0);
    assert_eq!(stats.snapshot_wait_count_total, 1);
    assert!(stats.snapshot_wait_ms_total > 0);
    Ok(())
}

#[tokio::test]
async fn prune_orderbook_guards_evicts_only_idle_unlocked_keys() {
    let coordinator = RestBaselineCoordinator::default();
    let idle_key = MarketKey::new("hyperliquid:xyz", "MU");
    let active_key = MarketKey::new("binance", "BTCUSDT");
    let idle_guard = coordinator.orderbook_guard(&idle_key).await;
    let active_guard = coordinator.orderbook_guard(&active_key).await;
    drop(idle_guard);
    let old_ms = common::time::now_ms().saturating_sub(ORDERBOOK_GUARD_TTL_MS + 1);
    coordinator.set_orderbook_guard_last_used_for_test(&idle_key, old_ms);
    coordinator.set_orderbook_guard_last_used_for_test(&active_key, old_ms);

    coordinator.prune_orderbook_guards(common::time::now_ms());
    let stats = coordinator.stats_snapshot();

    assert_eq!(stats.orderbook_keys, 1);
    assert_eq!(stats.orderbook_in_flight, 1);
    assert_eq!(stats.orderbook_guard_evicted_total, 1);

    drop(active_guard);
    coordinator.prune_orderbook_guards(common::time::now_ms());
    let released = coordinator.stats_snapshot();
    assert_eq!(released.orderbook_keys, 0);
    assert_eq!(released.orderbook_guard_evicted_total, 2);
}

#[tokio::test]
async fn prune_orderbook_guards_lru_cap_keeps_recent_and_active_keys() {
    let coordinator = RestBaselineCoordinator::default();
    let now_ms = common::time::now_ms();
    let active_key = MarketKey::new("binance", "BTCUSDT");
    let recent_key = MarketKey::new("okx", "ETHUSDT");
    let active_guard = coordinator.orderbook_guard(&active_key).await;
    let recent_guard = coordinator.orderbook_guard(&recent_key).await;
    drop(recent_guard);
    coordinator.set_orderbook_guard_last_used_for_test(&active_key, now_ms - 10_000);
    coordinator.set_orderbook_guard_last_used_for_test(&recent_key, now_ms);

    for index in 0_i64..5 {
        let key = MarketKey::new("mock", &format!("OLD{index}"));
        let guard = coordinator.orderbook_guard(&key).await;
        drop(guard);
        coordinator.set_orderbook_guard_last_used_for_test(&key, now_ms - 20_000 - index);
    }

    coordinator.prune_orderbook_guards_to_limit_for_test(2, now_ms);
    let stats = coordinator.stats_snapshot();

    assert_eq!(stats.orderbook_keys, 2);
    assert_eq!(stats.orderbook_guard_evicted_total, 5);
    assert!(coordinator.orderbook_locks.contains_key(&active_key));
    assert!(coordinator.orderbook_locks.contains_key(&recent_key));

    drop(active_guard);
}

#[tokio::test]
async fn lru_remove_rechecks_last_used_before_eviction() {
    let coordinator = RestBaselineCoordinator::default();
    let key = MarketKey::new("hyperliquid:xyz", "MU");
    let guard = coordinator.orderbook_guard(&key).await;
    drop(guard);
    let old_ms = common::time::now_ms().saturating_sub(10_000);
    coordinator.set_orderbook_guard_last_used_for_test(&key, old_ms);
    coordinator.set_orderbook_guard_last_used_for_test(&key, common::time::now_ms());

    let removed = coordinator.remove_orderbook_guard_if_idle(&key, old_ms, common::time::now_ms());

    assert!(!removed);
    assert_eq!(coordinator.stats_snapshot().orderbook_keys, 1);
}
