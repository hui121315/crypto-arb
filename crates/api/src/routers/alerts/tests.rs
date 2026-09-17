use super::*;
use axum::extract::{Path, State};
use std::time::Duration;

#[tokio::test]
async fn alert_rule_mutations_replay_without_duplicate_side_effects() -> Result<(), String> {
    let state = test_state().await?;
    state.watchlist().write().await.push(watchlist_item(7));
    let mut receiver = state.ws_hub().subscribe(realtime::channels::ALERTS);

    let first = create(State(state.clone()), HeaderMap::new(), Json(rule(7)))
        .await
        .map_err(|error| format!("first create failed: {error}"))?;
    let replay = create(State(state.clone()), HeaderMap::new(), Json(rule(7)))
        .await
        .map_err(|error| format!("create replay failed: {error}"))?;

    assert_eq!(first.0, replay.0);
    assert_eq!(first.0.rules.len(), 1);
    assert!(receiver.recv().await.is_ok());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), receiver.recv())
            .await
            .is_err()
    );

    let id = first.0.rules[0].id;
    let removed = remove(State(state.clone()), HeaderMap::new(), Path(id))
        .await
        .map_err(|error| format!("delete failed: {error}"))?;
    let delete_replay = remove(State(state.clone()), HeaderMap::new(), Path(id))
        .await
        .map_err(|error| format!("delete replay failed: {error}"))?;
    assert!(removed.0.rules.is_empty());
    assert_eq!(removed.0, delete_replay.0);
    assert!(receiver.recv().await.is_ok());
    assert!(
        tokio::time::timeout(Duration::from_millis(20), receiver.recv())
            .await
            .is_err()
    );
    Ok(())
}

async fn test_state() -> Result<AppState, String> {
    AppState::new(common::config::AppConfig::default())
        .await
        .map_err(|error| format!("test state failed: {error}"))
}

fn watchlist_item(id: i64) -> shared_types::WatchlistItem {
    shared_types::WatchlistItem {
        id,
        symbol: "BTC-USDT".into(),
        venue_long: Some("binance".into()),
        venue_short: Some("okx".into()),
        min_net_yield: None,
        min_volume_24h: None,
        enabled: true,
        created_at_ms: 1,
        persistence: shared_types::WatchlistPersistence::default(),
        runtime: shared_types::WatchlistItemRuntime::default(),
    }
}

fn rule(watchlist_id: i64) -> AlertRule {
    AlertRule {
        id: 0,
        watchlist_id,
        channel: shared_types::AlertChannel::Toast,
        cooldown_secs: 300,
        enabled: true,
        created_at_ms: 0,
        persistence: shared_types::WatchlistPersistence::default(),
        delivery: shared_types::AlertDeliveryState::configured(&shared_types::AlertChannel::Toast),
        runtime: shared_types::AlertRuleRuntime::default(),
    }
}
