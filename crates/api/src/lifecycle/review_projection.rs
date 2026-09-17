use super::tasks::{BackgroundTasks, ShutdownToken};
use crate::services::review;
use crate::state::AppState;
use realtime::{channels, WsMessage};
use std::time::Duration;
use tokio::sync::watch;
use tokio::time::MissedTickBehavior;

const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);
const EVENT_COALESCE: Duration = Duration::from_millis(100);
const TASK_NAME: &str = "review_projection";

pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(TASK_NAME, RECOVERY_INTERVAL.as_millis() as i64, move || {
        let state = state.clone();
        let shutdown = shutdown.clone();
        async move { run_updater(state, shutdown).await }
    });
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    let mut sources = WakeSources::new(&state);
    refresh_and_record(&state).await;
    let mut recovery = tokio::time::interval(RECOVERY_INTERVAL);
    recovery.set_missed_tick_behavior(MissedTickBehavior::Skip);
    recovery.reset();

    loop {
        let event_wake = tokio::select! {
            biased;
            () = shutdown.cancelled() => break,
            changed = sources.changed() => {
                if !changed {
                    break;
                }
                true
            }
            _ = recovery.tick() => false,
        };
        if event_wake && !coalesce_event(&mut sources, &shutdown).await {
            break;
        }
        refresh_and_record(&state).await;
    }
}

async fn coalesce_event(sources: &mut WakeSources, shutdown: &ShutdownToken) -> bool {
    let keep_running = tokio::select! {
        biased;
        () = shutdown.cancelled() => false,
        () = tokio::time::sleep(EVENT_COALESCE) => true,
    };
    if keep_running {
        sources.mark_seen();
    }
    keep_running
}

async fn refresh_and_record(state: &AppState) {
    let started_at_ms = common::time::now_ms();
    let close_runs = review::close_run_snapshots(state.close_runs().as_ref());
    let snapshot =
        review::runtime_snapshot_from_trading(state.trading_service(), &close_runs).await;
    state.cache_review_snapshot(snapshot.clone());
    let result = publish_if_observed(state, &snapshot);
    state
        .task_registry()
        .record_result_timed(TASK_NAME, started_at_ms, result);
}

fn publish_if_observed(
    state: &AppState,
    snapshot: &shared_types::ReviewRuntimeSnapshot,
) -> Result<(), String> {
    if state.ws_hub().subscriber_count(channels::REVIEW) == 0 {
        return Ok(());
    }
    let message = WsMessage::json(snapshot).map_err(|error| error.to_string())?;
    state.ws_hub().publish_throttled(channels::REVIEW, message);
    Ok(())
}

struct WakeSources {
    orders: watch::Receiver<u64>,
    close_runs: watch::Receiver<u64>,
    pnl: watch::Receiver<u64>,
}

impl WakeSources {
    fn new(state: &AppState) -> Self {
        Self {
            orders: state.ws_hub().subscribe_activity(channels::ORDERS),
            close_runs: state
                .ws_hub()
                .subscribe_activity(channels::CLOSE_RUN_ACTIVITY),
            pnl: state.portfolio_pnl_snapshot().subscribe_updates(),
        }
    }

    async fn changed(&mut self) -> bool {
        tokio::select! {
            result = self.orders.changed() => result.is_ok(),
            result = self.close_runs.changed() => result.is_ok(),
            result = self.pnl.changed() => result.is_ok(),
        }
    }

    fn mark_seen(&mut self) {
        self.orders.borrow_and_update();
        self.close_runs.borrow_and_update();
        self.pnl.borrow_and_update();
    }
}
