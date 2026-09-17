#[tokio::test]
async fn snapshot_exposes_app_ws_lag_counts_by_channel() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let _orders = state.ws_hub().subscribe(realtime::channels::ORDERS);
    let now_ms = common::time::now_ms();
    state
        .ws_hub()
        .record_lag(realtime::channels::ORDERS, 7, now_ms);

    let health = snapshot(&state);
    let row = health
        .rows
        .iter()
        .find(|row| {
            row.operation
                == format!(
                    "{}{}",
                    shared_types::OP_APP_WS_BROADCAST_PREFIX,
                    realtime::channels::ORDERS
                )
        })
        .expect("app WS orders runtime row");

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.requested, Some(1));
    assert_eq!(row.rows, Some(7));
    assert_eq!(row.source, "app_ws_hub");
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some("WS_BROADCAST_LAGGED")
    );
    assert!(row.message.contains("skipped_messages=7"));
    Ok(())
}

#[test]
fn old_app_ws_lag_keeps_cumulative_counts_without_current_warning() {
    let row = app_ws_broadcast_row(
        &realtime::WsChannelRuntimeSnapshot {
            channel: realtime::channels::SYSTEM.to_owned(),
            subscribers: 1,
            lag_events: 2,
            skipped_messages: 5,
            last_lag_at_ms: Some(1_000),
        },
        62_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.requested, Some(2));
    assert_eq!(row.rows, Some(5));
    assert!(row.problem.is_none());
    assert!(row.error.is_none());
}
