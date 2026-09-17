use super::portfolio::trigger::{self, RefreshTrigger};
use super::tasks::{BackgroundTasks, ShutdownToken};
use crate::services::portfolio_pnl;
use crate::state::AppState;
use std::time::Duration;
use tokio::time::MissedTickBehavior;
use tracing::info;

const UPDATER_INTERVAL: Duration = Duration::from_secs(30);

pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "portfolio_pnl",
        UPDATER_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_updater(state, shutdown).await }
        },
    );
    info!(
        period_secs = UPDATER_INTERVAL.as_secs(),
        "portfolio PnL projection updater started"
    );
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    refresh_once(&state, &registry).await;

    let mut tick = tokio::time::interval(UPDATER_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.reset();
    loop {
        match trigger::next(&mut tick, state.portfolio_pnl_refresh_signal(), &shutdown).await {
            RefreshTrigger::Shutdown => break,
            RefreshTrigger::Event => {
                if !trigger::coalesce(state.portfolio_pnl_refresh_signal(), &shutdown).await {
                    break;
                }
                tick.reset();
            }
            RefreshTrigger::Interval => {}
        }
        refresh_once(&state, &registry).await;
    }
}

async fn refresh_once(state: &AppState, registry: &crate::task_registry::TaskRegistry) {
    let started_at_ms = common::time::now_ms();
    let snapshot = portfolio_pnl::snapshot(state, started_at_ms).await;
    state.cache_portfolio_pnl_snapshot(snapshot);
    state.request_portfolio_refresh();
    registry.record_result_timed("portfolio_pnl", started_at_ms, Ok(()));
}
