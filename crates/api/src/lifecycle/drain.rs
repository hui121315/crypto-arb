use super::BackgroundTasks;
use crate::state::AppState;
use std::time::Duration;
use tracing::{error, info};

const SHUTDOWN_DRAIN_BUDGET: Duration = Duration::from_secs(5);
const SQL_LEDGER_DRAIN_BUDGET: Duration = Duration::from_secs(5);
const AUDIT_DRAIN_BUDGET: Duration = Duration::from_secs(5);

const COMPONENT_BACKGROUND_TASKS: &str = "background_tasks";
const COMPONENT_PORTFOLIO_NAV: &str = "portfolio_nav";
const COMPONENT_HISTORY_STORE: &str = "history_store";
const COMPONENT_TRADING_SQL_JOURNAL: &str = "trading_sql_journal";
const COMPONENT_AUDIT_LOG: &str = "audit_log_jsonl";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DrainStatus {
    Drained,
    NoPendingBuffer,
    TimedOut,
    Failed,
}

impl DrainStatus {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Drained => "drained",
            Self::NoPendingBuffer => "no_pending_buffer",
            Self::TimedOut => "timed_out",
            Self::Failed => "failed",
        }
    }

    const fn is_clean(self) -> bool {
        matches!(self, Self::Drained | Self::NoPendingBuffer)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DrainOutcome {
    pub(crate) component: &'static str,
    pub(crate) status: DrainStatus,
    pub(crate) detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ShutdownDrainReport {
    pub(crate) outcomes: Vec<DrainOutcome>,
}

impl ShutdownDrainReport {
    fn new(outcomes: Vec<DrainOutcome>) -> Self {
        Self { outcomes }
    }

    pub(crate) fn ensure_clean(&self) -> anyhow::Result<()> {
        let failures = self
            .outcomes
            .iter()
            .filter(|outcome| !outcome.status.is_clean())
            .map(|outcome| {
                format!(
                    "{}={} ({})",
                    outcome.component,
                    outcome.status.as_str(),
                    outcome.detail
                )
            })
            .collect::<Vec<_>>();
        if failures.is_empty() {
            return Ok(());
        }
        anyhow::bail!("server shutdown failed: {}", failures.join("; "))
    }

    fn log(&self) {
        self.outcomes
            .iter()
            .filter(|outcome| outcome.status.is_clean())
            .for_each(log_clean_outcome);
        self.outcomes
            .iter()
            .filter(|outcome| !outcome.status.is_clean())
            .for_each(log_unclean_outcome);
    }
}

fn log_clean_outcome(outcome: &DrainOutcome) {
    info!(
        component = outcome.component,
        status = outcome.status.as_str(),
        detail = %outcome.detail,
        "shutdown drain outcome"
    );
}

fn log_unclean_outcome(outcome: &DrainOutcome) {
    error!(
        component = outcome.component,
        status = outcome.status.as_str(),
        detail = %outcome.detail,
        "shutdown drain outcome"
    );
}

pub(crate) async fn drain_runtime(
    state: &AppState,
    tasks: &mut BackgroundTasks,
) -> ShutdownDrainReport {
    let producers_drained = tasks.shutdown(SHUTDOWN_DRAIN_BUDGET).await;
    let nav_enabled = state
        .portfolio_nav_storage_health()
        .snapshot(common::time::now_ms())
        .enabled;
    let history_backend = state.history_store().backend_name();
    let sql_configured = state
        .trading_service()
        .sql_ledger_storage_snapshot()
        .writer_configured;
    let audit_configured =
        crate::middleware::audit::health_snapshot(common::time::now_ms()).configured;

    let outcomes = vec![
        producer_outcome(producers_drained),
        synchronous_store_outcome(
            COMPONENT_PORTFOLIO_NAV,
            producers_drained,
            format!("enabled={nav_enabled}; awaited SQLite append has no writer queue"),
        ),
        synchronous_store_outcome(
            COMPONENT_HISTORY_STORE,
            producers_drained,
            format!("backend={history_backend}; awaited store calls have no writer queue"),
        ),
        drain_sql_journal(state, sql_configured).await,
        drain_audit_writer(audit_configured).await,
    ];
    let report = ShutdownDrainReport::new(outcomes);
    report.log();
    report
}

fn producer_outcome(drained: bool) -> DrainOutcome {
    if drained {
        return DrainOutcome {
            component: COMPONENT_BACKGROUND_TASKS,
            status: DrainStatus::Drained,
            detail: "all supervised producers exited within the bounded drain".to_owned(),
        };
    }
    DrainOutcome {
        component: COMPONENT_BACKGROUND_TASKS,
        status: DrainStatus::TimedOut,
        detail: format!(
            "producer drain exceeded {}ms; remaining tasks were aborted",
            SHUTDOWN_DRAIN_BUDGET.as_millis()
        ),
    }
}

fn synchronous_store_outcome(
    component: &'static str,
    producers_drained: bool,
    detail: String,
) -> DrainOutcome {
    if producers_drained {
        return DrainOutcome {
            component,
            status: DrainStatus::NoPendingBuffer,
            detail,
        };
    }
    DrainOutcome {
        component,
        status: DrainStatus::TimedOut,
        detail: format!(
            "producer drain failed; in-flight synchronous write is unconfirmed; {detail}"
        ),
    }
}

async fn drain_sql_journal(state: &AppState, configured: bool) -> DrainOutcome {
    let result = tokio::time::timeout(
        SQL_LEDGER_DRAIN_BUDGET,
        state.trading_service().drain_sql_ledger_and_shutdown(),
    )
    .await;
    match result {
        Ok(Ok(())) if configured => clean_outcome(
            COMPONENT_TRADING_SQL_JOURNAL,
            DrainStatus::Drained,
            "SQL journal queue drained and writer stopped",
        ),
        Ok(Ok(())) => clean_outcome(
            COMPONENT_TRADING_SQL_JOURNAL,
            DrainStatus::NoPendingBuffer,
            "SQL journal writer is not configured",
        ),
        Ok(Err(error)) => failed_outcome(COMPONENT_TRADING_SQL_JOURNAL, error),
        Err(_) => DrainOutcome {
            component: COMPONENT_TRADING_SQL_JOURNAL,
            status: DrainStatus::TimedOut,
            detail: format!(
                "SQL journal drain exceeded {}ms",
                SQL_LEDGER_DRAIN_BUDGET.as_millis()
            ),
        },
    }
}

async fn drain_audit_writer(configured: bool) -> DrainOutcome {
    let result =
        tokio::task::spawn_blocking(|| crate::middleware::audit::shutdown(AUDIT_DRAIN_BUDGET))
            .await
            .map_err(|error| format!("audit shutdown worker failed: {error}"))
            .and_then(|result| result);
    match result {
        Ok(()) if configured => clean_outcome(
            COMPONENT_AUDIT_LOG,
            DrainStatus::Drained,
            "audit JSONL queue drained and writer stopped",
        ),
        Ok(()) => clean_outcome(
            COMPONENT_AUDIT_LOG,
            DrainStatus::NoPendingBuffer,
            "audit JSONL writer is not configured",
        ),
        Err(error) if is_timeout_error(&error) => DrainOutcome {
            component: COMPONENT_AUDIT_LOG,
            status: DrainStatus::TimedOut,
            detail: error,
        },
        Err(error) => failed_outcome(COMPONENT_AUDIT_LOG, error),
    }
}

fn clean_outcome(
    component: &'static str,
    status: DrainStatus,
    detail: impl Into<String>,
) -> DrainOutcome {
    DrainOutcome {
        component,
        status,
        detail: detail.into(),
    }
}

fn failed_outcome(component: &'static str, detail: impl Into<String>) -> DrainOutcome {
    DrainOutcome {
        component,
        status: DrainStatus::Failed,
        detail: detail.into(),
    }
}

fn is_timeout_error(error: &str) -> bool {
    error.contains("timeout") || error.contains("timed out")
}

#[cfg(test)]
mod tests;
