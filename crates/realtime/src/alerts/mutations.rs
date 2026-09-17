use super::{AlertDeliveryState, AlertRule, AlertRuleRuntime, WatchlistItem, WatchlistPersistence};

#[derive(Debug, Clone, PartialEq)]
pub struct WatchlistMutation {
    pub item: WatchlistItem,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AlertRuleMutation {
    pub rule: AlertRule,
    pub changed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchlistRemoval {
    pub changed: bool,
    pub removed_alert_rule_ids: Vec<i64>,
}

pub fn insert_watchlist_item(
    rows: &mut Vec<WatchlistItem>,
    mut item: WatchlistItem,
    now_ms: i64,
    actor: &str,
) -> WatchlistMutation {
    normalize_watchlist_item(&mut item);
    if let Some(existing) = rows
        .iter()
        .find(|row| same_watchlist_business_key(row, &item))
    {
        return WatchlistMutation {
            item: existing.clone(),
            changed: false,
        };
    }

    item.id = next_id(rows.iter().map(|row| row.id));
    item.created_at_ms = now_ms;
    item.persistence = WatchlistPersistence::pending(actor, now_ms);
    rows.push(item.clone());
    WatchlistMutation {
        item,
        changed: true,
    }
}

pub fn insert_alert_rule(
    rows: &mut Vec<AlertRule>,
    mut rule: AlertRule,
    watchlist: &WatchlistItem,
    now_ms: i64,
    actor: &str,
) -> AlertRuleMutation {
    if let Some(existing) = rows.iter().find(|row| same_alert_business_key(row, &rule)) {
        return AlertRuleMutation {
            rule: existing.clone(),
            changed: false,
        };
    }

    rule.id = next_id(rows.iter().map(|row| row.id));
    rule.created_at_ms = now_ms;
    rule.persistence = WatchlistPersistence::pending(actor, now_ms);
    rule.delivery = AlertDeliveryState::configured(&rule.channel);
    rule.runtime = AlertRuleRuntime::configured(
        &rule.channel,
        rule.enabled,
        public_prewarm_leg_count(watchlist),
    );
    rows.push(rule.clone());
    AlertRuleMutation {
        rule,
        changed: true,
    }
}

pub fn remove_watchlist_item(
    watchlist: &mut Vec<WatchlistItem>,
    alert_rules: &mut Vec<AlertRule>,
    id: i64,
) -> WatchlistRemoval {
    let before = watchlist.len();
    watchlist.retain(|row| row.id != id);
    if before == watchlist.len() {
        return WatchlistRemoval {
            changed: false,
            removed_alert_rule_ids: Vec::new(),
        };
    }

    let removed_alert_rule_ids = alert_rules
        .iter()
        .filter(|rule| rule.watchlist_id == id)
        .map(|rule| rule.id)
        .collect::<Vec<_>>();
    alert_rules.retain(|rule| rule.watchlist_id != id);
    WatchlistRemoval {
        changed: true,
        removed_alert_rule_ids,
    }
}

pub fn remove_alert_rule(rows: &mut Vec<AlertRule>, id: i64) -> bool {
    let before = rows.len();
    rows.retain(|row| row.id != id);
    before != rows.len()
}

fn normalize_watchlist_item(item: &mut WatchlistItem) {
    item.id = 0;
    item.created_at_ms = 0;
    item.persistence = WatchlistPersistence::default();
    item.runtime = shared_types::WatchlistItemRuntime::default();
    item.symbol = item.symbol.trim().to_ascii_uppercase();
    item.venue_long = normalized_venue(item.venue_long.take());
    item.venue_short = normalized_venue(item.venue_short.take());
}

fn normalized_venue(value: Option<String>) -> Option<String> {
    value.map(|value| value.trim().to_ascii_lowercase())
}

fn same_watchlist_business_key(left: &WatchlistItem, right: &WatchlistItem) -> bool {
    left.symbol.eq_ignore_ascii_case(&right.symbol)
        && same_optional_venue(left.venue_long.as_deref(), right.venue_long.as_deref())
        && same_optional_venue(left.venue_short.as_deref(), right.venue_short.as_deref())
        && left.min_net_yield == right.min_net_yield
        && left.min_volume_24h == right.min_volume_24h
        && left.enabled == right.enabled
}

fn same_alert_business_key(left: &AlertRule, right: &AlertRule) -> bool {
    left.watchlist_id == right.watchlist_id
        && left.channel.delivery_kind() == right.channel.delivery_kind()
        && left.cooldown_secs == right.cooldown_secs
        && left.enabled == right.enabled
}

fn same_optional_venue(left: Option<&str>, right: Option<&str>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => shared_types::venue_names_equal(left, right),
        (None, None) => true,
        _ => false,
    }
}

fn public_prewarm_leg_count(item: &WatchlistItem) -> usize {
    match (item.venue_long.as_deref(), item.venue_short.as_deref()) {
        (Some(left), Some(right)) if shared_types::venue_names_equal(left, right) => 1,
        (Some(_), Some(_)) => 2,
        (Some(_), None) | (None, Some(_)) => 1,
        (None, None) => 0,
    }
}

fn next_id(ids: impl Iterator<Item = i64>) -> i64 {
    ids.max().unwrap_or(0) + 1
}
