use super::tasks::{BackgroundTasks, ShutdownToken};
use crate::state::AppState;

const IDLE_HEARTBEAT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
const MAX_EXPECTED_DELIVERY_MS: i64 = 60_000;

pub(super) fn spawn_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    if let Err(error) = crate::services::webhook::restore_config(state) {
        tracing::warn!(%error, "persisted webhook configuration could not be restored");
    }
    let dispatcher = std::sync::Arc::clone(state.webhook());
    let registry = state.task_registry().clone();
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise("webhook-delivery", MAX_EXPECTED_DELIVERY_MS, move || {
        let dispatcher = std::sync::Arc::clone(&dispatcher);
        let registry = registry.clone();
        let shutdown = shutdown.clone();
        let state = state.clone();
        async move { run_worker(state, dispatcher, registry, shutdown).await }
    });
}

async fn run_worker(
    state: AppState,
    dispatcher: std::sync::Arc<webhook::WebhookDispatcher>,
    registry: crate::task_registry::TaskRegistry,
    shutdown: ShutdownToken,
) {
    let started_at_ms = common::time::now_ms();
    registry.record_result_timed("webhook-delivery", started_at_ms, Ok::<(), String>(()));
    let mut last_fingerprint = None;
    crate::services::webhook::publish_status_if_changed(&state, &mut last_fingerprint).await;
    loop {
        let started_at_ms = common::time::now_ms();
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return,
            result = dispatcher.process_next(IDLE_HEARTBEAT_INTERVAL) => {
                let failed = result.is_err();
                registry.record_result_timed(
                    "webhook-delivery",
                    started_at_ms,
                    result.map(|_| ()).map_err(|error| error.to_string()),
                );
                // 每次投递尝试后推送终态：队列深度、成功/失败/丢弃计数和最近投递记录
                // 都在这里变化，空闲心跳不会产生重复推送。
                crate::services::webhook::publish_status_if_changed(&state, &mut last_fingerprint)
                    .await;
                if failed {
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn idle_worker_records_progress_while_queue_is_empty() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;
        let registry = state.task_registry().clone();
        let mut tasks = BackgroundTasks::new(registry.clone());
        spawn_worker(&state, &mut tasks);

        tokio::time::timeout(std::time::Duration::from_millis(250), async {
            loop {
                let ready = registry
                    .task_snapshots(common::time::now_ms())
                    .into_iter()
                    .find(|snapshot| snapshot.name == "webhook-delivery")
                    .is_some_and(|snapshot| snapshot.last_success_ms.is_some());
                if ready {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await?;

        let snapshot = registry
            .task_snapshots(common::time::now_ms())
            .into_iter()
            .find(|snapshot| snapshot.name == "webhook-delivery")
            .ok_or_else(|| anyhow::anyhow!("webhook delivery task was not registered"))?;
        assert!(snapshot.last_success_ms.is_some());
        assert!(snapshot.issue.is_none());
        assert!(tasks.shutdown(std::time::Duration::from_secs(2)).await);
        Ok(())
    }
}
