use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::state::AppState;
use tokio::time::MissedTickBehavior;

const REFRESH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);

pub(super) fn spawn_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "gate-crossex-mode",
        REFRESH_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_worker(state, shutdown).await }
        },
    );
}

async fn run_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(REFRESH_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        state
            .gate_crossex_mode()
            .refresh(
                state.aggregator(),
                state.instrument_registry(),
                started_at_ms,
            )
            .await;
        registry.record_result_timed("gate-crossex-mode", started_at_ms, Ok::<(), String>(()));
    }
}
