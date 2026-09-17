use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::state::AppState;
use crate::task_registry::TaskRegistry;
use tokio::time::MissedTickBehavior;
use tracing::info;

const TASK_NAME: &str = "venue_quality";
const UPDATER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);

pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let state = state.clone();
    let shutdown = tasks.shutdown_token();
    tasks.supervise(TASK_NAME, UPDATER_INTERVAL.as_millis() as i64, move || {
        let state = state.clone();
        let shutdown = shutdown.clone();
        async move { run_updater(state, shutdown).await }
    });

    info!(
        period_secs = UPDATER_INTERVAL.as_secs(),
        "venue quality updater started"
    );
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    let quality = state.venue_quality();
    let registry = state.task_registry();
    rebuild_snapshot(quality, registry);
    let mut tick = tokio::time::interval(UPDATER_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.tick().await;
    while tick_or_shutdown(&mut tick, &shutdown).await {
        rebuild_snapshot(quality, registry);
    }
}

fn rebuild_snapshot(quality: &realtime::VenueQualityTracker, registry: &TaskRegistry) {
    quality.rebuild_snapshot();
    registry.record_tick(TASK_NAME);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn venue_quality_uses_project_cadence() {
        assert_eq!(UPDATER_INTERVAL, std::time::Duration::from_secs(5));
    }

    #[test]
    fn venue_quality_rebuild_records_task_progress() {
        let quality = realtime::VenueQualityTracker::default();
        let registry = TaskRegistry::default();
        registry.register(TASK_NAME, UPDATER_INTERVAL.as_millis() as i64);
        let before = registry.task_snapshots(common::time::now_ms())[0].last_tick_ms;
        while common::time::now_ms() <= before {
            std::thread::yield_now();
        }

        rebuild_snapshot(&quality, &registry);

        let after = registry.task_snapshots(common::time::now_ms())[0].last_tick_ms;
        assert!(after > before);
    }
}
