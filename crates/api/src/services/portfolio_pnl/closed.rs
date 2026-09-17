use super::{day_start_ms, history_window_start_ms, pnl_evidence, PortfolioPnlToday, DAY_MS};
use shared_types::{CloseRun, CloseRunStatus, ExecutionLedgerEvent, OrderRecord};
use std::collections::BTreeMap;

pub(super) struct ClosedRealizedRow {
    row: review_domain::RealizedPnlRow,
    closed_at_ms: i64,
}

pub(super) fn closed_realized_rows(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    to_ms: i64,
) -> Vec<ClosedRealizedRow> {
    let close_times = terminal_close_times(close_runs);
    review_domain::realized_pnl_by_group_with_close_runs(
        orders,
        ledger,
        close_runs,
        i64::MIN,
        to_ms,
    )
    .into_values()
    .filter_map(|row| {
        let closed_at_ms = row
            .evidence
            .close_run_evidence
            .iter()
            .filter_map(|evidence| close_times.get(&evidence.close_run_id))
            .copied()
            .max()?;
        Some(ClosedRealizedRow { row, closed_at_ms })
    })
    .collect()
}

pub(super) fn today_from_closed_rows(
    closed: &[ClosedRealizedRow],
    now_ms: i64,
    source: &'static str,
) -> PortfolioPnlToday {
    let from_ms = day_start_ms(now_ms);
    let to_ms = now_ms.saturating_add(1);
    let rows = closed
        .iter()
        .filter(|entry| entry.closed_at_ms >= from_ms && entry.closed_at_ms < to_ms)
        .map(|entry| &entry.row)
        .collect::<Vec<_>>();
    let mut out = PortfolioPnlToday::default();
    for row in &rows {
        out.realized_pnl_usd += row.net_pnl_usd;
        out.funding_usd += row.funding_usd;
        out.fee_rebate_usd -= row.fee_usd;
    }
    out.evidence = pnl_evidence(&rows, source, now_ms);
    out
}

pub(super) fn daily_history_from_closed_rows(
    closed: &[ClosedRealizedRow],
    now_ms: i64,
    days: u32,
) -> Vec<(i64, f64)> {
    let today = day_start_ms(now_ms);
    let first_day = history_window_start_ms(now_ms, days);
    let mut values = BTreeMap::<i64, f64>::new();
    for entry in closed {
        if entry.closed_at_ms < first_day || entry.closed_at_ms >= today {
            continue;
        }
        *values.entry(day_start_ms(entry.closed_at_ms)).or_default() += entry.row.net_pnl_usd;
    }
    (0..days)
        .map(|idx| {
            let day = first_day + i64::from(idx) * DAY_MS;
            (day, values.remove(&day).unwrap_or(0.0))
        })
        .collect()
}

fn terminal_close_times(close_runs: &[CloseRun]) -> BTreeMap<String, i64> {
    close_runs
        .iter()
        .filter(|run| terminal_close_status(run.status))
        .map(|run| (run.id.clone(), run.updated_at_ms))
        .collect()
}

fn terminal_close_status(status: CloseRunStatus) -> bool {
    matches!(
        status,
        CloseRunStatus::Succeeded | CloseRunStatus::Compensated | CloseRunStatus::ManuallyResolved
    )
}
