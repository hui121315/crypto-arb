use dashmap::DashMap;
use shared_types::{
    problem::codes, ApiProblem, CloseRun, ExecutedTrade, ExecutionLedgerEvent, ListPage,
    ListStatus, LiveOrderState, MissedOpportunity, OrderRecord, PositionPairEvidence,
    ReviewCloseRunEvidence, ReviewDataSource, ReviewEnvelope, ReviewLedgerStatus, ReviewPnlField,
    VenueOperationHealth, VenueOperationStatus,
};
use std::collections::{BTreeMap, BTreeSet};
use trading::{ExecutionLedgerStorageSnapshot, SqlLedgerStorageSnapshot};

use crate::trading_service::TradingService;

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
pub(crate) const REVIEW_DEFAULT_LIMIT: usize = 50;
pub(crate) const REVIEW_MAX_LIMIT: usize = 100;
pub(crate) const REVIEW_MAX_DAYS: u32 = 365;
const REVIEW_SOURCE: &str = "review";
const REVIEW_STORAGE_VENUE: &str = "system";
const REVIEW_STORAGE_SOURCE: &str = "review_store";
const EXECUTION_LEDGER_STORAGE_SOURCE: &str = "execution_ledger_jsonl";
const SQL_LEDGER_STORAGE_SOURCE: &str = "trading_sql_ledger";

mod close_run_linkage;
mod executed_projection;
mod ledger;
mod missed;
mod paging;
mod runtime;
pub(crate) mod settlements;
mod storage_health;
mod strategy_performance;

use executed_projection::executed_envelope_at;
use ledger::realized_ledger_from_trading;
use paging::{list_page, min_window_ms, page_query_parts, review_snapshot_id, review_window_days};
pub(crate) use runtime::{runtime_snapshot_from_trading, warming_runtime_snapshot};
use storage_health::{
    review_storage_health, with_review_storage_health, with_trading_ledger_storage_health,
};

use strategy_performance::strategy_performance_envelope_from_trades;

#[cfg(test)]
use executed_projection::{executed_at, executed_review_snapshot_id};
#[cfg(test)]
use runtime::runtime_snapshot_from_parts;
#[cfg(test)]
use strategy_performance::strategy_performance_envelope_at;

#[cfg(test)]
use paging::page_rows;

#[cfg(test)]
pub(crate) use missed::missed_envelope;
pub(crate) use missed::missed_envelope_from_store;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ReviewPageQuery {
    limit: Option<usize>,
    cursor: Option<String>,
}

impl ReviewPageQuery {
    pub(crate) fn new(limit: Option<usize>, cursor: Option<String>) -> Self {
        Self { limit, cursor }
    }

    pub(crate) fn is_default_first_page(&self) -> bool {
        self.cursor
            .as_deref()
            .is_none_or(|cursor| cursor.trim().is_empty())
            && self.limit.is_none_or(|limit| limit == REVIEW_DEFAULT_LIMIT)
    }
}

#[cfg(test)]
pub(crate) fn executed(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    days: u32,
) -> Vec<ExecutedTrade> {
    let now_ms = common::time::now_ms();
    executed_at(orders, ledger, &[], days, now_ms)
}

#[cfg(test)]
pub(crate) fn executed_envelope(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    days: u32,
    page_query: &ReviewPageQuery,
) -> ReviewEnvelope<ExecutedTrade> {
    let now_ms = common::time::now_ms();
    let envelope = executed_envelope_at(orders, ledger, &[], days, page_query, now_ms);
    let row_count = envelope.row_count;
    with_review_storage_health(
        envelope,
        review_storage_health(ReviewDataSource::ExecutionLedger, row_count, now_ms),
    )
}

#[cfg(test)]
pub(crate) fn executed_envelope_with_close_runs(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    days: u32,
    page_query: &ReviewPageQuery,
) -> ReviewEnvelope<ExecutedTrade> {
    let now_ms = common::time::now_ms();
    let envelope = executed_envelope_at(orders, ledger, close_runs, days, page_query, now_ms);
    let row_count = envelope.row_count;
    with_review_storage_health(
        envelope,
        review_storage_health(ReviewDataSource::ExecutionLedger, row_count, now_ms),
    )
}

pub(crate) async fn executed_envelope_from_trading(
    service: &TradingService,
    close_runs: &[CloseRun],
    days: u32,
    page_query: &ReviewPageQuery,
) -> ReviewEnvelope<ExecutedTrade> {
    let now_ms = common::time::now_ms();
    let mut ignored_problems = Vec::new();
    let query_days = review_window_days(days, &mut ignored_problems);
    let from_ms = min_window_ms(now_ms, query_days);
    let to_ms = now_ms.saturating_add(1);
    let realized = realized_ledger_from_trading(service, from_ms, to_ms).await;
    let close_runs = close_runs_for_review(close_runs, &realized.close_runs);
    with_trading_ledger_storage_health(
        executed_envelope_at(
            &realized.orders,
            &realized.ledger,
            &close_runs,
            days,
            page_query,
            now_ms,
        ),
        service,
        now_ms,
    )
    .with_funding_payment_ingest_if_present(service)
}

pub(crate) fn close_run_snapshots(store: &DashMap<String, CloseRun>) -> Vec<CloseRun> {
    store.iter().map(|entry| entry.value().clone()).collect()
}

pub(crate) async fn scoped_executed_from_trading(
    service: &TradingService,
    close_runs: &[CloseRun],
    days: u32,
    page_query: &ReviewPageQuery,
    scope: &shared_types::review::ReviewScope,
    run: Option<&shared_types::ExecutionRun>,
) -> ReviewEnvelope<ExecutedTrade> {
    let now_ms = common::time::now_ms();
    let mut problems = Vec::new();
    let days = review_window_days(days, &mut problems);
    let from_ms = min_window_ms(now_ms, days);
    let realized = realized_ledger_from_trading(service, from_ms, now_ms.saturating_add(1)).await;
    let close_runs = close_runs_for_review(close_runs, &realized.close_runs);
    let materialized = executed_projection::materialize_executed_at(
        &realized.orders,
        &realized.ledger,
        &close_runs,
        days,
        now_ms,
    );
    let envelope = executed_projection::scoped_executed_envelope_at(
        &materialized,
        scope,
        run,
        page_query,
        executed_projection::ExecutedEnvelopeContext {
            ledger: &realized.ledger,
            days,
            offset: 0,
            limit: REVIEW_DEFAULT_LIMIT,
            now_ms,
            from_ms,
            page_status: if problems.is_empty() {
                ListStatus::Fresh
            } else {
                ListStatus::Degraded
            },
            page_problems: problems,
        },
    );
    with_trading_ledger_storage_health(envelope, service, now_ms)
        .with_funding_payment_ingest_if_present(service)
}

fn close_runs_for_review(hot: &[CloseRun], durable: &[CloseRun]) -> Vec<CloseRun> {
    let mut runs = BTreeMap::<String, CloseRun>::new();
    for run in durable.iter().chain(hot) {
        upsert_latest_review_close_run(&mut runs, run.clone());
    }
    runs.into_values().collect()
}

fn upsert_latest_review_close_run(runs: &mut BTreeMap<String, CloseRun>, candidate: CloseRun) {
    match runs.get(&candidate.id) {
        Some(current) if current.updated_at_ms > candidate.updated_at_ms => {}
        _ => {
            runs.insert(candidate.id.clone(), candidate);
        }
    }
}

trait ReviewEnvelopeRuntimeContext<T> {
    fn with_funding_payment_ingest_if_present(self, service: &TradingService) -> Self;
}

impl<T> ReviewEnvelopeRuntimeContext<T> for ReviewEnvelope<T> {
    fn with_funding_payment_ingest_if_present(mut self, service: &TradingService) -> Self {
        if let Some(report) = service.latest_private_funding_payment_ingest_report() {
            self = self.with_funding_payment_ingest(report);
        }
        self
    }
}

#[cfg(test)]
mod tests;
