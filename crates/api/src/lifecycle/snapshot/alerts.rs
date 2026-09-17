use super::*;

struct QueueOutcome {
    attempt: realtime::alerts::AlertQueueAttempt,
    subscriber_count: usize,
    problem: Option<shared_types::ApiProblem>,
}

struct EvaluationOutcome {
    runtime_changed: bool,
    delivery_attempted: bool,
}

/// persist 在 `scan_lock` 持有期间执行且位于 WS 推送之前：必须有上界，
/// 磁盘慢时不能把 `SQLite` 落盘延迟加到 arbitrage diff 推送前面。
const ALERT_PERSIST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

pub(super) async fn fire_matching_alerts(
    hub: &realtime::WsHub,
    metrics: &Arc<crate::metrics::Metrics>,
    sinks: &SnapshotSinks,
    opportunities: &[shared_types::ArbitrageOpportunityDto],
) -> Result<(), String> {
    let _serial = sinks.watchlist_alert_mutation_lock.lock().await;
    let watchlist = sinks.watchlist.read().await.clone();
    let now_ms = common::time::now_ms();
    // 第一阶段：写锁内只做纯内存评估与入队（无 await），锁持有时间与磁盘无关。
    let (runtime_changed, rules_snapshot) = {
        let mut rules = sinks.alert_rules.write().await;
        let evaluation =
            evaluate_and_queue_locked(hub, metrics, &mut rules, &watchlist, opportunities, now_ms);
        sync_active_cooldowns(sinks, &rules, now_ms);
        (
            evaluation.runtime_changed,
            evaluation.delivery_attempted.then(|| rules.clone()),
        )
    };
    // 第二阶段：锁外持久化（mutation_lock 仍串行化并发 mutation，快照一致），
    // 完成后短暂回锁记录 persist 状态。读端点在此窗口只会看到"状态未刷新"的
    // 良性瞬态，不再被磁盘 IO 阻塞。
    let persist_result = if let Some(rules_snapshot) = rules_snapshot {
        let persisted = tokio::time::timeout(
            ALERT_PERSIST_TIMEOUT,
            sinks
                .watchlist_alert_store
                .persist_snapshot(&watchlist, &rules_snapshot),
        )
        .await;
        let status_result = match persisted {
            Ok(Ok(status)) => Ok(status),
            Ok(Err(error)) => Err(format!(
                "watchlist alert delivery status persist failed: {error}"
            )),
            Err(_) => Err("watchlist alert delivery status persist timed out".to_owned()),
        };
        let mut rules = sinks.alert_rules.write().await;
        match status_result {
            Ok(status) => {
                crate::services::watchlist_alerts::mark_alert_rule_persist_status(
                    &mut rules, status,
                );
                Ok(())
            }
            Err(error) => {
                crate::services::watchlist_alerts::mark_alert_rule_persist_status(
                    &mut rules,
                    shared_types::WatchlistPersistStatus::Degraded,
                );
                Err(error)
            }
        }
    } else {
        Ok(())
    };

    if runtime_changed {
        publish_alert_rule_runtime(hub, sinks, now_ms).await?;
    }
    persist_result
}

fn evaluate_and_queue_locked(
    hub: &realtime::WsHub,
    metrics: &crate::metrics::Metrics,
    rules: &mut [shared_types::AlertRule],
    watchlist: &[shared_types::WatchlistItem],
    opportunities: &[shared_types::ArbitrageOpportunityDto],
    now_ms: i64,
) -> EvaluationOutcome {
    let evaluation =
        realtime::alerts::evaluate_alert_rules(rules, watchlist, opportunities, now_ms);
    let delivery_attempted = queue_evaluation_attempts(hub, metrics, rules, evaluation.attempts);
    EvaluationOutcome {
        runtime_changed: evaluation.runtime_changed,
        delivery_attempted,
    }
}

fn queue_evaluation_attempts(
    hub: &realtime::WsHub,
    metrics: &crate::metrics::Metrics,
    rules: &mut [shared_types::AlertRule],
    attempts: Vec<realtime::alerts::AlertQueueAttempt>,
) -> bool {
    let mut delivery_attempted = false;
    for attempt in attempts {
        if !rule_generation_exists(rules, &attempt) {
            continue;
        }
        delivery_attempted = true;
        let outcome = queue_alert(hub, attempt);
        apply_queue_outcome(metrics, rules, &outcome);
    }
    delivery_attempted
}

fn rule_generation_exists(
    rules: &[shared_types::AlertRule],
    attempt: &realtime::alerts::AlertQueueAttempt,
) -> bool {
    rules.iter().any(|rule| {
        rule.id == attempt.notification.rule_id && rule.created_at_ms == attempt.rule_created_at_ms
    })
}

fn queue_alert(
    hub: &realtime::WsHub,
    attempt: realtime::alerts::AlertQueueAttempt,
) -> QueueOutcome {
    let event = shared_types::AlertStreamEvent::AlertTriggered {
        notification: attempt.notification.clone(),
    };
    match realtime::WsMessage::json(&event) {
        Ok(message) => QueueOutcome {
            subscriber_count: hub.publish(realtime::channels::ALERTS, message),
            attempt,
            problem: None,
        },
        Err(error) => QueueOutcome {
            subscriber_count: 0,
            attempt,
            problem: Some(
                shared_types::ApiProblem::new(
                    "ALERT_NOTIFICATION_SERIALIZE_FAILED",
                    format!("alert notification serialization failed: {error}"),
                )
                .with_source("alert_runtime"),
            ),
        },
    }
}

fn apply_queue_outcome(
    metrics: &crate::metrics::Metrics,
    rules: &mut [shared_types::AlertRule],
    outcome: &QueueOutcome,
) {
    if realtime::alerts::apply_alert_queue_result(
        rules,
        &outcome.attempt,
        outcome.subscriber_count,
        outcome.problem.clone(),
    ) {
        metrics.record_alert_fired();
    }
}

fn sync_active_cooldowns(sinks: &SnapshotSinks, rules: &[shared_types::AlertRule], now_ms: i64) {
    sinks.alert_cooldowns.clear();
    for rule in rules.iter() {
        if let Some((rule_id, deadline_ms)) = active_cooldown(rule, now_ms) {
            sinks.alert_cooldowns.insert(rule_id, deadline_ms);
        }
    }
}

fn active_cooldown(rule: &shared_types::AlertRule, now_ms: i64) -> Option<(i64, i64)> {
    rule.enabled
        .then_some(rule.runtime.next_eligible_at_ms)
        .flatten()
        .filter(|deadline_ms| *deadline_ms > now_ms)
        .map(|deadline_ms| (rule.id, deadline_ms))
}

async fn publish_alert_rule_runtime(
    hub: &realtime::WsHub,
    sinks: &SnapshotSinks,
    timestamp_ms: i64,
) -> Result<(), String> {
    let envelope = realtime::alerts::alert_rules_envelope_with_storage(
        sinks.alert_rules.read().await.clone(),
        sinks.watchlist_alert_store.health(),
    );
    let event = shared_types::AlertStreamEvent::AlertRulesChanged {
        envelope: Box::new(envelope),
        timestamp_ms,
    };
    let message = realtime::WsMessage::json(&event)
        .map_err(|error| format!("alert rule runtime serialization failed: {error}"))?;
    hub.publish(realtime::channels::ALERTS, message);
    Ok(())
}

#[cfg(test)]
#[path = "alerts/tests.rs"]
mod tests;
