use super::tasks::{delay_or_shutdown, tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::services::trading_credentials;
use crate::state::AppState;
use crate::trading_service::PrivateFundingPaymentIngestReport;
use tracing::{info, warn};

const UPDATER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5 * 60);
const COLD_START_DELAY: std::time::Duration = std::time::Duration::from_secs(45);
const LOOKBACK_MS: i64 = 9 * 60 * 60 * 1_000;

pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let state = state.clone();
    let interval = UPDATER_INTERVAL;
    let shutdown = tasks.shutdown_token();
    tasks.supervise(
        "private_funding_payments",
        interval.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_updater(state, shutdown).await }
        },
    );

    info!(
        period_secs = interval.as_secs(),
        lookback_hours = LOOKBACK_MS / 3_600_000,
        "private funding payments updater started"
    );
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    if !delay_or_shutdown(COLD_START_DELAY, &shutdown).await {
        return;
    }
    let registry = state.task_registry().clone();
    let interval = UPDATER_INTERVAL;
    let started_at_ms = common::time::now_ms();
    let report = run_once(&state).await;
    registry.record_result_timed(
        "private_funding_payments",
        started_at_ms,
        funding_payments_outcome(&report),
    );

    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        let report = run_once(&state).await;
        registry.record_result_timed(
            "private_funding_payments",
            started_at_ms,
            funding_payments_outcome(&report),
        );
    }
}

async fn run_once(state: &AppState) -> PrivateFundingPaymentIngestReport {
    let end_time_ms = common::time::now_ms();
    let start_time_ms = end_time_ms.saturating_sub(LOOKBACK_MS);
    let batch = state
        .trading_service()
        .ingest_configured_private_funding_payments(
            trading_credentials::current_adapter_credentials(),
            Some(start_time_ms),
            Some(end_time_ms),
        )
        .await;
    let mut report = batch.report;
    if !batch.ledger_events.is_empty() {
        if let Err(error) = super::ledger_projection::persist_then_publish_ledger_projected_runs(
            state,
            &batch.ledger_events,
            "funding_payment_execution_event",
            "funding_payment_close_event",
        )
        .await
        {
            report = state
                .trading_service()
                .record_private_funding_payment_storage_error(report, &error);
        }
    }
    log_report(&report);
    report
}

fn funding_payments_outcome(report: &PrivateFundingPaymentIngestReport) -> Result<(), String> {
    if let Some(error) = &report.fetch_error {
        return Err(error.clone());
    }
    Ok(())
}

fn log_report(report: &PrivateFundingPaymentIngestReport) {
    if !report.is_success() || report.route_failures > 0 {
        log_degraded_report(report);
    } else {
        log_success_report(report);
    }
}

fn log_degraded_report(report: &PrivateFundingPaymentIngestReport) {
    warn!(
        fetched = report.fetched,
        mapped = report.mapped,
        ledger_events = report.ledger_events,
        skipped = report.skipped,
        invalid = report.invalid,
        route_failures = report.route_failures,
        unsupported = report.unsupported,
        fetch_error = ?report.fetch_error,
        "private funding payment ingestion degraded"
    );
}

fn log_success_report(report: &PrivateFundingPaymentIngestReport) {
    info!(
        fetched = report.fetched,
        mapped = report.mapped,
        ledger_events = report.ledger_events,
        skipped = report.skipped,
        invalid = report.invalid,
        unsupported = report.unsupported,
        "private funding payment ingestion done"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_failure_keeps_global_ingestion_task_healthy() {
        let report = PrivateFundingPaymentIngestReport {
            route_failures: 1,
            ..PrivateFundingPaymentIngestReport::default()
        };

        assert!(funding_payments_outcome(&report).is_ok());
    }

    #[test]
    fn global_fetch_error_marks_ingestion_task_unhealthy() {
        let report = PrivateFundingPaymentIngestReport {
            fetch_error: Some("funding ledger unavailable".to_owned()),
            ..PrivateFundingPaymentIngestReport::default()
        };

        assert_eq!(
            funding_payments_outcome(&report),
            Err("funding ledger unavailable".to_owned())
        );
    }
}
