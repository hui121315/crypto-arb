use super::tasks::{BackgroundTasks, ShutdownToken};
use crate::state::AppState;
use realtime::channels;
use shared_types::AutomationRuntimeStatus;
use std::future::Future;
use std::pin::Pin;
use tracing::info;

const RECOVERY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
type WorkerFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

pub(super) fn spawn_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "automated-arbitrage",
        RECOVERY_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            worker_future(state, shutdown)
        },
    );
    info!(
        recovery_period_secs = RECOVERY_INTERVAL.as_secs(),
        hot_path = "opportunity_automation_execution_activity",
        "automated arbitrage worker started"
    );
}

fn worker_future(state: AppState, shutdown: ShutdownToken) -> WorkerFuture {
    Box::pin(run_worker(state, shutdown))
}

async fn run_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut opportunities = state.opportunity_index().subscribe_updates();
    let mut automation = state.ws_hub().subscribe_activity(channels::AUTOMATION);
    let mut execution = state.ws_hub().subscribe_activity(channels::EXECUTION);
    record_evaluation(&state, &registry).await;
    loop {
        let now_ms = common::time::now_ms();
        let delay = next_wake_delay(&state.automation().snapshot(), now_ms);
        tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            _ = opportunities.changed() => {}
            _ = automation.changed() => {}
            _ = execution.changed() => {}
            _ = tokio::time::sleep(delay) => {}
        }
        record_evaluation(&state, &registry).await;
    }
}

async fn record_evaluation(state: &AppState, registry: &crate::task_registry::TaskRegistry) {
    let started_at_ms = common::time::now_ms();
    Box::pin(crate::services::automated_arbitrage::evaluate_once(
        state,
        started_at_ms,
    ))
    .await;
    registry.record_result_timed("automated-arbitrage", started_at_ms, Ok(()));
}

fn next_wake_delay(status: &AutomationRuntimeStatus, now_ms: i64) -> std::time::Duration {
    let Some(remaining_ms) = status
        .cooldown_until_ms
        .and_then(|deadline| deadline.checked_sub(now_ms))
        .filter(|remaining| *remaining > 0)
        .and_then(|remaining| u64::try_from(remaining).ok())
    else {
        return RECOVERY_INTERVAL;
    };
    RECOVERY_INTERVAL.min(std::time::Duration::from_millis(remaining_ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn supervised_worker_erases_the_large_execution_future() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;
        let tasks = BackgroundTasks::new(state.task_registry().clone());
        let future = worker_future(state, tasks.shutdown_token());

        assert!(std::mem::size_of_val(&future) <= 16);
        Ok(())
    }

    #[test]
    fn cooldown_deadline_wakes_worker_without_periodic_polling() {
        let status = AutomationRuntimeStatus {
            cooldown_until_ms: Some(2_500),
            ..Default::default()
        };

        assert_eq!(
            next_wake_delay(&status, 1_000),
            std::time::Duration::from_millis(1_500)
        );
        assert_eq!(next_wake_delay(&status, 3_000), RECOVERY_INTERVAL);
    }
}
