use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::state::AppState;
use tokio::time::MissedTickBehavior;
use tracing::debug;

const PRUNE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

pub(super) fn spawn_pruner(state: &AppState, tasks: &mut BackgroundTasks) {
    let state = state.clone();
    let shutdown = tasks.shutdown_token();
    tasks.supervise(
        "ws_housekeeping",
        PRUNE_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_pruner(state, shutdown).await }
        },
    );
}

async fn run_pruner(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let hub = state.ws_hub().clone();
    let mut tick = tokio::time::interval(PRUNE_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let removed = hub.prune_empty();
        if removed > 0 {
            debug!(removed, "pruned empty websocket channels");
        }
        registry.record_tick("ws_housekeeping");
    }
}
