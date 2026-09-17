use super::tasks::{delay_or_shutdown, tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::services::{
    close_runs::{self, AutoCompensationOutcome},
    reconciliation_health::ReconciliationHealthCycle,
    run_finality::{self, RunFinalityOutcome},
};
use crate::state::AppState;
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

const RECONCILE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
const COLD_START_DELAY: std::time::Duration = std::time::Duration::from_secs(8);

pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "reconciliation",
        RECONCILE_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_updater(state, shutdown).await }
        },
    );
    info!(
        period_secs = RECONCILE_INTERVAL.as_secs(),
        "order reconciliation updater started"
    );
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    if !delay_or_shutdown(COLD_START_DELAY, &shutdown).await {
        return;
    }
    let registry = state.task_registry().clone();
    let mut tick = tokio::time::interval(RECONCILE_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        let result = reconcile_once(&state).await;
        registry.record_result_timed("reconciliation", started_at_ms, result);
    }
}

async fn reconcile_once(state: &AppState) -> Result<(), String> {
    let cycle = ReconciliationHealthCycle::from_orders(&state.trading_service().list_orders());
    let result = state
        .trading_service()
        .reconcile_and_refresh_missing_orders()
        .await;
    // A remote read failure is recorded in reconciliation health and must stay
    // visible to the product, but it is not a lifecycle-task crash. Keeping
    // this loop healthy lets the next bounded retry and run-finality pass run.
    handle_reconcile_result(state, &cycle, result);
    let finality = run_finality::refresh_pending_runs(state).await;
    state.run_finality_health().record_outcome(&finality);
    handle_run_finality_outcome(&finality);
    let auto_compensation = close_runs::auto_submit_compensation_once(state).await;
    handle_auto_compensation_outcome(&auto_compensation);
    Ok(())
}

fn handle_reconcile_result(
    state: &AppState,
    cycle: &ReconciliationHealthCycle,
    result: Result<crate::trading_service::ReconcileOutcome, exchange::ExchangeError>,
) {
    match result {
        Ok(outcome) => {
            state.reconciliation_health().record_success(
                cycle,
                &outcome.refreshed,
                outcome.diffs.len(),
                &outcome.refresh_failures,
            );
            publish_reconcile_outcome(state, &outcome);
        }
        Err(error) => {
            let message = error.to_string();
            state
                .reconciliation_health()
                .record_failure(cycle, &message);
            warn!(%error, "order reconciliation failed");
        }
    }
}

fn publish_reconcile_outcome(state: &AppState, outcome: &crate::trading_service::ReconcileOutcome) {
    publish_refreshed_orders(state, &outcome.refreshed);
    if outcome.diffs.is_empty() {
        debug!("order reconciliation clear");
        return;
    }
    publish_reconcile_diff(state, &outcome.diffs);
}

fn publish_reconcile_diff(state: &AppState, diffs: &[trading::ReconcileDiff]) {
    if let Err(error) =
        crate::services::ws_publish::publish_reconcile_event(state, "order_reconcile_diff", diffs)
    {
        warn!(%error, "order reconciliation event publish failed");
    }
}

fn publish_refreshed_orders(state: &AppState, rows: &[shared_types::OrderRecord]) {
    for record in rows {
        if let Err(error) = crate::services::ws_publish::publish_order_event(
            state,
            "order_reconcile_status_backfilled",
            record,
        ) {
            warn!(%error, order_id = %record.intent.id, "reconciled order publish failed");
        }
    }
}

fn handle_run_finality_outcome(outcome: &RunFinalityOutcome) {
    if outcome.scanned_order_count == 0 {
        return;
    }
    if outcome.has_failures() {
        warn_run_finality_outcome(outcome);
        return;
    }
    debug!(
        scanned = outcome.scanned_order_count,
        refreshed = outcome.refreshed_order_count,
        remote_missing = outcome.remote_missing_count,
        missing_local = outcome.skipped_missing_local_count,
        terminal = outcome.skipped_terminal_count,
        "run finality refresh completed"
    );
}

fn warn_run_finality_outcome(outcome: &RunFinalityOutcome) {
    warn!(
        scanned = outcome.scanned_order_count,
        refreshed = outcome.refreshed_order_count,
        refresh_failures = outcome.refresh_failure_count,
        publish_failures = outcome.publish_failure_count,
        remote_missing = outcome.remote_missing_count,
        missing_local = outcome.skipped_missing_local_count,
        terminal = outcome.skipped_terminal_count,
        "run finality refresh completed with failures"
    );
}

fn handle_auto_compensation_outcome(outcome: &AutoCompensationOutcome) {
    if !outcome.has_activity() {
        return;
    }
    if outcome.has_failures() {
        warn_auto_compensation_outcome(outcome);
        return;
    }
    debug_auto_compensation_outcome(outcome);
}

fn warn_auto_compensation_outcome(outcome: &AutoCompensationOutcome) {
    warn!(
        scanned = outcome.scanned_run_count,
        eligible = outcome.eligible_run_count,
        submitted = outcome.submitted_count,
        skipped_multi_candidate = outcome.skipped_multi_candidate_count,
        skipped_live = outcome.skipped_live_count,
        submission_failures = outcome.submission_failure_count,
        publish_failures = outcome.publish_failure_count,
        "close-run auto compensation completed with blocked submissions"
    );
}

fn debug_auto_compensation_outcome(outcome: &AutoCompensationOutcome) {
    debug!(
        scanned = outcome.scanned_run_count,
        eligible = outcome.eligible_run_count,
        submitted = outcome.submitted_count,
        skipped_multi_candidate = outcome.skipped_multi_candidate_count,
        skipped_live = outcome.skipped_live_count,
        "close-run auto compensation completed"
    );
}
