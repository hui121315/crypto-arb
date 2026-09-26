use super::*;

#[cfg(test)]
pub(super) async fn apply_events(
    state: &AppState,
    venue: &'static str,
    events: Vec<crate::trading_service::private_ws_events::PrivateWsEvent>,
) {
    let outcomes =
        crate::trading_service::private_ws_mapper::apply_events(state.trading_service(), events)
            .await;
    project_outcomes(state, venue, outcomes, None).await;
}

pub(super) async fn apply_events_in_session(
    state: &AppState,
    venue: &'static str,
    events: Vec<crate::trading_service::private_ws_events::PrivateWsEvent>,
    session: &PrivateWsSession,
    account: tokio::sync::MutexGuard<'_, ()>,
) {
    let outcomes =
        crate::trading_service::private_ws_mapper::apply_events(state.trading_service(), events)
            .await;
    // Cache/ledger association is atomic with credential replacement; SQL must not block the control lock.
    drop(account);
    project_outcomes(state, venue, outcomes, Some(session)).await;
}

async fn project_outcomes(
    state: &AppState,
    venue: &'static str,
    outcomes: Vec<crate::trading_service::private_ws_events::PrivateWsApplyOutcome>,
    session: Option<&PrivateWsSession>,
) {
    let mut account_cache_updated = false;
    let mut open_order_cache_updated = false;
    let mut dirty = Vec::new();
    for outcome in outcomes {
        let durability = persist_ledger_events(state, &outcome).await;
        let _account = match session {
            Some(session) => {
                let Some(guard) = session.lock(state).await else {
                    continue;
                };
                Some(guard)
            }
            None => None,
        };
        let Some(outcome) = apply_outcome(state, venue, outcome, durability) else {
            continue;
        };
        account_cache_updated |= outcome.account_cache_updated;
        open_order_cache_updated |= outcome.open_order_cache_updated;
        dirty.extend(outcome.account_cache_dirty);
    }
    let _account = match session {
        Some(session) => {
            let Some(guard) = session.lock(state).await else {
                return;
            };
            Some(guard)
        }
        None => None,
    };
    project_account_batch(
        state,
        account_cache_updated,
        open_order_cache_updated,
        dirty,
    );
}

fn apply_outcome(
    state: &AppState,
    venue: &'static str,
    outcome: crate::trading_service::private_ws_events::PrivateWsApplyOutcome,
    durability: Result<(), String>,
) -> Option<crate::trading_service::private_ws_events::PrivateWsApplyOutcome> {
    if let Err(error) = durability {
        state
            .private_ws_health()
            .record_apply_durability_failure(venue, &outcome, &error);
        let event_ids = outcome
            .ledger_events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>();
        warn!(venue, ?event_ids, %error, "private ws ledger durability gate failed");
        return None;
    }
    state
        .private_ws_health()
        .record_apply_outcome(venue, &outcome);
    if let Some(record) = outcome.order.as_ref() {
        publish_order(state, record, outcome.order_projection_handled_by_ledger);
    }
    Some(outcome)
}

fn project_account_batch(
    state: &AppState,
    account_cache_updated: bool,
    open_order_cache_updated: bool,
    dirty: Vec<crate::trading_service::private_ws_events::PrivateAccountDirty>,
) {
    let dirty = unresolved_account_dirty(state.trading_service(), dirty);
    let account_recovery_pending = !dirty.is_empty();
    if should_request_portfolio_refresh(
        account_cache_updated,
        open_order_cache_updated,
        account_recovery_pending,
    ) {
        state.request_portfolio_refresh();
    }
    state.private_account_refresh_queue().enqueue(dirty);
}

fn unresolved_account_dirty(
    service: &crate::trading_service::TradingService,
    dirty: Vec<crate::trading_service::private_ws_events::PrivateAccountDirty>,
) -> Vec<crate::trading_service::private_ws_events::PrivateAccountDirty> {
    let now_ms = common::time::now_ms();
    dirty
        .into_iter()
        .filter_map(|mut dirty| {
            service
                .unresolved_private_account_scope(&dirty.venue, dirty.scope, now_ms)
                .map(|scope| {
                    dirty.scope = scope;
                    dirty
                })
        })
        .collect()
}

const fn should_request_portfolio_refresh(
    account_cache_updated: bool,
    open_order_cache_updated: bool,
    account_recovery_pending: bool,
) -> bool {
    !account_recovery_pending && (account_cache_updated || open_order_cache_updated)
}

async fn persist_ledger_events(
    state: &AppState,
    outcome: &crate::trading_service::private_ws_events::PrivateWsApplyOutcome,
) -> Result<(), String> {
    if outcome.ledger_events.is_empty() {
        return if outcome.ledger_updated {
            Err("private WS ledger outcome reported an update without events".to_owned())
        } else {
            Ok(())
        };
    }
    super::super::ledger_projection::persist_then_publish_ledger_projected_runs(
        state,
        &outcome.ledger_events,
        "private_ws_fill_event",
        "private_ws_close_ledger_event",
    )
    .await
}

fn publish_order(
    state: &AppState,
    record: &shared_types::OrderRecord,
    projection_handled_by_ledger: bool,
) {
    let publish = if projection_handled_by_ledger {
        crate::services::ws_publish::publish_order_record_event
    } else {
        crate::services::ws_publish::publish_order_event
    };
    if let Err(error) = publish(state, "private_ws_order_update", record) {
        warn!(%error, "private ws order publish failed");
    }
}

#[cfg(test)]
mod tests {
    use super::should_request_portfolio_refresh;

    #[test]
    fn account_patch_projects_immediately() {
        assert!(should_request_portfolio_refresh(true, true, false));
    }

    #[test]
    fn partial_account_patch_waits_for_unresolved_scope() {
        assert!(!should_request_portfolio_refresh(true, true, true));
    }

    #[test]
    fn terminal_order_waits_for_account_recovery_before_full_snapshot() {
        assert!(!should_request_portfolio_refresh(false, true, true));
    }

    #[test]
    fn open_order_only_update_projects_immediately() {
        assert!(should_request_portfolio_refresh(false, true, false));
    }

    #[test]
    fn dirty_account_without_fresh_cache_does_not_project() {
        assert!(!should_request_portfolio_refresh(false, false, true));
    }
}
