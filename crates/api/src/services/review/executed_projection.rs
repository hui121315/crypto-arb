use super::close_run_linkage::attach_close_run_evidence;
use super::ledger::{
    aggregate_missing_fields_for_trades, ledger_projected_orders, ledger_status_for_total,
    merge_review_problems, review_status_for_ledger_context, saturating_u32,
    ReviewLedgerProblemContext,
};
use super::paging::{
    list_page, min_window_ms, page_query_parts, review_snapshot_id, review_window_days,
};
use super::ReviewPageQuery;
use shared_types::{
    ApiProblem, CloseRun, ExecutedTrade, ExecutionLedgerEvent, ListStatus, OrderRecord,
    ReviewDataSource, ReviewEnvelope,
};
use std::collections::BTreeMap;

pub(super) fn executed_envelope_at(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    days: u32,
    page_query: &ReviewPageQuery,
    now_ms: i64,
) -> ReviewEnvelope<ExecutedTrade> {
    let mut window_problems = Vec::new();
    let days = review_window_days(days, &mut window_problems);
    let from_ms = min_window_ms(now_ms, days);
    let realized = review_domain::realized_pnl_by_group_with_close_runs(
        orders,
        ledger,
        close_runs,
        from_ms,
        now_ms.saturating_add(1),
    );
    let projected_orders = ledger_projected_orders(orders, &realized);
    let snapshot_id = executed_review_snapshot_id(&realized);
    let (offset, limit, mut status, mut problems) = page_query_parts(page_query, &snapshot_id);
    problems.extend(window_problems);
    if !problems.is_empty() {
        status = ListStatus::Degraded;
    }
    let page_rows =
        review_domain::executed_page_from_orders(&projected_orders, now_ms, days, offset, limit);
    let mut rows = review_domain::apply_realized_pnl(page_rows.rows, &realized);
    attach_close_run_evidence(&mut rows, close_runs);
    finish_envelope(
        rows,
        page_rows.total_rows,
        &snapshot_id,
        ExecutedEnvelopeContext {
            ledger,
            days,
            offset,
            limit,
            now_ms,
            from_ms,
            page_status: status,
            page_problems: problems,
        },
    )
}

pub(super) struct ReviewMaterialization {
    pub(super) trades: Vec<ExecutedTrade>,
    snapshot_id: String,
}

pub(super) fn materialize_executed_at(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    days: u32,
    now_ms: i64,
) -> ReviewMaterialization {
    let from_ms = min_window_ms(now_ms, days);
    let realized = review_domain::realized_pnl_by_group_with_close_runs(
        orders,
        ledger,
        close_runs,
        from_ms,
        now_ms.saturating_add(1),
    );
    let rows = ledger_projected_orders(orders, &realized);
    let page = review_domain::executed_page_from_orders(&rows, now_ms, days, 0, usize::MAX);
    let mut trades = review_domain::apply_realized_pnl(page.rows, &realized);
    attach_close_run_evidence(&mut trades, close_runs);
    ReviewMaterialization {
        trades,
        snapshot_id: executed_review_snapshot_id(&realized),
    }
}

pub(super) struct ExecutedEnvelopeContext<'a> {
    pub(super) ledger: &'a [ExecutionLedgerEvent],
    pub(super) days: u32,
    pub(super) offset: usize,
    pub(super) limit: usize,
    pub(super) now_ms: i64,
    pub(super) from_ms: i64,
    pub(super) page_status: ListStatus,
    pub(super) page_problems: Vec<ApiProblem>,
}

pub(super) fn executed_envelope_from_materialization(
    materialized: &ReviewMaterialization,
    context: ExecutedEnvelopeContext<'_>,
) -> ReviewEnvelope<ExecutedTrade> {
    let total_rows = materialized.trades.len();
    let rows = materialized
        .trades
        .iter()
        .skip(context.offset)
        .take(context.limit)
        .cloned()
        .collect::<Vec<_>>();
    finish_envelope(rows, total_rows, &materialized.snapshot_id, context)
}

fn finish_envelope(
    rows: Vec<ExecutedTrade>,
    total_rows: usize,
    snapshot_id: &str,
    context: ExecutedEnvelopeContext<'_>,
) -> ReviewEnvelope<ExecutedTrade> {
    let missing_fields = aggregate_missing_fields_for_trades(&rows);
    let ledger_status = ledger_status_for_total(context.ledger, total_rows, &missing_fields);
    let (ledger_list_status, ledger_problems) =
        review_status_for_ledger_context(ReviewLedgerProblemContext {
            ledger_status,
            window_days: context.days.max(1),
            window_from_ms: context.from_ms,
            window_to_ms: context.now_ms.saturating_add(1),
            ledger_event_count: context.ledger.len(),
            row_count: total_rows,
            missing_fields: &missing_fields,
            sample_count: saturating_u32(total_rows),
            excluded_incomplete_count: 0,
        });
    let (status, problems) = merge_review_problems(
        context.page_status,
        context.page_problems,
        ledger_list_status,
        ledger_problems,
    );
    let page = list_page(
        context.limit,
        context.offset,
        rows.len(),
        total_rows,
        snapshot_id,
    );
    ReviewEnvelope::new(
        rows,
        context.now_ms,
        context.days,
        ReviewDataSource::ExecutionLedger,
        Some(ledger_status),
        missing_fields,
    )
    .with_page(page, status, problems)
}

#[cfg(test)]
pub(super) fn executed_at(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    days: u32,
    now_ms: i64,
) -> Vec<ExecutedTrade> {
    let mut ignored_problems = Vec::new();
    let days = review_window_days(days, &mut ignored_problems);
    materialize_executed_at(orders, ledger, close_runs, days, now_ms).trades
}

pub(super) fn executed_review_snapshot_id(
    realized: &BTreeMap<String, review_domain::RealizedPnlRow>,
) -> String {
    review_snapshot_id(
        "executed",
        realized
            .iter()
            .map(|(group_id, row)| format!("{group_id}:{row:?}")),
    )
}
