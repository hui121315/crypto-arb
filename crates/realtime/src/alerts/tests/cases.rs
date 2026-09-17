use super::fixtures::{alert_rule, ready_opportunity, watchlist_item};
use super::*;

#[test]
fn constructors_declare_volatile_restart_and_side_effect_budget() {
    let watchlist = watchlist_envelope(Vec::new());
    let alerts = alert_rules_envelope(Vec::new());

    assert_eq!(watchlist.runtime, alerts.runtime);
    assert_eq!(watchlist.runtime.feature_gate, WATCHLIST_ALERTS_FEATURE);
    assert!(watchlist.runtime.volatile);
    assert_eq!(watchlist.runtime.restart_behavior, "cleared_on_restart");
    assert_eq!(watchlist.runtime.private_ws_symbols_from_watchlist, 0);
    assert_eq!(
        watchlist.runtime.public_ticker_symbols_per_venue_limit,
        WATCHLIST_TICKER_SYMBOLS_PER_VENUE
    );
}

#[test]
fn watchlist_and_alert_create_replay_without_duplicate_business_side_effects() {
    let mut watchlist = Vec::new();
    let first = insert_watchlist_item(&mut watchlist, watchlist_item(0), 10, "operator");
    let second = insert_watchlist_item(&mut watchlist, watchlist_item(0), 20, "operator");

    assert!(first.changed);
    assert!(!second.changed);
    assert_eq!(first.item, second.item);
    assert_eq!(watchlist.len(), 1);
    assert_eq!(first.item.persistence.created_by, "operator");
    assert_eq!(first.item.persistence.version, 1);
    assert_eq!(
        first.item.persistence.persist_status,
        WatchlistPersistStatus::Pending
    );

    let mut rules = Vec::new();
    let first_rule = insert_alert_rule(
        &mut rules,
        alert_rule(0, first.item.id),
        &first.item,
        30,
        "operator",
    );
    let second_rule = insert_alert_rule(
        &mut rules,
        alert_rule(0, first.item.id),
        &first.item,
        40,
        "operator",
    );

    assert!(first_rule.changed);
    assert!(!second_rule.changed);
    assert_eq!(first_rule.rule, second_rule.rule);
    assert_eq!(rules.len(), 1);
    assert_eq!(first_rule.rule.persistence.created_by, "operator");
    assert_eq!(first_rule.rule.delivery.delivery_kind, "toast");
}

#[test]
fn watchlist_delete_replays_and_cascades_attached_rules() {
    let mut watchlist = vec![watchlist_item(7)];
    let mut rules = vec![alert_rule(9, 7)];

    let first = remove_watchlist_item(&mut watchlist, &mut rules, 7);
    let replay = remove_watchlist_item(&mut watchlist, &mut rules, 7);

    assert!(first.changed);
    assert_eq!(first.removed_alert_rule_ids, vec![9]);
    assert!(!replay.changed);
    assert!(replay.removed_alert_rule_ids.is_empty());
    assert!(watchlist.is_empty());
    assert!(rules.is_empty());
}

#[test]
fn alert_matching_requires_fresh_execution_and_highest_verified_net_profit() {
    let watchlist = vec![watchlist_item(7)];
    let mut rules = vec![alert_rule(9, 7)];
    let low = ready_opportunity("low", shared_types::StrategyKind::PerpCross, 7.0);
    let high = ready_opportunity("high", shared_types::StrategyKind::PerpCross, 9.5);
    let non_p0 = ready_opportunity("phase2", shared_types::StrategyKind::FundingCarry, 10.0);
    let mut blocked = ready_opportunity("blocked", shared_types::StrategyKind::PerpCross, 11.0);
    blocked.execution_blockers.push("fixture blocker".into());

    let opportunities = vec![low, non_p0, blocked, high];
    let now_ms = chrono::Utc::now().timestamp_millis();
    let evaluation = evaluate_alert_rules(&mut rules, &watchlist, &opportunities, now_ms);

    assert_eq!(evaluation.attempts.len(), 1);
    assert_eq!(evaluation.attempts[0].notification.opportunity_id, "high");
    assert_eq!(evaluation.attempts[0].notification.one_cycle_net_bps, 9.5);
    assert_eq!(rules[0].runtime.status, AlertRuleRuntimeStatus::Ready);
}

#[test]
fn alert_matching_rejects_stale_evidence_but_not_missing_legacy_ranking() {
    let watchlist = vec![watchlist_item(7)];
    let mut rules = vec![alert_rule(9, 7)];
    let mut stale = ready_opportunity("stale", shared_types::StrategyKind::PerpCross, 9.5);
    if let Some(evidence) = stale.long_leg_market_evidence.as_mut() {
        evidence.health.quality = shared_types::MarketDataQuality::StaleBlocked;
    }
    let mut no_ranking =
        ready_opportunity("no-ranking", shared_types::StrategyKind::PerpCross, 9.0);
    no_ranking.ranking_key = None;

    let now_ms = chrono::Utc::now().timestamp_millis();
    let evaluation = evaluate_alert_rules(&mut rules, &watchlist, &[stale, no_ranking], now_ms);

    assert_eq!(evaluation.attempts.len(), 1);
    assert_eq!(
        evaluation.attempts[0].notification.opportunity_id,
        "no-ranking"
    );
    assert_eq!(rules[0].runtime.status, AlertRuleRuntimeStatus::Ready);
}

#[test]
fn alert_monitoring_does_not_require_list_orderbook_depth() {
    let watchlist = vec![watchlist_item(7)];
    let mut rules = vec![alert_rule(9, 7)];
    let mut opportunity =
        ready_opportunity("depth-on-build", shared_types::StrategyKind::SpotPerp, 9.0);
    opportunity.execution_eligible = false;
    opportunity.execution_blockers = vec![shared_types::DEFERRED_SPOT_PERP_TICKET_BLOCKER.into()];
    let now_ms = chrono::Utc::now().timestamp_millis();

    let evaluation = evaluate_alert_rules(&mut rules, &watchlist, &[opportunity], now_ms);

    assert_eq!(evaluation.attempts.len(), 1);
    assert_eq!(
        evaluation.attempts[0].notification.opportunity_id,
        "depth-on-build"
    );
}

#[test]
fn toast_queue_requires_real_subscriber_before_cooldown_and_count() {
    let watchlist = vec![watchlist_item(7)];
    let mut rules = vec![alert_rule(9, 7)];
    let opportunity = ready_opportunity("deliver", shared_types::StrategyKind::PerpCross, 9.1);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let evaluation = evaluate_alert_rules(&mut rules, &watchlist, &[opportunity], now_ms);
    let attempt = &evaluation.attempts[0];

    assert!(!apply_alert_queue_result(&mut rules, attempt, 0, None));
    assert_eq!(rules[0].runtime.status, AlertRuleRuntimeStatus::Blocked);
    assert_eq!(rules[0].runtime.trigger_count, 0);
    assert_eq!(rules[0].runtime.next_eligible_at_ms, None);
    assert_eq!(
        rules[0].delivery.last_delivery_status,
        AlertDeliveryStatus::Blocked
    );
    assert_eq!(
        rules[0]
            .delivery
            .last_error
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("ALERT_TOAST_NOT_QUEUED")
    );
    assert_eq!(
        rules[0]
            .runtime
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("ALERT_TOAST_NOT_QUEUED")
    );

    assert!(apply_alert_queue_result(&mut rules, attempt, 1, None));
    assert_eq!(rules[0].runtime.status, AlertRuleRuntimeStatus::Queued);
    assert_eq!(rules[0].runtime.trigger_count, 1);
    assert!(rules[0].runtime.next_eligible_at_ms.is_some());
    assert_eq!(
        rules[0].delivery.last_delivery_status,
        AlertDeliveryStatus::Queued
    );
    assert_eq!(rules[0].delivery.last_fired_at_ms, Some(now_ms));
    assert!(rules[0].delivery.last_error.is_none());
}

#[test]
fn unvalidated_extreme_cooldown_cannot_wrap_into_the_past() {
    let watchlist = vec![watchlist_item(7)];
    let mut rule = alert_rule(9, 7);
    rule.cooldown_secs = u64::MAX;
    let opportunity = ready_opportunity("deliver", shared_types::StrategyKind::PerpCross, 9.1);
    let now_ms = chrono::Utc::now().timestamp_millis();
    let evaluation = evaluate_alert_rules(
        std::slice::from_mut(&mut rule),
        &watchlist,
        &[opportunity],
        now_ms,
    );

    assert!(apply_alert_queue_result(
        std::slice::from_mut(&mut rule),
        &evaluation.attempts[0],
        1,
        None,
    ));
    assert_eq!(rule.runtime.next_eligible_at_ms, Some(i64::MAX));
}

#[test]
fn webhook_channels_remain_fail_closed_and_secret_redacted() {
    let channel = AlertChannel::GenericWebhook {
        url: "https://hooks.example/secret-path".into(),
        secret: Some("token-value".into()),
    };

    assert!(channel.validate().is_err());
    assert!(!channel.is_runtime_deliverable());
    let serialized = serde_json::to_string(&channel).unwrap_or_default();
    assert!(!serialized.contains("secret-path"));
    assert!(!serialized.contains("token-value"));
}
