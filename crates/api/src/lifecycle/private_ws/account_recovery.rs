use super::super::tasks::{BackgroundTasks, ShutdownToken};
use crate::services::private_account_refresh::PrivateAccountRefresh;
use crate::state::AppState;
use crate::trading_service::TradingService;
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::warn;

const TASK_NAME: &str = "private_account_refresh";
const ACCOUNT_RECOVERY_COALESCE_WINDOW: Duration = Duration::from_millis(100);
const FOLLOWUP_ACCOUNT_WS_SETTLE_WINDOW: Duration = Duration::from_millis(250);
const WORKER_HEARTBEAT: Duration = Duration::from_secs(30);

pub(super) fn spawn_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let state = state.clone();
    let shutdown = tasks.shutdown_token();
    tasks.supervise(TASK_NAME, WORKER_HEARTBEAT.as_millis() as i64, move || {
        let state = state.clone();
        let shutdown = shutdown.clone();
        async move { run_worker(state, shutdown).await }
    });
}

async fn run_worker(state: AppState, shutdown: ShutdownToken) {
    let mut heartbeat = tokio::time::interval(WORKER_HEARTBEAT);
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return,
            _ = heartbeat.tick() => state.task_registry().record_tick(TASK_NAME),
            () = state.private_account_refresh_queue().notified() => {
                if state.private_account_refresh_queue().is_empty() {
                    continue;
                }
                if !wait_for_coalesce(&shutdown).await {
                    return;
                }
                if !recover_queued_accounts(&state, &shutdown).await {
                    return;
                }
                state.task_registry().record_tick(TASK_NAME);
            }
        }
    }
}

async fn wait_for_coalesce(shutdown: &ShutdownToken) -> bool {
    tokio::select! {
        biased;
        () = shutdown.cancelled() => false,
        () = tokio::time::sleep(ACCOUNT_RECOVERY_COALESCE_WINDOW) => true,
    }
}

async fn recover_queued_accounts(state: &AppState, shutdown: &ShutdownToken) -> bool {
    let (followup_ws, immediate): (Vec<_>, Vec<_>) = state
        .private_account_refresh_queue()
        .drain()
        .into_iter()
        .partition(|refresh| refresh.wait_for_followup_ws);
    let mut account_cache_refreshed = recover_account_refreshes(state, immediate).await;
    if followup_ws.is_empty() {
        if account_cache_refreshed {
            state.request_portfolio_refresh();
        }
        return true;
    }
    if !wait_for_followup_account_ws(shutdown).await {
        return false;
    }
    account_cache_refreshed |= recover_account_refreshes(state, followup_ws).await;
    if account_cache_refreshed {
        state.request_portfolio_refresh();
    }
    true
}

async fn wait_for_followup_account_ws(shutdown: &ShutdownToken) -> bool {
    tokio::select! {
        biased;
        () = shutdown.cancelled() => false,
        () = tokio::time::sleep(FOLLOWUP_ACCOUNT_WS_SETTLE_WINDOW) => true,
    }
}

async fn recover_account_refreshes(
    state: &AppState,
    refreshes: Vec<PrivateAccountRefresh>,
) -> bool {
    let refreshes = unresolved_account_refreshes(state.trading_service(), refreshes);
    if refreshes.is_empty() {
        return false;
    }
    let position_venues = refreshes
        .iter()
        .filter(|refresh| refresh.scope.invalidates_positions())
        .map(|refresh| refresh.venue.clone())
        .collect::<Vec<_>>();
    let balance_venues = refreshes
        .iter()
        .filter(|refresh| refresh.scope.invalidates_balances())
        .map(|refresh| refresh.venue.clone())
        .collect::<Vec<_>>();
    let service = state.trading_service();
    let positions = refresh_positions(service, &position_venues);
    let balances = refresh_balances(service, &balance_venues);
    let (positions_refreshed, balances_refreshed) = tokio::join!(positions, balances);
    positions_refreshed || balances_refreshed
}

fn unresolved_account_refreshes(
    service: &TradingService,
    refreshes: Vec<PrivateAccountRefresh>,
) -> Vec<PrivateAccountRefresh> {
    let now_ms = common::time::now_ms();
    refreshes
        .into_iter()
        .filter_map(|refresh| {
            service
                .unresolved_private_account_scope(&refresh.venue, refresh.scope, now_ms)
                .map(|scope| PrivateAccountRefresh {
                    venue: refresh.venue,
                    scope,
                    wait_for_followup_ws: refresh.wait_for_followup_ws,
                })
        })
        .collect()
}

async fn refresh_positions(service: &TradingService, venues: &[String]) -> bool {
    if venues.is_empty() {
        return false;
    }
    match service.refresh_scoped_positions(venues).await {
        Ok(_) => true,
        Err(error) => {
            warn!(?venues, %error, "private ws position cache refresh failed");
            false
        }
    }
}

async fn refresh_balances(service: &TradingService, venues: &[String]) -> bool {
    if venues.is_empty() {
        return false;
    }
    match service.list_scoped_balances(venues).await {
        Ok(_) => true,
        Err(error) => {
            warn!(?venues, %error, "private ws balance cache refresh failed");
            false
        }
    }
}
