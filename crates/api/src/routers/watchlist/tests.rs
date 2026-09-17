use super::*;
use axum::extract::{Path, State};
use std::time::Duration;

#[tokio::test]
async fn watchlist_create_and_delete_replay_without_duplicate_ws_side_effects() -> Result<(), String>
{
    let state = test_state().await?;
    let mut receiver = state.ws_hub().subscribe(realtime::channels::WATCHLIST);

    let first = create(State(state.clone()), HeaderMap::new(), Json(item()))
        .await
        .map_err(|error| format!("first create failed: {error}"))?;
    let replay = create(State(state.clone()), HeaderMap::new(), Json(item()))
        .await
        .map_err(|error| format!("create replay failed: {error}"))?;

    assert_eq!(first.0.items, replay.0.items);
    assert_eq!(first.0.items.len(), 1);
    assert!(receiver.recv().await.is_ok());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), receiver.recv())
            .await
            .is_err()
    );

    let id = first.0.items[0].id;
    let removed = remove(State(state.clone()), HeaderMap::new(), Path(id))
        .await
        .map_err(|error| format!("delete failed: {error}"))?;
    let delete_replay = remove(State(state.clone()), HeaderMap::new(), Path(id))
        .await
        .map_err(|error| format!("delete replay failed: {error}"))?;
    assert!(removed.0.items.is_empty());
    assert_eq!(removed.0, delete_replay.0);
    assert!(receiver.recv().await.is_ok());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), receiver.recv())
            .await
            .is_err()
    );
    Ok(())
}

#[tokio::test]
async fn watchlist_delete_cascades_rule_and_cooldown_atomically() -> Result<(), String> {
    let state = test_state().await?;
    state.watchlist().write().await.push(stored_item(7));
    state.alert_rules().write().await.push(stored_rule(9, 7));
    state.alert_cooldowns().insert(9, 42);

    let _ = remove(State(state.clone()), HeaderMap::new(), Path(7))
        .await
        .map_err(|error| format!("delete failed: {error}"))?;

    assert!(state.watchlist().read().await.is_empty());
    assert!(state.alert_rules().read().await.is_empty());
    assert!(!state.alert_cooldowns().contains_key(&9));
    Ok(())
}

#[tokio::test]
async fn volatile_watchlist_alert_state_restarts_empty_with_explicit_runtime_meta(
) -> Result<(), String> {
    let before_restart = test_state().await?;
    before_restart
        .watchlist()
        .write()
        .await
        .push(stored_item(7));
    before_restart
        .alert_rules()
        .write()
        .await
        .push(stored_rule(9, 7));
    before_restart.alert_cooldowns().insert(9, 42);

    let after_restart = test_state().await?;
    let watchlist = super::list(State(after_restart.clone())).await.0;
    let alerts =
        realtime::alerts::alert_rules_envelope(after_restart.alert_rules().read().await.clone());

    assert!(watchlist.items.is_empty());
    assert!(alerts.rules.is_empty());
    assert!(after_restart.alert_cooldowns().is_empty());
    assert_eq!(watchlist.runtime.persistence, "memory");
    assert!(watchlist.runtime.volatile);
    assert_eq!(watchlist.runtime.restart_behavior, "cleared_on_restart");
    assert_eq!(watchlist.runtime, alerts.runtime);
    Ok(())
}

#[tokio::test]
async fn unavailable_durable_storage_rolls_back_watchlist_mutation() -> Result<(), String> {
    let path = std::env::temp_dir().join(format!(
        "crossline-watchlist-storage-directory-{}-{}",
        std::process::id(),
        common::time::now_ms()
    ));
    std::fs::create_dir_all(&path).map_err(|error| error.to_string())?;
    let mut config = common::config::AppConfig::default();
    config.api_surface.watchlist_alerts = true;
    config.storage.watchlist_alerts_path = Some(path.display().to_string());
    let state = AppState::new(config)
        .await
        .map_err(|error| format!("test state failed: {error}"))?;

    let Err(error) = create(State(state.clone()), HeaderMap::new(), Json(item())).await else {
        return Err("directory-backed SQLite path accepted mutation".to_owned());
    };

    assert_eq!(error.status(), axum::http::StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.code(), "WATCHLIST_STORAGE_UNAVAILABLE");
    assert!(state.watchlist().read().await.is_empty());
    assert_eq!(
        state.watchlist_alert_store().health().status,
        shared_types::WatchlistStorageStatus::Degraded
    );
    std::fs::remove_dir(&path).map_err(|error| error.to_string())?;
    Ok(())
}

async fn test_state() -> Result<AppState, String> {
    AppState::new(common::config::AppConfig::default())
        .await
        .map_err(|error| format!("test state failed: {error}"))
}

fn item() -> WatchlistItem {
    WatchlistItem {
        id: 0,
        symbol: " btc-usdt ".into(),
        venue_long: Some("BINANCE".into()),
        venue_short: Some("okx".into()),
        min_net_yield: Some(0.1),
        min_volume_24h: Some(1000.0),
        enabled: true,
        created_at_ms: 0,
        persistence: shared_types::WatchlistPersistence::default(),
        runtime: shared_types::WatchlistItemRuntime::default(),
    }
}

fn stored_item(id: i64) -> WatchlistItem {
    WatchlistItem { id, ..item() }
}

fn stored_rule(id: i64, watchlist_id: i64) -> shared_types::AlertRule {
    shared_types::AlertRule {
        id,
        watchlist_id,
        channel: shared_types::AlertChannel::Toast,
        cooldown_secs: 300,
        enabled: true,
        created_at_ms: 1,
        persistence: shared_types::WatchlistPersistence::default(),
        delivery: shared_types::AlertDeliveryState::configured(&shared_types::AlertChannel::Toast),
        runtime: shared_types::AlertRuleRuntime::configured(
            &shared_types::AlertChannel::Toast,
            true,
            2,
        ),
    }
}
