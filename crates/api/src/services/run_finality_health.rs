use crate::services::run_finality::{
    RunFinalityOutcome, RunFinalitySampleProblem, RunFinalityVenueOutcome,
};
use dashmap::DashMap;
use shared_types::{normalized_venue_name, venue_family, VenueOperationStatus};
use std::collections::BTreeSet;

pub(crate) const GLOBAL_RUN_FINALITY_VENUE: &str = "*";
pub(crate) const SOURCE_RUN_FINALITY_RUNTIME: &str = "run_finality_runtime";

const RUN_FINALITY_STALE_MS: i64 = 90_000;

#[derive(Default)]
pub(crate) struct RunFinalityHealthStore {
    rows: DashMap<String, RunFinalityRuntimeHealth>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunFinalityRuntimeHealth {
    pub(crate) venue: String,
    pub(crate) status: VenueOperationStatus,
    pub(crate) message: String,
    pub(crate) requested: Option<u64>,
    pub(crate) rows: Option<u64>,
    pub(crate) freshness_ms: Option<i64>,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) error: Option<String>,
    pub(crate) observed_at_ms: i64,
    pub(crate) scanned_order_count: usize,
    pub(crate) refreshed_order_count: usize,
    pub(crate) remote_missing_count: usize,
    pub(crate) skipped_terminal_count: usize,
    pub(crate) refresh_failure_count: usize,
    pub(crate) publish_failure_count: usize,
    pub(crate) sample_problem: Option<RunFinalitySampleProblem>,
}

impl RunFinalityHealthStore {
    pub(crate) fn snapshot(&self, now_ms: i64) -> Vec<RunFinalityRuntimeHealth> {
        self.rows
            .iter()
            .map(|row| stale_adjusted(row.value().clone(), now_ms))
            .collect()
    }

    pub(crate) fn record_outcome(&self, outcome: &RunFinalityOutcome) {
        let rows = if outcome.venue_outcomes.is_empty() {
            vec![runtime_row(
                GLOBAL_RUN_FINALITY_VENUE,
                &RunFinalityVenueOutcome::default(),
            )]
        } else {
            outcome
                .venue_outcomes
                .iter()
                .map(|(venue, venue_outcome)| runtime_row(venue, venue_outcome))
                .collect()
        };
        self.replace_cycle_rows(rows);
    }

    pub(crate) fn invalidate_credentials_update(&self, venue: &str) {
        let exact_key = normalized_venue_name(venue);
        let family_key = normalized_venue_name(venue_family(venue));
        let keys = self
            .rows
            .iter()
            .filter_map(|entry| {
                let key = entry.key();
                let key_family = normalized_venue_name(venue_family(key));
                (key == &exact_key || key_family == family_key).then(|| key.clone())
            })
            .collect::<Vec<_>>();

        for key in keys {
            self.rows.remove(&key);
        }
    }

    fn replace_cycle_rows(&self, rows: Vec<RunFinalityRuntimeHealth>) {
        let venues = rows
            .iter()
            .map(|row| row.venue.clone())
            .collect::<BTreeSet<_>>();
        self.rows.retain(|venue, _| venues.contains(venue));
        for row in rows {
            self.rows.insert(row.venue.clone(), row);
        }
    }
}

fn runtime_row(venue: &str, outcome: &RunFinalityVenueOutcome) -> RunFinalityRuntimeHealth {
    let status = outcome_status(outcome);
    let message = outcome_message(outcome);
    RunFinalityRuntimeHealth {
        venue: venue.to_owned(),
        status,
        message: message.clone(),
        requested: Some(outcome.scanned_order_count as u64),
        rows: Some(outcome_success_count(outcome) as u64),
        freshness_ms: Some(0),
        retry_after_ms: None,
        error: attention_error(status, &message),
        observed_at_ms: common::time::now_ms(),
        scanned_order_count: outcome.scanned_order_count,
        refreshed_order_count: outcome.refreshed_order_count,
        remote_missing_count: outcome.remote_missing_count,
        skipped_terminal_count: outcome.skipped_terminal_count,
        refresh_failure_count: outcome.refresh_failure_count,
        publish_failure_count: outcome.publish_failure_count,
        sample_problem: outcome.sample_problem.clone(),
    }
}

fn outcome_status(outcome: &RunFinalityVenueOutcome) -> VenueOperationStatus {
    if outcome.refresh_failure_count > 0 || outcome.publish_failure_count > 0 {
        VenueOperationStatus::Blocked
    } else if outcome.remote_missing_count > 0 {
        VenueOperationStatus::Warn
    } else {
        VenueOperationStatus::Ok
    }
}

fn outcome_message(outcome: &RunFinalityVenueOutcome) -> String {
    let sample = outcome
        .sample_problem
        .as_ref()
        .map(sample_problem_message)
        .unwrap_or_default();
    format!(
        "订单终态回查完成：待确认 {}，刷新 {}，远端缺失 {}，已终态 {}，刷新失败 {}，发布失败 {}{}",
        outcome.scanned_order_count,
        outcome.refreshed_order_count,
        outcome.remote_missing_count,
        outcome.skipped_terminal_count,
        outcome.refresh_failure_count,
        outcome.publish_failure_count,
        sample
    )
}

fn sample_problem_message(sample: &RunFinalitySampleProblem) -> String {
    format!(
        "，样本订单 {} / {}：{}",
        sample.raw_order_id, sample.source, sample.message
    )
}

fn outcome_success_count(outcome: &RunFinalityVenueOutcome) -> usize {
    outcome
        .refreshed_order_count
        .saturating_add(outcome.skipped_terminal_count)
}

fn attention_error(status: VenueOperationStatus, message: &str) -> Option<String> {
    matches!(
        status,
        VenueOperationStatus::Warn | VenueOperationStatus::Blocked
    )
    .then(|| message.to_owned())
}

fn stale_adjusted(mut row: RunFinalityRuntimeHealth, now_ms: i64) -> RunFinalityRuntimeHealth {
    let freshness_ms = now_ms.saturating_sub(row.observed_at_ms);
    row.freshness_ms = Some(freshness_ms);
    if row.status == VenueOperationStatus::Ok && freshness_ms > RUN_FINALITY_STALE_MS {
        row.status = VenueOperationStatus::Warn;
        row.message = "订单终态回查样本已变旧".to_owned();
        row.error = Some(row.message.clone());
    }
    row
}

#[cfg(test)]
mod tests;
