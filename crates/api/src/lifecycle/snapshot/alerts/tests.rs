use super::*;

#[test]
fn toast_without_subscriber_is_not_reported_as_queued() {
    let hub = realtime::WsHub::new(8);
    let outcome = queue_alert(&hub, queue_attempt());

    assert_eq!(outcome.subscriber_count, 0);
    assert!(outcome.problem.is_none());
}

#[tokio::test]
async fn toast_with_subscriber_queues_typed_user_alert_event() -> Result<(), String> {
    let hub = realtime::WsHub::new(8);
    let mut receiver = hub.subscribe(realtime::channels::ALERTS);

    let outcome = queue_alert(&hub, queue_attempt());
    let message = receiver
        .recv()
        .await
        .map_err(|error| format!("alert receive failed: {error}"))?;

    assert_eq!(outcome.subscriber_count, 1);
    let Some(value) = message.payload_json() else {
        return Err("alert event was not a JSON message".into());
    };
    let event = serde_json::from_value::<shared_types::AlertStreamEvent>(value)
        .map_err(|error| format!("typed alert decode failed: {error}"))?;
    let shared_types::AlertStreamEvent::AlertTriggered { notification } = event else {
        return Err("expected alert_triggered event".into());
    };
    assert_eq!(notification.opportunity_id, "opp-1");
    assert_eq!(notification.one_cycle_net_bps, 9.1);
    Ok(())
}

#[test]
fn deleted_rule_generation_is_rejected_before_notification_queue() {
    let hub = realtime::WsHub::new(8);
    let mut receiver = hub.subscribe(realtime::channels::ALERTS);
    let metrics = crate::metrics::Metrics::new();
    let mut rules = Vec::new();

    queue_evaluation_attempts(&hub, &metrics, &mut rules, vec![queue_attempt()]);

    assert!(matches!(
        receiver.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(metrics.snapshot().alerts_fired_total, 0);
}

#[test]
fn cooldown_registry_only_tracks_future_enabled_delivery_deadlines() {
    let mut rule = shared_types::AlertRule {
        id: 7,
        watchlist_id: 2,
        channel: shared_types::AlertChannel::Toast,
        cooldown_secs: 300,
        enabled: true,
        created_at_ms: 1,
        persistence: shared_types::WatchlistPersistence::default(),
        delivery: shared_types::AlertDeliveryState::configured(&shared_types::AlertChannel::Toast),
        runtime: shared_types::AlertRuleRuntime::default(),
    };
    rule.runtime.next_eligible_at_ms = Some(2_000);

    assert_eq!(active_cooldown(&rule, 1_000), Some((7, 2_000)));
    assert_eq!(active_cooldown(&rule, 2_000), None);
    rule.enabled = false;
    assert_eq!(active_cooldown(&rule, 1_000), None);
}

fn queue_attempt() -> realtime::alerts::AlertQueueAttempt {
    realtime::alerts::AlertQueueAttempt {
        notification: shared_types::AlertNotification {
            id: "alert:1:opp-1:42".into(),
            rule_id: 1,
            watchlist_id: 2,
            opportunity_id: "opp-1".into(),
            symbol: "BTC-USDT".into(),
            strategy: shared_types::StrategyKind::PerpCross,
            long_exchange: "binance".into(),
            short_exchange: "okx".into(),
            one_cycle_net_bps: 9.1,
            net_single_yield: 0.2,
            queued_at_ms: 42,
        },
        rule_created_at_ms: 1,
    }
}
