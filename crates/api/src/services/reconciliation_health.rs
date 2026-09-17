use crate::trading_service::ReconcileRefreshFailure;
use dashmap::DashMap;
use shared_types::{LiveOrderState, OrderRecord, VenueOperationStatus};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) const GLOBAL_RECONCILIATION_VENUE: &str = "*";
pub(crate) const SOURCE_RECONCILIATION_RUNTIME: &str = "reconciliation_runtime";

const RECONCILIATION_STALE_MS: i64 = 90_000;

#[derive(Default)]
pub(crate) struct ReconciliationHealthStore {
    rows: DashMap<String, ReconciliationRuntimeHealth>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReconciliationRuntimeHealth {
    pub(crate) venue: String,
    pub(crate) status: VenueOperationStatus,
    pub(crate) message: String,
    pub(crate) requested: Option<u64>,
    pub(crate) rows: Option<u64>,
    pub(crate) freshness_ms: Option<i64>,
    pub(crate) retry_after_ms: Option<u64>,
    pub(crate) error: Option<String>,
    pub(crate) observed_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ReconciliationHealthCycle {
    open_by_venue: BTreeMap<String, usize>,
    open_total: usize,
}

impl ReconciliationHealthCycle {
    pub(crate) fn from_orders(orders: &[OrderRecord]) -> Self {
        let mut cycle = Self::default();
        for order in orders
            .iter()
            .filter(|order| is_reconcile_order(order.state))
        {
            let venue = shared_types::normalized_venue_name(&order.intent.exchange);
            cycle.open_total = cycle.open_total.saturating_add(1);
            *cycle.open_by_venue.entry(venue).or_default() += 1;
        }
        cycle
    }
}

impl ReconciliationHealthStore {
    pub(crate) fn snapshot(&self, now_ms: i64) -> Vec<ReconciliationRuntimeHealth> {
        self.rows
            .iter()
            .map(|row| stale_adjusted(row.value().clone(), now_ms))
            .collect()
    }

    pub(crate) fn record_success(
        &self,
        cycle: &ReconciliationHealthCycle,
        refreshed: &[OrderRecord],
        diff_count: usize,
        refresh_failures: &[ReconcileRefreshFailure],
    ) {
        let failures_by_venue = refresh_failure_counts(refresh_failures);
        let status = success_status(diff_count, refresh_failures.len());
        let message = format!(
            "订单回查完成：本地待确认 {}，差异 {}，刷新 {}，刷新失败 {}",
            cycle.open_total,
            diff_count,
            refreshed.len(),
            refresh_failures.len()
        );
        if cycle.open_by_venue.is_empty() {
            self.replace_cycle_rows(vec![runtime_row(
                GLOBAL_RECONCILIATION_VENUE,
                status,
                message,
                0,
                refreshed.len(),
                refresh_failure_error(refresh_failures),
            )]);
            return;
        }
        let mut rows = Vec::with_capacity(cycle.open_by_venue.len());
        for (venue, requested) in &cycle.open_by_venue {
            rows.push(runtime_row(
                venue,
                venue_success_status(status, *failures_by_venue.get(venue).unwrap_or(&0)),
                message.clone(),
                *requested,
                *requested,
                venue_refresh_failure_error(refresh_failures, venue),
            ));
        }
        self.replace_cycle_rows(rows);
    }

    pub(crate) fn record_failure(&self, cycle: &ReconciliationHealthCycle, error: &str) {
        let message = format!("订单回查失败：{error}");
        if cycle.open_by_venue.is_empty() {
            self.replace_cycle_rows(vec![runtime_row(
                GLOBAL_RECONCILIATION_VENUE,
                VenueOperationStatus::Blocked,
                message,
                0,
                0,
                Some(error.to_owned()),
            )]);
            return;
        }
        let mut rows = Vec::with_capacity(cycle.open_by_venue.len());
        for (venue, requested) in &cycle.open_by_venue {
            rows.push(runtime_row(
                venue,
                VenueOperationStatus::Blocked,
                message.clone(),
                *requested,
                0,
                Some(error.to_owned()),
            ));
        }
        self.replace_cycle_rows(rows);
    }

    fn record(&self, row: ReconciliationRuntimeHealth) {
        self.rows.insert(row.venue.clone(), row);
    }

    fn replace_cycle_rows(&self, rows: Vec<ReconciliationRuntimeHealth>) {
        let venues = rows
            .iter()
            .map(|row| row.venue.clone())
            .collect::<BTreeSet<_>>();
        self.rows.retain(|venue, _| venues.contains(venue));
        for row in rows {
            self.record(row);
        }
    }
}

fn runtime_row(
    venue: &str,
    status: VenueOperationStatus,
    message: String,
    requested: usize,
    rows: usize,
    error: Option<String>,
) -> ReconciliationRuntimeHealth {
    ReconciliationRuntimeHealth {
        venue: venue.to_owned(),
        status,
        message,
        requested: Some(requested as u64),
        rows: Some(rows as u64),
        freshness_ms: Some(0),
        retry_after_ms: None,
        error,
        observed_at_ms: common::time::now_ms(),
    }
}

fn refresh_failure_counts(failures: &[ReconcileRefreshFailure]) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for failure in failures {
        *counts.entry(failure.venue.clone()).or_default() += 1;
    }
    counts
}

fn success_status(diff_count: usize, refresh_failure_count: usize) -> VenueOperationStatus {
    if refresh_failure_count > 0 {
        VenueOperationStatus::Blocked
    } else if diff_count > 0 {
        VenueOperationStatus::Warn
    } else {
        VenueOperationStatus::Ok
    }
}

fn venue_success_status(
    status: VenueOperationStatus,
    venue_failure_count: usize,
) -> VenueOperationStatus {
    if venue_failure_count > 0 {
        VenueOperationStatus::Blocked
    } else {
        status
    }
}

fn refresh_failure_error(failures: &[ReconcileRefreshFailure]) -> Option<String> {
    (!failures.is_empty()).then(|| format!("{} 笔订单刷新失败", failures.len()))
}

fn venue_refresh_failure_error(
    failures: &[ReconcileRefreshFailure],
    venue: &str,
) -> Option<String> {
    failures
        .iter()
        .find(|failure| failure.venue == venue)
        .map(|failure| format!("{}: {}", failure.internal_order_id, failure.error))
}

fn stale_adjusted(
    mut row: ReconciliationRuntimeHealth,
    now_ms: i64,
) -> ReconciliationRuntimeHealth {
    let freshness_ms = now_ms.saturating_sub(row.observed_at_ms);
    row.freshness_ms = Some(freshness_ms);
    if row.status == VenueOperationStatus::Ok && freshness_ms > RECONCILIATION_STALE_MS {
        row.status = VenueOperationStatus::Warn;
        row.message = "订单回查样本已变旧".to_owned();
    }
    row
}

fn is_reconcile_order(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Created
            | LiveOrderState::RiskChecked
            | LiveOrderState::Submitted
            | LiveOrderState::Accepted
            | LiveOrderState::PartiallyFilled
            | LiveOrderState::CancelRequested
            | LiveOrderState::Unknown
    )
}

#[cfg(test)]
mod tests;
