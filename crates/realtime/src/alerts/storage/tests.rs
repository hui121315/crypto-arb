use super::*;
use crate::alerts::{
    AlertChannel, AlertDeliveryState, WatchlistPersistence, WatchlistPrewarmStatus,
};

#[tokio::test]
async fn sqlite_snapshot_restores_config_delivery_and_cooldown_without_stale_prewarm(
) -> Result<(), String> {
    let path = temp_path("roundtrip");
    let first = WatchlistAlertStore::initialize(Some(path.clone()), true).await;
    let mut item = watchlist_item();
    item.runtime.status = WatchlistPrewarmStatus::Fresh;
    let mut rule = alert_rule();
    rule.delivery.last_fired_at_ms = Some(common::time::now_ms());
    rule.delivery.last_delivery_status = AlertDeliveryStatus::Queued;
    rule.runtime.last_triggered_at_ms = rule.delivery.last_fired_at_ms;
    rule.runtime.next_eligible_at_ms = Some(common::time::now_ms() + 60_000);
    rule.runtime.trigger_count = 2;

    assert_eq!(
        first.store.persist_snapshot(&[item], &[rule]).await?,
        WatchlistPersistStatus::Persisted
    );
    drop(first);

    let restored = WatchlistAlertStore::initialize(Some(path.clone()), true).await;
    assert_eq!(restored.watchlist.len(), 1);
    assert_eq!(restored.alert_rules.len(), 1);
    assert_eq!(
        restored.watchlist[0].runtime.status,
        WatchlistPrewarmStatus::Idle
    );
    assert_eq!(
        restored.watchlist[0].persistence.persist_status,
        WatchlistPersistStatus::Persisted
    );
    assert_eq!(
        restored.alert_rules[0].runtime.status,
        AlertRuleRuntimeStatus::Cooldown
    );
    assert_eq!(restored.alert_rules[0].runtime.trigger_count, 2);
    assert_eq!(restored.store.health().revision, 1);
    assert_eq!(
        restored.store.health().status,
        WatchlistStorageStatus::Ready
    );
    cleanup(&path);
    Ok(())
}

#[tokio::test]
async fn corrupted_snapshot_hash_blocks_restore_and_future_mutation() -> Result<(), String> {
    let path = temp_path("corrupt");
    let first = WatchlistAlertStore::initialize(Some(path.clone()), true).await;
    first
        .store
        .persist_snapshot(&[watchlist_item()], &[alert_rule()])
        .await?;
    let conn = Connection::open(&path).map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE watchlist_alert_snapshots SET payload_json = payload_json || 'tampered'",
        [],
    )
    .map_err(|error| error.to_string())?;
    drop(conn);
    drop(first);

    let restored = WatchlistAlertStore::initialize(Some(path.clone()), true).await;
    assert!(restored.watchlist.is_empty());
    assert_eq!(
        restored.store.health().status,
        WatchlistStorageStatus::Degraded
    );
    assert_eq!(
        restored
            .store
            .health()
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("WATCHLIST_STORAGE_LOAD_FAILED")
    );
    assert!(restored
        .store
        .persist_snapshot(&[watchlist_item()], &[alert_rule()])
        .await
        .is_err());
    cleanup(&path);
    Ok(())
}

#[tokio::test]
async fn schema_identity_drift_blocks_restore_and_future_mutation() -> Result<(), String> {
    let path = temp_path("schema-drift");
    let first = WatchlistAlertStore::initialize(Some(path.clone()), true).await;
    first
        .store
        .persist_snapshot(&[watchlist_item()], &[alert_rule()])
        .await?;
    let conn = Connection::open(&path).map_err(|error| error.to_string())?;
    conn.execute(
        "UPDATE watchlist_alert_meta SET value = 'tampered' WHERE key = 'schema_hash'",
        [],
    )
    .map_err(|error| error.to_string())?;
    drop(conn);
    drop(first);

    let restored = WatchlistAlertStore::initialize(Some(path.clone()), true).await;
    assert!(restored.watchlist.is_empty());
    assert_eq!(
        restored.store.health().status,
        WatchlistStorageStatus::Degraded
    );
    assert!(restored
        .store
        .persist_snapshot(&[watchlist_item()], &[alert_rule()])
        .await
        .is_err());
    cleanup(&path);
    Ok(())
}

fn watchlist_item() -> WatchlistItem {
    let now_ms = common::time::now_ms();
    WatchlistItem {
        id: 1,
        symbol: "BTC-USDT".to_owned(),
        venue_long: Some("binance".to_owned()),
        venue_short: Some("okx".to_owned()),
        min_net_yield: Some(0.1),
        min_volume_24h: Some(1000.0),
        enabled: true,
        created_at_ms: now_ms,
        persistence: WatchlistPersistence::pending("operator", now_ms),
        runtime: WatchlistItemRuntime::default(),
    }
}

fn alert_rule() -> AlertRule {
    let now_ms = common::time::now_ms();
    AlertRule {
        id: 1,
        watchlist_id: 1,
        channel: AlertChannel::Toast,
        cooldown_secs: 300,
        enabled: true,
        created_at_ms: now_ms,
        persistence: WatchlistPersistence::pending("operator", now_ms),
        delivery: AlertDeliveryState::configured(&AlertChannel::Toast),
        runtime: AlertRuleRuntime::configured(&AlertChannel::Toast, true, 2),
    }
}

fn temp_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-watchlist-alerts-{label}-{}-{}.sqlite",
        std::process::id(),
        common::time::now_ms()
    ))
}

fn cleanup(path: &Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}
