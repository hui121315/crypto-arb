use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::state::AppState;
use shared_types::ExecutionLedgerEvent;
use std::time::Duration;
use tracing::{info, warn};

const WORKER_INTERVAL: Duration = Duration::from_millis(250);
const PROJECTION_JOB_BATCH_LIMIT: usize = 128;
const PROJECTION_JOB_LEASE_MS: i64 = 30_000;
const RUN_COST_REBUILD_PAGE_SIZE: usize = 512;
const RETRY_BASE_MS: i64 = 500;
const RETRY_MAX_MS: i64 = 60_000;

pub(super) fn spawn_worker(state: &AppState, tasks: &mut BackgroundTasks) {
    if !sql_projection_configured(state) {
        tasks.register_disabled("ledger_projection_jobs", WORKER_INTERVAL.as_millis() as i64);
        return;
    }
    let state = state.clone();
    let shutdown = tasks.shutdown_token();
    tasks.supervise(
        "ledger_projection_jobs",
        WORKER_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_worker(state, shutdown).await }
        },
    );
    info!(
        interval_ms = WORKER_INTERVAL.as_millis() as u64,
        lease_ms = PROJECTION_JOB_LEASE_MS,
        "ledger projection job worker started"
    );
}

async fn run_worker(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    if let Err(error) = state
        .trading_service()
        .rebuild_run_cost_facts(RUN_COST_REBUILD_PAGE_SIZE)
        .await
    {
        warn!(%error, "startup run-cost fact rebuild remains pending");
    }
    let mut tick = tokio::time::interval(WORKER_INTERVAL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        registry.record_result_timed(
            "ledger_projection_jobs",
            started_at_ms,
            process_pending_jobs(&state).await.map(|_| ()),
        );
    }
}

pub(super) async fn persist_then_publish_ledger_projected_runs(
    state: &AppState,
    events: &[ExecutionLedgerEvent],
    execution_event_name: &'static str,
    close_event_name: &'static str,
) -> Result<(), String> {
    state
        .trading_service()
        .persist_ledger_event_group_durable(events)
        .await?;
    state.request_portfolio_pnl_refresh();
    if sql_projection_configured(state) {
        if let Err(error) = process_pending_jobs(state).await {
            warn!(%error, "ledger event committed; projection jobs remain pending for worker retry");
        }
        return Ok(());
    }
    for event in events {
        publish_execution_runs(state, event, execution_event_name);
        publish_close_runs(state, event, close_event_name);
    }
    Ok(())
}

async fn process_pending_jobs(state: &AppState) -> Result<usize, String> {
    let mut completed = 0usize;
    let mut errors = Vec::new();
    for projector in [
        trading::EXECUTION_RUN_PROJECTOR,
        trading::CLOSE_RUN_PROJECTOR,
        trading::RUN_COST_PROJECTOR,
    ] {
        match process_projector(state, projector).await {
            Ok(count) => completed = completed.saturating_add(count),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty() {
        Ok(completed)
    } else {
        Err(errors.join("; "))
    }
}

async fn process_projector(state: &AppState, projector: &'static str) -> Result<usize, String> {
    let now_ms = common::time::now_ms();
    let jobs = state
        .trading_service()
        .claim_sql_projection_jobs(
            projector,
            now_ms,
            PROJECTION_JOB_BATCH_LIMIT,
            PROJECTION_JOB_LEASE_MS,
        )
        .await?;
    let mut completed = 0usize;
    let mut errors = Vec::new();
    for job in jobs {
        match project_claimed_job(state, &job).await {
            Ok(projected) => match state
                .trading_service()
                .complete_sql_projection_job(&job, common::time::now_ms())
                .await
            {
                Ok(trading::SqlProjectionJobAck::Applied) => {
                    publish_projected_job(state, &job, &projected);
                    completed = completed.saturating_add(1);
                }
                Ok(trading::SqlProjectionJobAck::StaleClaim) => {}
                Err(error) => errors.push(format!("{} complete failed: {error}", job.event_id)),
            },
            Err(error) => {
                let available_at_ms = common::time::now_ms()
                    .saturating_add(projection_retry_delay_ms(job.attempt_count));
                if let Err(retry_error) = state
                    .trading_service()
                    .retry_sql_projection_job(&job, available_at_ms, &error)
                    .await
                {
                    errors.push(format!(
                        "{} projection failed: {error}; retry failed: {retry_error}",
                        job.event_id
                    ));
                } else {
                    errors.push(format!("{} projection failed: {error}", job.event_id));
                }
            }
        }
    }
    if errors.is_empty() {
        Ok(completed)
    } else {
        Err(errors.join("; "))
    }
}

enum ProjectedRuns {
    Execution(Vec<shared_types::ExecutionRun>),
    Close(Vec<shared_types::CloseRun>),
    RunCost,
}

async fn project_claimed_job(
    state: &AppState,
    job: &trading::SqlProjectionJob,
) -> Result<ProjectedRuns, String> {
    match job.projector.as_str() {
        trading::EXECUTION_RUN_PROJECTOR => {
            crate::services::execution_runs::project_ledger_event_update_durable(state, &job.event)
                .map(ProjectedRuns::Execution)
                .map_err(|error| error.to_string())
        }
        trading::CLOSE_RUN_PROJECTOR => {
            crate::services::close_runs::project_ledger_event_update_durable(state, &job.event)
                .map(ProjectedRuns::Close)
                .map_err(|error| error.to_string())
        }
        trading::RUN_COST_PROJECTOR => state
            .trading_service()
            .project_run_cost_event(&job.event)
            .await
            .map(|_| ProjectedRuns::RunCost),
        projector => Err(format!("unsupported ledger projector {projector}")),
    }
}

fn publish_projected_job(state: &AppState, job: &trading::SqlProjectionJob, runs: &ProjectedRuns) {
    match runs {
        ProjectedRuns::Execution(runs) => publish_execution_jobs(state, &job.event_id, runs),
        ProjectedRuns::Close(runs) => publish_close_jobs(state, &job.event_id, runs),
        ProjectedRuns::RunCost => {}
    }
}

fn publish_execution_jobs(state: &AppState, event_id: &str, runs: &[shared_types::ExecutionRun]) {
    for run in runs {
        if let Err(error) = crate::services::ws_publish::publish_execution_run_event(
            state,
            "ledger_projection_execution_event",
            run,
        ) {
            warn!(event_id, %error, "execution run job publish failed");
        }
    }
}

fn publish_close_jobs(state: &AppState, event_id: &str, runs: &[shared_types::CloseRun]) {
    for run in runs {
        if let Err(error) = crate::services::ws_publish::publish_close_run_event(
            state,
            "ledger_projection_close_event",
            run,
        ) {
            warn!(event_id, %error, "close run job publish failed");
        }
    }
}

fn sql_projection_configured(state: &AppState) -> bool {
    let snapshot = state.trading_service().sql_ledger_storage_snapshot();
    snapshot.migration.configured && snapshot.writer_configured
}

fn projection_retry_delay_ms(attempt_count: i32) -> i64 {
    let shift = attempt_count.saturating_sub(1).clamp(0, 7) as u32;
    RETRY_BASE_MS
        .saturating_mul(1_i64 << shift)
        .min(RETRY_MAX_MS)
}

fn publish_execution_runs(
    state: &AppState,
    event: &ExecutionLedgerEvent,
    event_name: &'static str,
) {
    for run in crate::services::execution_runs::project_ledger_event_update(state, event) {
        if let Err(error) =
            crate::services::ws_publish::publish_execution_run_event(state, event_name, &run)
        {
            warn!(%error, "execution run ledger publish failed");
        }
    }
}

fn publish_close_runs(state: &AppState, event: &ExecutionLedgerEvent, event_name: &'static str) {
    for run in crate::services::close_runs::project_ledger_event_update(state, event) {
        if let Err(error) =
            crate::services::ws_publish::publish_close_run_event(state, event_name, &run)
        {
            warn!(%error, "close run ledger publish failed");
        }
    }
}

#[cfg(test)]
#[path = "ledger_projection/tests.rs"]
mod tests;
