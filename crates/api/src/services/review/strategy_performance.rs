#[cfg(test)]
use super::executed_projection::executed_at;
use super::ledger::{
    aggregate_missing_fields_for_trades, ledger_status, review_status_for_ledger_context,
    ReviewLedgerProblemContext,
};
use super::paging::{list_page, min_window_ms, review_snapshot_id};
#[cfg(test)]
use super::storage_health::{review_storage_health, with_review_storage_health};
#[cfg(test)]
use shared_types::{CloseRun, OrderRecord};
use shared_types::{
    ExecutedTrade, ExecutionLedgerEvent, ReviewDataSource, ReviewEnvelope, StrategyPerformance,
    P0_EXECUTABLE_STRATEGY_KINDS,
};

fn strategy_performance_from_trades(trades: &[ExecutedTrade]) -> Vec<StrategyPerformance> {
    P0_EXECUTABLE_STRATEGY_KINDS
        .into_iter()
        .map(|kind| review_domain::compute_performance(trades, kind))
        .filter(|row| row.total_trades_30d > 0)
        .collect()
}

#[cfg(test)]
pub(crate) fn strategy_performance_envelope_at(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    now_ms: i64,
) -> ReviewEnvelope<StrategyPerformance> {
    let envelope = strategy_performance_envelope_from_parts(orders, ledger, close_runs, now_ms);
    let row_count = envelope.row_count;
    with_review_storage_health(
        envelope,
        review_storage_health(ReviewDataSource::ExecutionLedger, row_count, now_ms),
    )
}

#[cfg(test)]
fn strategy_performance_envelope_from_parts(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    now_ms: i64,
) -> ReviewEnvelope<StrategyPerformance> {
    let trades = executed_at(orders, ledger, close_runs, 30, now_ms);
    strategy_performance_envelope_from_trades(&trades, ledger, now_ms)
}

pub(super) fn strategy_performance_envelope_from_trades(
    trades: &[ExecutedTrade],
    ledger: &[ExecutionLedgerEvent],
    now_ms: i64,
) -> ReviewEnvelope<StrategyPerformance> {
    let ledger_status = ledger_status(ledger, trades);
    let rows = strategy_performance_from_trades(trades);
    let missing_fields = aggregate_missing_fields_for_trades(trades);
    let sample_count = rows.iter().map(|row| row.trades_30d).sum();
    let excluded_incomplete_count = rows.iter().map(|row| row.skipped_trades_30d).sum();
    let (status, problems) = review_status_for_ledger_context(ReviewLedgerProblemContext {
        ledger_status,
        window_days: 30,
        window_from_ms: min_window_ms(now_ms, 30),
        window_to_ms: now_ms.saturating_add(1),
        ledger_event_count: ledger.len(),
        row_count: rows.len(),
        missing_fields: &missing_fields,
        sample_count,
        excluded_incomplete_count,
    });
    let snapshot_id = review_snapshot_id(
        "strategy-performance",
        rows.iter().map(|row| {
            format!(
                "{:?}:{}:{}:{}:{}:{}:{}:{:?}:{:016x}:{:016x}:{:016x}",
                row.kind,
                row.total_trades_30d,
                row.trades_30d,
                row.actual_trades_30d,
                row.estimated_trades_30d,
                row.skipped_trades_30d,
                row.partial_evidence_trades_30d,
                row.sample_status,
                row.net_pnl_30d_usd.to_bits(),
                row.actual_net_pnl_30d_usd.to_bits(),
                row.estimated_net_pnl_30d_usd.to_bits(),
            )
        }),
    );
    let page = list_page(rows.len(), 0, rows.len(), rows.len(), &snapshot_id);
    ReviewEnvelope::new(
        rows,
        now_ms,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ledger_status),
        missing_fields,
    )
    .with_page(page, status, problems)
}
