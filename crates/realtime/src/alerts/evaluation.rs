use std::cmp::Ordering;
use std::collections::HashMap;

use shared_types::{
    market_monitor_net_bps_at, ApiProblem, ArbitrageOpportunityDto, StrategyKind, WatchlistItem,
};

use super::{AlertDeliveryStatus, AlertNotification, AlertRule, AlertRuleRuntimeStatus};

#[derive(Debug, Clone, PartialEq)]
pub struct AlertQueueAttempt {
    pub notification: AlertNotification,
    pub rule_created_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertEvaluation {
    pub attempts: Vec<AlertQueueAttempt>,
    pub runtime_changed: bool,
}

pub fn evaluate_alert_rules(
    rules: &mut [AlertRule],
    watchlist: &[WatchlistItem],
    opportunities: &[ArbitrageOpportunityDto],
    now_ms: i64,
) -> AlertEvaluation {
    let watchlist_by_id = enabled_watchlist_by_id(watchlist);
    let opportunities_by_symbol = opportunities_by_symbol(opportunities);
    let runtime_changed = !rules.is_empty();
    let mut attempts = Vec::new();

    for rule in rules {
        rule.runtime.last_evaluated_at_ms = Some(now_ms);
        if !rule.enabled {
            set_rule_status(rule, AlertRuleRuntimeStatus::Disabled, None);
            continue;
        }
        if !rule.channel.is_runtime_deliverable() {
            set_rule_status(
                rule,
                AlertRuleRuntimeStatus::Blocked,
                Some(runtime_problem(
                    "ALERT_DELIVERY_UNSUPPORTED",
                    "configured alert channel has no runtime delivery transport",
                )),
            );
            continue;
        }
        let Some(item) = watchlist_by_id.get(&rule.watchlist_id).copied() else {
            set_rule_status(
                rule,
                AlertRuleRuntimeStatus::Blocked,
                Some(runtime_problem(
                    "ALERT_WATCHLIST_MISSING",
                    "alert rule references a missing or disabled watchlist item",
                )),
            );
            continue;
        };
        rule.runtime.watchlist_public_prewarm_legs = public_prewarm_leg_count(item);
        rule.runtime.private_ws_symbols = 0;
        if rule
            .runtime
            .next_eligible_at_ms
            .is_some_and(|deadline| now_ms < deadline)
        {
            set_rule_status(rule, AlertRuleRuntimeStatus::Cooldown, None);
            continue;
        }

        let symbol_key = item.symbol.to_ascii_uppercase();
        let candidate = opportunities_by_symbol
            .get(&symbol_key)
            .and_then(|rows| best_matching_opportunity(item, rows, now_ms));
        let Some((opportunity, one_cycle_net_bps, strategy)) = candidate else {
            set_rule_status(rule, AlertRuleRuntimeStatus::Waiting, None);
            continue;
        };
        set_rule_status(rule, AlertRuleRuntimeStatus::Ready, None);
        attempts.push(AlertQueueAttempt {
            notification: notification(
                rule,
                item,
                opportunity,
                strategy,
                one_cycle_net_bps,
                now_ms,
            ),
            rule_created_at_ms: rule.created_at_ms,
        });
    }

    AlertEvaluation {
        attempts,
        runtime_changed,
    }
}

pub fn apply_alert_queue_result(
    rules: &mut [AlertRule],
    attempt: &AlertQueueAttempt,
    subscriber_count: usize,
    problem: Option<ApiProblem>,
) -> bool {
    let Some(rule) = rules.iter_mut().find(|rule| {
        rule.id == attempt.notification.rule_id && rule.created_at_ms == attempt.rule_created_at_ms
    }) else {
        return false;
    };

    if subscriber_count == 0 || problem.is_some() {
        let problem = problem.unwrap_or_else(|| {
            runtime_problem(
                "ALERT_TOAST_NOT_QUEUED",
                "no authenticated application websocket subscriber accepted the toast alert",
            )
        });
        rule.delivery.last_delivery_status = AlertDeliveryStatus::Blocked;
        rule.delivery.last_error = Some(problem.clone());
        set_rule_status(rule, AlertRuleRuntimeStatus::Blocked, Some(problem));
        rule.runtime.next_eligible_at_ms = None;
        return false;
    }

    let queued_at_ms = attempt.notification.queued_at_ms;
    rule.runtime.status = AlertRuleRuntimeStatus::Queued;
    rule.runtime.problem = None;
    rule.runtime.last_triggered_at_ms = Some(queued_at_ms);
    rule.runtime.next_eligible_at_ms = Some(cooldown_deadline_ms(queued_at_ms, rule.cooldown_secs));
    rule.runtime.trigger_count = rule.runtime.trigger_count.saturating_add(1);
    rule.runtime.last_opportunity_id = Some(attempt.notification.opportunity_id.clone());
    rule.delivery.last_fired_at_ms = Some(queued_at_ms);
    rule.delivery.last_delivery_status = AlertDeliveryStatus::Queued;
    rule.delivery.last_error = None;
    true
}

fn best_matching_opportunity<'a>(
    item: &WatchlistItem,
    rows: &[&'a ArbitrageOpportunityDto],
    now_ms: i64,
) -> Option<(&'a ArbitrageOpportunityDto, f64, StrategyKind)> {
    rows.iter()
        .filter_map(|row| eligible_candidate(item, row, now_ms))
        .max_by(|left, right| compare_candidate(*left, *right))
}

fn eligible_candidate<'a>(
    item: &WatchlistItem,
    opportunity: &'a ArbitrageOpportunityDto,
    now_ms: i64,
) -> Option<(&'a ArbitrageOpportunityDto, f64, StrategyKind)> {
    if !watchlist_matches(item, opportunity) {
        return None;
    }
    let strategy = opportunity.strategy_kind?;
    let one_cycle_net_bps = market_monitor_net_bps_at(opportunity, now_ms)?;
    if !opportunity.net_single_yield.is_finite() || !opportunity.volume_24h.is_finite() {
        return None;
    }
    Some((opportunity, one_cycle_net_bps, strategy))
}

fn compare_candidate(
    left: (&ArbitrageOpportunityDto, f64, StrategyKind),
    right: (&ArbitrageOpportunityDto, f64, StrategyKind),
) -> Ordering {
    left.1
        .total_cmp(&right.1)
        .then_with(|| left.0.volume_24h.total_cmp(&right.0.volume_24h))
        .then_with(|| left.0.net_single_yield.total_cmp(&right.0.net_single_yield))
        .then_with(|| right.0.id.cmp(&left.0.id))
}

fn watchlist_matches(item: &WatchlistItem, opportunity: &ArbitrageOpportunityDto) -> bool {
    opportunity.symbol.eq_ignore_ascii_case(&item.symbol)
        && item
            .venue_long
            .as_deref()
            .is_none_or(|venue| shared_types::venue_names_equal(&opportunity.long_exchange, venue))
        && item
            .venue_short
            .as_deref()
            .is_none_or(|venue| shared_types::venue_names_equal(&opportunity.short_exchange, venue))
        && item
            .min_net_yield
            .is_none_or(|minimum| opportunity.net_single_yield >= minimum)
        && item
            .min_volume_24h
            .is_none_or(|minimum| opportunity.volume_24h >= minimum)
}

fn notification(
    rule: &AlertRule,
    item: &WatchlistItem,
    opportunity: &ArbitrageOpportunityDto,
    strategy: StrategyKind,
    one_cycle_net_bps: f64,
    now_ms: i64,
) -> AlertNotification {
    AlertNotification {
        id: format!("alert:{}:{}:{now_ms}", rule.id, opportunity.id),
        rule_id: rule.id,
        watchlist_id: item.id,
        opportunity_id: opportunity.id.clone(),
        symbol: opportunity.symbol.clone(),
        strategy,
        long_exchange: opportunity.long_exchange.clone(),
        short_exchange: opportunity.short_exchange.clone(),
        one_cycle_net_bps,
        net_single_yield: opportunity.net_single_yield,
        queued_at_ms: now_ms,
    }
}

fn cooldown_deadline_ms(queued_at_ms: i64, cooldown_secs: u64) -> i64 {
    i64::try_from(cooldown_secs)
        .ok()
        .and_then(|seconds| seconds.checked_mul(1_000))
        .map_or(i64::MAX, |cooldown_ms| {
            queued_at_ms.saturating_add(cooldown_ms)
        })
}

fn enabled_watchlist_by_id(rows: &[WatchlistItem]) -> HashMap<i64, &WatchlistItem> {
    rows.iter()
        .filter(|item| item.enabled)
        .map(|item| (item.id, item))
        .collect()
}

fn opportunities_by_symbol(
    rows: &[ArbitrageOpportunityDto],
) -> HashMap<String, Vec<&ArbitrageOpportunityDto>> {
    let mut out: HashMap<String, Vec<&ArbitrageOpportunityDto>> = HashMap::new();
    for row in rows {
        out.entry(row.symbol.to_ascii_uppercase())
            .or_default()
            .push(row);
    }
    out
}

fn set_rule_status(
    rule: &mut AlertRule,
    status: AlertRuleRuntimeStatus,
    problem: Option<ApiProblem>,
) {
    rule.runtime.status = status;
    rule.runtime.problem = problem;
}

fn runtime_problem(code: &str, message: &str) -> ApiProblem {
    ApiProblem::new(code, message).with_source("alert_runtime")
}

fn public_prewarm_leg_count(item: &WatchlistItem) -> usize {
    match (item.venue_long.as_deref(), item.venue_short.as_deref()) {
        (Some(left), Some(right)) if shared_types::venue_names_equal(left, right) => 1,
        (Some(_), Some(_)) => 2,
        (Some(_), None) | (None, Some(_)) => 1,
        (None, None) => 0,
    }
}
