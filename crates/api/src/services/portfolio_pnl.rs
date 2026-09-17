use crate::state::AppState;
use shared_types::{CloseRun, ExecutionLedgerEvent, OrderRecord, PortfolioPnlEvidence};
#[cfg(test)]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

mod closed;
mod evidence;

use closed::{closed_realized_rows, daily_history_from_closed_rows, today_from_closed_rows};
use evidence::pnl_evidence;
#[cfg(test)]
use evidence::PNL_FIELDS;

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;
const HISTORY_LOOKBACK_DAYS: u32 = 60;
const PNL_SOURCE_LEDGER: &str = "trading_execution_ledger+close_runs";
const PNL_SOURCE_SQL: &str = "trading_sql_realized_window+execution_ledger+close_runs";

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PortfolioPnlToday {
    pub realized_pnl_usd: f64,
    pub funding_usd: f64,
    pub fee_rebate_usd: f64,
    pub evidence: PortfolioPnlEvidence,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PortfolioPnlSnapshot {
    pub(crate) today: PortfolioPnlToday,
    pub(crate) history: Vec<(i64, f64)>,
}

pub(crate) async fn snapshot(state: &AppState, now_ms: i64) -> PortfolioPnlSnapshot {
    let from_ms = history_window_start_ms(now_ms, HISTORY_LOOKBACK_DAYS);
    let to_ms = now_ms.saturating_add(1);
    let realized = realized_ledger_from_state(state, from_ms, to_ms).await;
    snapshot_from_ledger_with_source(
        &realized.orders,
        &realized.ledger,
        &realized.close_runs,
        now_ms,
        realized.source,
    )
}

#[cfg(test)]
pub(crate) async fn today(state: &AppState, now_ms: i64) -> PortfolioPnlToday {
    let from_ms = day_start_ms(now_ms);
    let to_ms = now_ms.saturating_add(1);
    let realized = realized_ledger_from_state(state, from_ms, to_ms).await;
    today_from_ledger_with_source(
        &realized.orders,
        &realized.ledger,
        &realized.close_runs,
        now_ms,
        realized.source,
    )
}

struct PortfolioRealizedLedger {
    ledger: Vec<ExecutionLedgerEvent>,
    orders: Vec<OrderRecord>,
    close_runs: Vec<CloseRun>,
    source: &'static str,
}

#[cfg(test)]
fn snapshot_from_ledger(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    now_ms: i64,
) -> PortfolioPnlSnapshot {
    snapshot_from_ledger_with_source(orders, ledger, close_runs, now_ms, PNL_SOURCE_LEDGER)
}

fn snapshot_from_ledger_with_source(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    now_ms: i64,
    source: &'static str,
) -> PortfolioPnlSnapshot {
    let closed = closed_realized_rows(orders, ledger, close_runs, now_ms.saturating_add(1));
    PortfolioPnlSnapshot {
        today: today_from_closed_rows(&closed, now_ms, source),
        history: daily_history_from_closed_rows(&closed, now_ms, HISTORY_LOOKBACK_DAYS),
    }
}

async fn realized_ledger_from_state(
    state: &AppState,
    from_ms: i64,
    to_ms: i64,
) -> PortfolioRealizedLedger {
    if to_ms <= from_ms {
        return PortfolioRealizedLedger {
            ledger: Vec::new(),
            orders: Vec::new(),
            close_runs: Vec::new(),
            source: PNL_SOURCE_LEDGER,
        };
    }
    let service = state.trading_service();
    if let Some(window) = service.list_sql_realized_window(from_ms, to_ms).await {
        return realized_ledger_from_sql_window(window);
    }
    let ledger = service.list_execution_ledger_events_for_realized_window(from_ms, to_ms);
    let orders = orders_from_ledger(state, &ledger);
    let close_runs = close_run_snapshots(state);
    PortfolioRealizedLedger {
        ledger,
        orders,
        close_runs,
        source: PNL_SOURCE_LEDGER,
    }
}

fn realized_ledger_from_sql_window(window: trading::SqlRealizedWindow) -> PortfolioRealizedLedger {
    PortfolioRealizedLedger {
        ledger: window.events,
        orders: window.order_snapshots,
        close_runs: window.close_runs,
        source: PNL_SOURCE_SQL,
    }
}

fn close_run_snapshots(state: &AppState) -> Vec<CloseRun> {
    state
        .close_runs()
        .iter()
        .map(|entry| entry.value().clone())
        .collect()
}

fn orders_from_ledger(state: &AppState, ledger: &[ExecutionLedgerEvent]) -> Vec<OrderRecord> {
    let service = state.trading_service();
    ledger
        .iter()
        .map(|event| event.order.identity.internal_order_id.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|order_id| service.get_order(order_id))
        .collect()
}

#[cfg(test)]
fn today_from_ledger(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    now_ms: i64,
) -> PortfolioPnlToday {
    today_from_ledger_with_source(orders, ledger, close_runs, now_ms, PNL_SOURCE_LEDGER)
}

#[cfg(test)]
fn today_from_ledger_with_source(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    now_ms: i64,
    source: &'static str,
) -> PortfolioPnlToday {
    let day_start = day_start_ms(now_ms);
    let realized = review_domain::realized_pnl_by_group_with_close_runs(
        orders,
        ledger,
        close_runs,
        day_start,
        now_ms.saturating_add(1),
    );
    let mut out = PortfolioPnlToday::default();
    for row in realized.values() {
        out.realized_pnl_usd += row.net_pnl_usd;
        out.funding_usd += row.funding_usd;
        out.fee_rebate_usd -= row.fee_usd;
    }
    let rows = realized.values().collect::<Vec<_>>();
    out.evidence = pnl_evidence(&rows, source, now_ms);
    out
}

#[cfg(test)]
fn daily_history_from_ledger(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    now_ms: i64,
    days: u32,
) -> Vec<(i64, f64)> {
    let today = day_start_ms(now_ms);
    let first_day = history_window_start_ms(now_ms, days);
    let mut values = realized_by_day(orders, ledger, close_runs, first_day, today);
    (0..days)
        .map(|idx| {
            let day = first_day + i64::from(idx) * DAY_MS;
            (day, values.remove(&day).unwrap_or(0.0))
        })
        .collect()
}

#[cfg(test)]
fn realized_by_day(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    from_ms: i64,
    to_ms: i64,
) -> BTreeMap<i64, f64> {
    let mut out = BTreeMap::new();
    for row in review_domain::realized_pnl_by_group_with_close_runs(
        orders, ledger, close_runs, from_ms, to_ms,
    )
    .values()
    {
        *out.entry(row.realized_day_ms).or_default() += row.net_pnl_usd;
    }
    out
}

fn day_start_ms(ts: i64) -> i64 {
    ts - ts.rem_euclid(DAY_MS)
}

fn history_window_start_ms(now_ms: i64, days: u32) -> i64 {
    day_start_ms(now_ms).saturating_sub(i64::from(days) * DAY_MS)
}

#[cfg(test)]
mod tests;
