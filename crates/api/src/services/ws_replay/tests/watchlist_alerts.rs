use super::*;

#[tokio::test]
async fn watchlist_and_alert_channels_replay_current_volatile_state() -> Result<(), String> {
    let state = test_state().await?;
    let watchlist = shared_types::WatchlistItem {
        id: 7,
        symbol: "BTC-USDT".into(),
        venue_long: Some("binance".into()),
        venue_short: Some("okx".into()),
        min_net_yield: Some(0.1),
        min_volume_24h: None,
        enabled: true,
        created_at_ms: 1,
        persistence: shared_types::WatchlistPersistence::default(),
        runtime: shared_types::WatchlistItemRuntime::default(),
    };
    state.watchlist().write().await.push(watchlist);
    state
        .alert_rules()
        .write()
        .await
        .push(shared_types::AlertRule {
            id: 9,
            watchlist_id: 7,
            channel: shared_types::AlertChannel::Toast,
            cooldown_secs: 300,
            enabled: true,
            created_at_ms: 2,
            persistence: shared_types::WatchlistPersistence::default(),
            delivery: shared_types::AlertDeliveryState::configured(
                &shared_types::AlertChannel::Toast,
            ),
            runtime: shared_types::AlertRuleRuntime::configured(
                &shared_types::AlertChannel::Toast,
                true,
                2,
            ),
        });

    let watchlist = payloads_for_channel(channels::WATCHLIST, &state)
        .await
        .map_err(|error| format!("watchlist replay failed: {error}"))?;
    let alerts = payloads_for_channel(channels::ALERTS, &state)
        .await
        .map_err(|error| format!("alert replay failed: {error}"))?;

    assert_eq!(watchlist[0].payload["event"], "watchlist_changed");
    assert_eq!(watchlist[0].payload["envelope"]["items"][0]["id"], 7);
    assert_eq!(alerts[0].payload["event"], "alert_rules_changed");
    assert_eq!(alerts[0].payload["envelope"]["rules"][0]["id"], 9);
    assert_eq!(
        alerts[0].payload["envelope"]["rules"][0]["runtime"]["transport"],
        "app_websocket_toast"
    );
    Ok(())
}
