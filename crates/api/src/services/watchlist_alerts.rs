use axum::http::StatusCode;
use common::AppError;
use realtime::alerts::{
    alert_rules_envelope_with_storage, insert_alert_rule, insert_watchlist_item, remove_alert_rule,
    remove_watchlist_item, watchlist_envelope_with_storage,
};
use shared_types::{
    AlertRule, AlertRulesEnvelope, WatchlistEnvelope, WatchlistItem, WatchlistPersistStatus,
};

use crate::state::AppState;

pub(crate) struct WatchlistMutationResponse {
    pub(crate) envelope: WatchlistEnvelope,
    pub(crate) item: Option<WatchlistItem>,
    pub(crate) changed: bool,
    pub(crate) removed_alert_rule_ids: Vec<i64>,
    pub(crate) alert_rules_envelope: Option<AlertRulesEnvelope>,
}

pub(crate) struct AlertRuleMutationResponse {
    pub(crate) envelope: AlertRulesEnvelope,
    pub(crate) rule: Option<AlertRule>,
    pub(crate) changed: bool,
}

pub(crate) fn watchlist_envelope(state: &AppState, items: Vec<WatchlistItem>) -> WatchlistEnvelope {
    watchlist_envelope_with_storage(items, state.watchlist_alert_store().health())
}

pub(crate) fn alert_rules_envelope(state: &AppState, rules: Vec<AlertRule>) -> AlertRulesEnvelope {
    alert_rules_envelope_with_storage(rules, state.watchlist_alert_store().health())
}

// mutation 一律 snapshot-then-commit：在 mutation_lock（唯一写者串行化）保护下
// 克隆当前行集、在本地副本上套用变更并完成 SQLite 持久化（全程不持 RwLock），
// 成功后短暂加写锁提交。读端点与告警评估循环不再被磁盘 IO 阻塞；持久化失败
// 时共享状态从未被改动，无需回滚。
pub(crate) async fn create_watchlist_item(
    state: &AppState,
    item: WatchlistItem,
    actor: &str,
) -> Result<WatchlistMutationResponse, AppError> {
    let _serial = state.watchlist_alert_mutation_lock().lock().await;
    let mut watchlist = state.watchlist().read().await.clone();
    let mut alert_rules = state.alert_rules().read().await.clone();
    let mutation = insert_watchlist_item(&mut watchlist, item, common::time::now_ms(), actor);
    if mutation.changed {
        persist_rows(state, &mut watchlist, &mut alert_rules).await?;
        *state.watchlist().write().await = watchlist.clone();
        *state.alert_rules().write().await = alert_rules;
    }
    let item = watchlist
        .iter()
        .find(|item| item.id == mutation.item.id)
        .cloned();
    Ok(WatchlistMutationResponse {
        envelope: watchlist_envelope(state, watchlist),
        item,
        changed: mutation.changed,
        removed_alert_rule_ids: Vec::new(),
        alert_rules_envelope: None,
    })
}

pub(crate) async fn remove_watchlist(
    state: &AppState,
    id: i64,
) -> Result<WatchlistMutationResponse, AppError> {
    let _serial = state.watchlist_alert_mutation_lock().lock().await;
    let mut watchlist = state.watchlist().read().await.clone();
    let mut alert_rules = state.alert_rules().read().await.clone();
    let removal = remove_watchlist_item(&mut watchlist, &mut alert_rules, id);
    if removal.changed {
        persist_rows(state, &mut watchlist, &mut alert_rules).await?;
        *state.watchlist().write().await = watchlist.clone();
        *state.alert_rules().write().await = alert_rules.clone();
    }
    Ok(WatchlistMutationResponse {
        envelope: watchlist_envelope(state, watchlist),
        item: None,
        changed: removal.changed,
        removed_alert_rule_ids: removal.removed_alert_rule_ids,
        alert_rules_envelope: Some(alert_rules_envelope(state, alert_rules)),
    })
}

pub(crate) async fn create_alert_rule(
    state: &AppState,
    rule: AlertRule,
    actor: &str,
) -> Result<AlertRuleMutationResponse, AppError> {
    let _serial = state.watchlist_alert_mutation_lock().lock().await;
    let mut watchlist = state.watchlist().read().await.clone();
    let mut alert_rules = state.alert_rules().read().await.clone();
    let watchlist_item = watchlist
        .iter()
        .find(|item| item.id == rule.watchlist_id)
        .cloned()
        .ok_or_else(|| AppError::NotFound(format!("watchlist: {}", rule.watchlist_id)))?;
    let mutation = insert_alert_rule(
        &mut alert_rules,
        rule,
        &watchlist_item,
        common::time::now_ms(),
        actor,
    );
    if mutation.changed {
        persist_rows(state, &mut watchlist, &mut alert_rules).await?;
        *state.watchlist().write().await = watchlist;
        *state.alert_rules().write().await = alert_rules.clone();
    }
    let rule = alert_rules
        .iter()
        .find(|rule| rule.id == mutation.rule.id)
        .cloned();
    Ok(AlertRuleMutationResponse {
        envelope: alert_rules_envelope(state, alert_rules),
        rule,
        changed: mutation.changed,
    })
}

pub(crate) async fn remove_alert_rule_by_id(
    state: &AppState,
    id: i64,
) -> Result<AlertRuleMutationResponse, AppError> {
    let _serial = state.watchlist_alert_mutation_lock().lock().await;
    let mut watchlist = state.watchlist().read().await.clone();
    let mut alert_rules = state.alert_rules().read().await.clone();
    let changed = remove_alert_rule(&mut alert_rules, id);
    if changed {
        persist_rows(state, &mut watchlist, &mut alert_rules).await?;
        *state.watchlist().write().await = watchlist;
        *state.alert_rules().write().await = alert_rules.clone();
    }
    Ok(AlertRuleMutationResponse {
        envelope: alert_rules_envelope(state, alert_rules),
        rule: None,
        changed,
    })
}

pub(crate) fn mark_persist_status(
    watchlist: &mut [WatchlistItem],
    alert_rules: &mut [AlertRule],
    status: WatchlistPersistStatus,
) {
    for item in watchlist {
        item.persistence.persist_status = status;
    }
    for rule in alert_rules {
        rule.persistence.persist_status = status;
    }
}

pub(crate) fn mark_alert_rule_persist_status(
    alert_rules: &mut [AlertRule],
    status: WatchlistPersistStatus,
) {
    for rule in alert_rules {
        rule.persistence.persist_status = status;
    }
}

async fn persist_rows(
    state: &AppState,
    watchlist: &mut [WatchlistItem],
    alert_rules: &mut [AlertRule],
) -> Result<(), AppError> {
    match state
        .watchlist_alert_store()
        .persist_snapshot(watchlist, alert_rules)
        .await
    {
        Ok(status) => {
            mark_persist_status(watchlist, alert_rules, status);
            Ok(())
        }
        Err(error) => {
            tracing::warn!(%error, "watchlist alert mutation rolled back after persist failure");
            Err(storage_unavailable(state))
        }
    }
}

fn storage_unavailable(state: &AppState) -> AppError {
    AppError::domain(
        StatusCode::SERVICE_UNAVAILABLE,
        "WATCHLIST_STORAGE_UNAVAILABLE",
        "watchlist alert storage is unavailable; mutation was not applied",
    )
    .with_details(serde_json::json!({
        "storage": state.watchlist_alert_store().health(),
    }))
}
