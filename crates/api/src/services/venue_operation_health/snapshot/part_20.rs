const SOURCE_APP_WS_HUB: &str = "app_ws_hub";
const APP_WS_LAG_RECENT_MS: i64 = 60_000;
const APP_WS_LAG_RETRY_AFTER_MS: u64 = 100;

fn app_ws_broadcast_rows(state: &AppState, now_ms: i64) -> Vec<VenueOperationHealth> {
    let snapshots = state.ws_hub().runtime_snapshots();
    snapshots
        .iter()
        .map(|snapshot| app_ws_broadcast_row(snapshot, now_ms))
        .collect()
}

fn app_ws_broadcast_row(
    snapshot: &realtime::WsChannelRuntimeSnapshot,
    now_ms: i64,
) -> VenueOperationHealth {
    let freshness_ms = snapshot
        .last_lag_at_ms
        .map(|observed_at_ms| now_ms.saturating_sub(observed_at_ms).max(0));
    let recent_lag = freshness_ms.is_some_and(|age| age <= APP_WS_LAG_RECENT_MS);
    let status = if recent_lag {
        VenueOperationStatus::Warn
    } else {
        VenueOperationStatus::Ok
    };
    let message = format!(
        "app WS channel {}: subscribers={}, lag_events={}, skipped_messages={}, last_lag_at_ms={}",
        snapshot.channel,
        snapshot.subscribers,
        snapshot.lag_events,
        snapshot.skipped_messages,
        snapshot
            .last_lag_at_ms
            .map_or_else(|| "none".to_owned(), |value| value.to_string())
    );
    let problem = recent_lag.then(|| {
        let mut problem = ApiProblem::new("WS_BROADCAST_LAGGED", message.clone())
            .with_source(SOURCE_APP_WS_HUB)
            .with_retry_after_ms(Some(APP_WS_LAG_RETRY_AFTER_MS));
        problem.details = Some(serde_json::json!({
            "channel": snapshot.channel,
            "lagEvents": snapshot.lag_events,
            "skippedMessages": snapshot.skipped_messages,
            "lastLagAtMs": snapshot.last_lag_at_ms,
        }));
        problem
    });
    VenueOperationHealth {
        venue: "app".to_owned(),
        operation: format!(
            "{}{}",
            shared_types::OP_APP_WS_BROADCAST_PREFIX,
            snapshot.channel
        ),
        status,
        source: SOURCE_APP_WS_HUB.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(true),
        requested: Some(snapshot.lag_events),
        rows: Some(snapshot.skipped_messages),
        freshness_ms,
        retry_after_ms: recent_lag.then_some(APP_WS_LAG_RETRY_AFTER_MS),
        latency_ms: None,
        latency_p95_ms: None,
        error: recent_lag.then_some(message),
        evidence: None,
        problem,
        observed_at_ms: snapshot.last_lag_at_ms.unwrap_or(now_ms),
    }
}
