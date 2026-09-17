use super::{run_task_once, BackgroundTasks, ShutdownToken};
use std::future::Future;
use std::time::{Duration, Instant};
use tracing::{info, warn};

const TASK_SHUTDOWN_GRACE: Duration = Duration::from_secs(1);
const WATCHER_SHUTDOWN_MARGIN: Duration = Duration::from_millis(250);

impl BackgroundTasks {
    /// Stop producers cooperatively, then abort and collect stragglers before
    /// the global budget expires so durable drains can report their real state.
    pub(crate) async fn shutdown(&mut self, drain_budget: Duration) -> bool {
        let _ = self.shutdown_tx.send(true);
        let started_at = Instant::now();
        let cooperative_budget =
            drain_budget.min(TASK_SHUTDOWN_GRACE.saturating_add(WATCHER_SHUTDOWN_MARGIN));
        if self.drain_within(cooperative_budget).await {
            info!("background tasks stopped within drain budget");
            return true;
        }
        self.abort_stragglers(started_at, drain_budget, cooperative_budget)
            .await
    }

    async fn abort_stragglers(
        &mut self,
        started_at: Instant,
        drain_budget: Duration,
        cooperative_budget: Duration,
    ) -> bool {
        let remaining = drain_budget.saturating_sub(started_at.elapsed());
        if remaining.is_zero() {
            self.force_abort(drain_budget).await;
            return false;
        }
        let stragglers = self.running_task_names();
        warn!(
            grace_ms = cooperative_budget.as_millis() as u64,
            remaining_ms = remaining.as_millis() as u64,
            ?stragglers,
            "background task watchers exceeded shutdown grace; aborting stragglers"
        );
        self.watchers.abort_all();
        self.finish_aborted_drain(drain_budget, remaining).await
    }

    async fn finish_aborted_drain(&mut self, drain_budget: Duration, remaining: Duration) -> bool {
        if self.drain_within(remaining).await {
            info!("background task stragglers aborted within drain budget");
            true
        } else {
            self.force_abort(drain_budget).await;
            false
        }
    }

    fn running_task_names(&self) -> Vec<&'static str> {
        self.registry
            .task_snapshots(common::time::now_ms())
            .into_iter()
            .filter(|snapshot| snapshot.running)
            .map(|snapshot| snapshot.name)
            .collect()
    }

    async fn drain_within(&mut self, drain_budget: Duration) -> bool {
        tokio::time::timeout(drain_budget, async {
            while self.watchers.join_next().await.is_some() {}
        })
        .await
        .is_ok()
    }

    async fn force_abort(&mut self, drain_budget: Duration) {
        warn!(
            budget_ms = drain_budget.as_millis() as u64,
            "drain budget exceeded; aborting remaining background tasks"
        );
        self.watchers.shutdown().await;
    }
}

pub(super) async fn run_task_until_shutdown<F, Fut>(
    name: &'static str,
    shutdown: &ShutdownToken,
    factory: &mut F,
) -> Option<Result<(), String>>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let task = run_task_once(factory);
    tokio::pin!(task);
    tokio::select! {
        biased;
        () = shutdown.cancelled() => {
            if tokio::time::timeout(TASK_SHUTDOWN_GRACE, &mut task).await.is_err() {
                warn!(
                    task = name,
                    grace_ms = TASK_SHUTDOWN_GRACE.as_millis() as u64,
                    "background task iteration cancelled after shutdown grace"
                );
            }
            None
        }
        outcome = &mut task => Some(outcome),
    }
}
