use std::collections::BTreeMap;

use crate::state::AppState;
use shared_types::{
    ApiProblem, CloseRun, CloseRunCompensationAttempt, CloseRunUnwindPlan, ExecutionRun,
    ExecutionRunLeg, LiveOrderState, OrderRecord, normalized_venue_name, problem::codes,
};
use tracing::{debug, warn};

const ORDER_EVENT: &str = "run_finality_status_backfilled";
const EXECUTION_RUN_EVENT: &str = "execution_run_updated";
const CLOSE_RUN_EVENT: &str = "close_run_updated";
const FINALITY_PROBLEM_SOURCE: &str = "run_finality";

#[path = "../collect.rs"]
mod collect;
#[path = "../problems.rs"]
mod problems;

use collect::{collect_pending_order_refs, is_terminal_order_state};
use problems::{finality_refresh_failure_problem, finality_remote_missing_problem};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunFinalityOutcome {
    pub(crate) scanned_order_count: usize,
    pub(crate) refreshed_order_count: usize,
    pub(crate) remote_missing_count: usize,
    pub(crate) skipped_missing_local_count: usize,
    pub(crate) skipped_terminal_count: usize,
    pub(crate) refresh_failure_count: usize,
    pub(crate) publish_failure_count: usize,
    pub(crate) sample_problem: Option<RunFinalitySampleProblem>,
    pub(crate) venue_outcomes: BTreeMap<String, RunFinalityVenueOutcome>,
}

impl RunFinalityOutcome {
    pub(crate) fn has_failures(&self) -> bool {
        self.refresh_failure_count > 0 || self.publish_failure_count > 0
    }

    fn record_scanned(&mut self, target: &PendingOrderTarget) {
        self.venue_mut(target).scanned_order_count += 1;
    }

    fn record_refreshed(&mut self, target: &PendingOrderTarget) {
        self.refreshed_order_count += 1;
        self.venue_mut(target).refreshed_order_count += 1;
    }

    fn record_remote_missing(
        &mut self,
        target: &PendingOrderTarget,
        problem: &ApiProblem,
        checked_at_ms: i64,
    ) {
        self.remote_missing_count += 1;
        self.venue_mut(target).remote_missing_count += 1;
        self.record_sample_problem(RunFinalitySampleProblem::from_problem(
            target,
            problem,
            checked_at_ms,
            None,
        ));
    }

    fn record_skipped_terminal(&mut self, target: &PendingOrderTarget) {
        self.skipped_terminal_count += 1;
        self.venue_mut(target).skipped_terminal_count += 1;
    }

    fn record_refresh_failure(
        &mut self,
        target: &PendingOrderTarget,
        problem: &ApiProblem,
        checked_at_ms: i64,
        error: String,
    ) {
        self.refresh_failure_count += 1;
        self.venue_mut(target).refresh_failure_count += 1;
        self.record_sample_problem(RunFinalitySampleProblem::from_problem(
            target,
            problem,
            checked_at_ms,
            Some(error),
        ));
    }

    fn record_publish_failure(&mut self, target: &PendingOrderTarget, error: String) {
        self.publish_failure_count += 1;
        self.venue_mut(target).publish_failure_count += 1;
        self.record_sample_problem(RunFinalitySampleProblem::publish_failure(
            target,
            error,
            common::time::now_ms(),
        ));
    }

    fn record_sample_problem(&mut self, sample: RunFinalitySampleProblem) {
        if self.sample_problem.is_none() {
            self.sample_problem = Some(sample.clone());
        }
        let venue = self.venue_outcomes.entry(sample.venue.clone()).or_default();
        if venue.sample_problem.is_none() {
            venue.sample_problem = Some(sample);
        }
    }

    fn venue_mut(&mut self, target: &PendingOrderTarget) -> &mut RunFinalityVenueOutcome {
        self.venue_outcomes.entry(target.venue.clone()).or_default()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct RunFinalityVenueOutcome {
    pub(crate) scanned_order_count: usize,
    pub(crate) refreshed_order_count: usize,
    pub(crate) remote_missing_count: usize,
    pub(crate) skipped_terminal_count: usize,
    pub(crate) refresh_failure_count: usize,
    pub(crate) publish_failure_count: usize,
    pub(crate) sample_problem: Option<RunFinalitySampleProblem>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunFinalitySampleProblem {
    pub(crate) raw_order_id: String,
    pub(crate) internal_order_id: String,
    pub(crate) venue: String,
    pub(crate) source: String,
    pub(crate) order_state: LiveOrderState,
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) status: Option<u16>,
    pub(crate) checked_at_ms: i64,
    pub(crate) error: Option<String>,
}

impl RunFinalitySampleProblem {
    fn from_problem(
        target: &PendingOrderTarget,
        problem: &ApiProblem,
        checked_at_ms: i64,
        error: Option<String>,
    ) -> Self {
        Self {
            raw_order_id: target.raw_order_id.clone(),
            internal_order_id: target.internal_order_id.clone(),
            venue: target.venue.clone(),
            source: target.source.label().to_owned(),
            order_state: target.state,
            code: problem.code.clone(),
            message: problem.message.clone(),
            status: problem.status,
            checked_at_ms,
            error,
        }
    }

    fn publish_failure(target: &PendingOrderTarget, error: String, checked_at_ms: i64) -> Self {
        Self {
            raw_order_id: target.raw_order_id.clone(),
            internal_order_id: target.internal_order_id.clone(),
            venue: target.venue.clone(),
            source: target.source.label().to_owned(),
            order_state: target.state,
            code: codes::HEDGE_ORDER_FINALITY_FAILED.to_owned(),
            message: format!("订单终态事件发布失败: {error}"),
            status: Some(502),
            checked_at_ms,
            error: Some(error),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingOrderSource {
    ExecutionRun,
    CloseRun,
    Mixed,
}

impl PendingOrderSource {
    fn label(self) -> &'static str {
        match self {
            Self::ExecutionRun => "ExecutionRun",
            Self::CloseRun => "CloseRun",
            Self::Mixed => "Mixed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOrderRef {
    order_id: String,
    source: PendingOrderSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingOrderTarget {
    raw_order_id: String,
    internal_order_id: String,
    venue: String,
    source: PendingOrderSource,
    state: LiveOrderState,
}

type PendingOrderMap = BTreeMap<String, PendingOrderSource>;

pub(crate) async fn refresh_pending_runs(state: &AppState) -> RunFinalityOutcome {
    let pending = collect_pending_order_refs(state);
    let mut outcome = RunFinalityOutcome {
        scanned_order_count: pending.len(),
        ..RunFinalityOutcome::default()
    };
    for order_ref in pending {
        refresh_pending_order(state, order_ref, &mut outcome).await;
    }
    outcome
}

async fn refresh_pending_order(
    state: &AppState,
    order_ref: PendingOrderRef,
    outcome: &mut RunFinalityOutcome,
) {
    let Some((target, local_record)) = pending_order_target(state, &order_ref) else {
        if !project_unsubmitted_execution_leg(state, &order_ref, outcome) {
            outcome.skipped_missing_local_count += 1;
        }
        return;
    };
    outcome.record_scanned(&target);
    if is_terminal_order_state(target.state) {
        publish_terminal_order(state, &local_record, &target, outcome);
        return;
    }
    let result = state
        .trading_service()
        .refresh_order_state(&target.internal_order_id)
        .await;
    handle_refresh_result(state, &target, result, outcome);
}

fn pending_order_target(
    state: &AppState,
    order_ref: &PendingOrderRef,
) -> Option<(PendingOrderTarget, OrderRecord)> {
    let record = local_order_record(state, &order_ref.order_id)?;
    let target = PendingOrderTarget {
        raw_order_id: order_ref.order_id.clone(),
        internal_order_id: record.intent.id.clone(),
        venue: normalized_venue_name(&record.intent.exchange),
        source: order_ref.source,
        state: record.state,
    };
    Some((target, record))
}

fn handle_refresh_result(
    state: &AppState,
    target: &PendingOrderTarget,
    result: trading::TradingResult<Option<OrderRecord>>,
    outcome: &mut RunFinalityOutcome,
) {
    match result {
        Ok(Some(record)) => publish_refreshed_order(state, &record, target, outcome),
        Ok(None) => record_remote_missing(state, target, outcome),
        Err(error) => record_refresh_failure(state, target, &error, outcome),
    }
}
