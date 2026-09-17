use super::tasks::{BackgroundTasks, ShutdownToken};
use crate::services::{portfolio, portfolio_snapshot_envelope, runtime_problem};
use crate::state::AppState;
use shared_types::{PortfolioSnapshot, PortfolioSnapshotEnvelope};
use tokio::time::MissedTickBehavior;
use tracing::{info, warn};

pub(super) mod trigger;

const UPDATER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
const OP_PORTFOLIO_SNAPSHOT: &str = "snapshot";

struct PublishOutcome {
    envelope: PortfolioSnapshotEnvelope,
    task_error: Option<String>,
}

pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "portfolio",
        UPDATER_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_updater(state, shutdown).await }
        },
    );

    info!(
        period_secs = UPDATER_INTERVAL.as_secs(),
        "portfolio updater started"
    );
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut last_snapshot = state.portfolio_snapshot().value_now();
    let started_at_ms = common::time::now_ms();
    let result = publish_once(&state, &mut last_snapshot).await;
    registry.record_result_timed("portfolio", started_at_ms, result);

    let mut tick = tokio::time::interval(UPDATER_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    tick.reset();
    loop {
        match trigger::next(&mut tick, state.portfolio_refresh_signal(), &shutdown).await {
            trigger::RefreshTrigger::Shutdown => break,
            trigger::RefreshTrigger::Event => {
                if !trigger::coalesce(state.portfolio_refresh_signal(), &shutdown).await {
                    break;
                }
                tick.reset();
            }
            trigger::RefreshTrigger::Interval => {}
        }
        let started_at_ms = common::time::now_ms();
        let result = publish_once(&state, &mut last_snapshot).await;
        registry.record_result_timed("portfolio", started_at_ms, result);
    }
}

async fn publish_once(
    state: &AppState,
    last_snapshot: &mut Option<PortfolioSnapshot>,
) -> Result<(), String> {
    let outcome = snapshot_or_problem(state, last_snapshot).await;
    cache_outcome(state, &outcome.envelope);
    // 快照计算与 REST 缓存写入始终执行；零订阅者时只跳过 WS 全量序列化。
    if state
        .ws_hub()
        .subscriber_count(realtime::channels::PORTFOLIO)
        > 0
    {
        let message = snapshot_ws_message(&outcome.envelope)?;
        state
            .ws_hub()
            .publish_throttled(realtime::channels::PORTFOLIO, message);
    }
    match outcome.task_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

async fn snapshot_or_problem(
    state: &AppState,
    last_snapshot: &mut Option<PortfolioSnapshot>,
) -> PublishOutcome {
    match portfolio::snapshot(state).await {
        Ok(snapshot) => {
            *last_snapshot = Some(snapshot.clone());
            PublishOutcome {
                envelope: portfolio_snapshot_envelope::snapshot_envelope(
                    snapshot,
                    portfolio_snapshot_envelope::SOURCE_LIFECYCLE,
                    common::time::now_ms(),
                ),
                task_error: None,
            }
        }
        Err(error) => stale_snapshot_or_problem(last_snapshot.as_ref(), &error),
    }
}

fn stale_snapshot_or_problem(
    last_snapshot: Option<&PortfolioSnapshot>,
    error: &common::AppError,
) -> PublishOutcome {
    let now_ms = common::time::now_ms();
    let Some(snapshot) = last_snapshot else {
        warn!(%error, "portfolio snapshot failed before any successful snapshot");
        return PublishOutcome {
            envelope: portfolio_snapshot_envelope::error_envelope(
                error,
                portfolio_snapshot_envelope::SOURCE_LIFECYCLE,
                now_ms,
            ),
            task_error: Some(error.to_string()),
        };
    };
    warn!(%error, "portfolio snapshot failed; publishing stale degraded snapshot");
    let snapshot = stale_degraded_snapshot(snapshot.clone(), error, now_ms);
    PublishOutcome {
        envelope: portfolio_snapshot_envelope::stale_envelope(
            snapshot,
            error,
            portfolio_snapshot_envelope::SOURCE_LIFECYCLE_STALE,
            now_ms,
        ),
        task_error: None,
    }
}

fn cache_outcome(state: &AppState, envelope: &PortfolioSnapshotEnvelope) {
    if let Some(snapshot) = envelope.snapshot.clone() {
        state.cache_portfolio_snapshot(snapshot);
    }
    state.cache_portfolio_snapshot_envelope(envelope.clone());
}

fn snapshot_ws_message(
    envelope: &PortfolioSnapshotEnvelope,
) -> Result<realtime::WsMessage, String> {
    realtime::WsMessage::json(envelope).map_err(|error| {
        warn!(%error, "portfolio snapshot serialization failed");
        error.to_string()
    })
}

fn stale_degraded_snapshot(
    mut snapshot: PortfolioSnapshot,
    error: &common::AppError,
    now_ms: i64,
) -> PortfolioSnapshot {
    let problem = runtime_problem::from_app_error("portfolio", OP_PORTFOLIO_SNAPSHOT, error, &[]);
    let freshness_ms = now_ms.saturating_sub(snapshot.server_now_ms);
    let position_count = snapshot.positions.len() as u64;
    snapshot.degraded = true;
    snapshot.problems.push(problem);
    snapshot
        .operation_health
        .push(portfolio_snapshot_envelope::blocked_operation_health(
            error,
            portfolio_snapshot_envelope::SOURCE_LIFECYCLE_STALE,
            "portfolio snapshot failed; last successful snapshot published as stale",
            position_count,
            Some(freshness_ms),
            now_ms,
        ));
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        AccountStateSnapshot, HardLimitsUsage, PnlBreakdown, PortfolioSummary, RiskSnapshot,
        VenueOperationStatus,
    };

    #[test]
    fn stale_degraded_snapshot_keeps_last_business_values_and_problem_source() {
        let error = common::AppError::RateLimited {
            retry_after_secs: 2,
        };
        let snapshot = base_snapshot(1_000);

        let snapshot = stale_degraded_snapshot(snapshot, &error, 1_750);

        assert!(snapshot.degraded);
        assert_eq!(snapshot.snapshot_version, "pos-1");
        assert_eq!(snapshot.server_now_ms, 1_000);
        assert_eq!(snapshot.summary.total_nav_usd, 123.0);
        assert_eq!(snapshot.problems.len(), 1);
        assert_eq!(snapshot.problems[0].operation, OP_PORTFOLIO_SNAPSHOT);
        assert_eq!(snapshot.problems[0].retry_after_ms, Some(2_000));

        let row = &snapshot.operation_health[0];
        assert_eq!(row.status, VenueOperationStatus::Blocked);
        assert_eq!(
            row.source,
            portfolio_snapshot_envelope::SOURCE_LIFECYCLE_STALE
        );
        assert_eq!(row.freshness_ms, Some(750));
        assert_eq!(row.retry_after_ms, Some(2_000));
        assert_eq!(
            row.problem
                .as_ref()
                .and_then(|problem| problem.source.as_deref()),
            Some(portfolio_snapshot_envelope::SOURCE_LIFECYCLE_STALE)
        );
    }

    #[test]
    fn cold_start_failure_publishes_error_envelope_without_fake_snapshot() {
        let error = common::AppError::RateLimited {
            retry_after_secs: 2,
        };

        let outcome = stale_snapshot_or_problem(None, &error);

        assert!(outcome.task_error.is_some());
        assert!(outcome.envelope.snapshot.is_none());
        assert_eq!(
            outcome
                .envelope
                .problem
                .as_ref()
                .map(|problem| problem.code.as_str()),
            Some(shared_types::problem::codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE)
        );
        assert_eq!(outcome.envelope.retry_after_ms, Some(2_000));
    }

    fn base_snapshot(server_now_ms: i64) -> PortfolioSnapshot {
        PortfolioSnapshot {
            summary: PortfolioSummary {
                total_nav_usd: 123.0,
                nav_evidence: shared_types::PortfolioNavEvidence::default(),
                nav_change_24h_pct: Some(1.0),
                net_delta_usd: 2.0,
                net_delta_pct_of_nav: 3.0,
                naked_exposure_usd: 4.0,
                naked_position_count: 1,
                realized_pnl_today_usd: 5.0,
                pnl_breakdown: PnlBreakdown::default(),
                updated_at_ms: server_now_ms,
            },
            positions: Vec::new(),
            balances: Vec::new(),
            risk: RiskSnapshot {
                var_99_1d_usd: 0.0,
                var_pct_of_nav: 0.0,
                var_sample_size: 0,
                funding_clustering: Vec::new(),
                delta_concentration: Vec::new(),
                margin_utilization: Vec::new(),
                hard_limits: HardLimitsUsage::default(),
                updated_at_ms: server_now_ms,
            },
            server_now_ms,
            snapshot_version: "pos-1".to_owned(),
            degraded: false,
            problems: Vec::new(),
            operation_health: Vec::new(),
            account_state: AccountStateSnapshot::default(),
            recent_close_runs: Vec::new(),
        }
    }
}
