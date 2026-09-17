use super::paging::review_problem;
use super::*;

pub(super) async fn realized_ledger_from_trading(
    service: &TradingService,
    from_ms: i64,
    to_ms: i64,
) -> ReviewRealizedLedger {
    if let Some(window) = service.list_sql_realized_window(from_ms, to_ms).await {
        ReviewRealizedLedger {
            ledger: window.events,
            orders: window.order_snapshots,
            close_runs: window.close_runs,
        }
    } else {
        let ledger = service.list_execution_ledger_events_for_realized_window(from_ms, to_ms);
        let orders = orders_from_ledger(service, &ledger);
        ReviewRealizedLedger {
            ledger,
            orders,
            close_runs: Vec::new(),
        }
    }
}

pub(super) struct ReviewRealizedLedger {
    pub ledger: Vec<ExecutionLedgerEvent>,
    pub orders: Vec<OrderRecord>,
    pub close_runs: Vec<CloseRun>,
}

pub(super) fn ledger_status(
    ledger: &[ExecutionLedgerEvent],
    rows: &[ExecutedTrade],
) -> ReviewLedgerStatus {
    if ledger.is_empty() {
        ReviewLedgerStatus::NoLedgerEvents
    } else if rows.is_empty() {
        ReviewLedgerStatus::NoCompleteRows
    } else if rows.iter().any(|row| !row.missing_fields.is_empty()) {
        ReviewLedgerStatus::PartialEvidence
    } else {
        ReviewLedgerStatus::LedgerBacked
    }
}

fn push_missing_field(fields: &mut Vec<ReviewPnlField>, field: ReviewPnlField) {
    if !fields.contains(&field) {
        fields.push(field);
    }
}

pub(super) fn aggregate_missing_fields_for_trades(trades: &[ExecutedTrade]) -> Vec<ReviewPnlField> {
    let mut fields = Vec::new();
    for trade in trades {
        for field in &trade.missing_fields {
            push_missing_field(&mut fields, *field);
        }
    }
    fields
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ReviewLedgerProblemContext<'a> {
    pub ledger_status: ReviewLedgerStatus,
    pub window_days: u32,
    pub window_from_ms: i64,
    pub window_to_ms: i64,
    pub ledger_event_count: usize,
    pub row_count: usize,
    pub missing_fields: &'a [ReviewPnlField],
    pub sample_count: u32,
    pub excluded_incomplete_count: u32,
}

pub(super) fn review_status_for_ledger_context(
    context: ReviewLedgerProblemContext<'_>,
) -> (ListStatus, Vec<ApiProblem>) {
    if matches!(
        context.ledger_status,
        ReviewLedgerStatus::LedgerBacked | ReviewLedgerStatus::NoLedgerEvents
    ) {
        return (ListStatus::Fresh, Vec::new());
    }
    (ListStatus::Degraded, vec![review_ledger_problem(context)])
}

pub(super) fn merge_review_problems(
    page_status: ListStatus,
    mut page_problems: Vec<ApiProblem>,
    ledger_status: ListStatus,
    ledger_problems: Vec<ApiProblem>,
) -> (ListStatus, Vec<ApiProblem>) {
    page_problems.extend(ledger_problems);
    let status = if page_status == ListStatus::Degraded || ledger_status == ListStatus::Degraded {
        ListStatus::Degraded
    } else {
        ListStatus::Fresh
    };
    (status, page_problems)
}

pub(super) fn saturating_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

pub(super) fn review_ledger_problem(context: ReviewLedgerProblemContext<'_>) -> ApiProblem {
    review_problem(
        codes::REVIEW_LEDGER_INCOMPLETE,
        "review execution ledger has incomplete PnL evidence",
        serde_json::json!({
            "ledgerStatus": context.ledger_status,
            "windowDays": context.window_days,
            "windowFromMs": context.window_from_ms,
            "windowToMs": context.window_to_ms,
            "ledgerEventCount": context.ledger_event_count,
            "rowCount": context.row_count,
            "missingFields": context.missing_fields,
            "sampleCount": context.sample_count,
            "excludedIncompleteCount": context.excluded_incomplete_count,
        }),
    )
}

pub(super) fn ledger_status_for_total(
    ledger: &[ExecutionLedgerEvent],
    total_rows: usize,
    missing_fields: &[ReviewPnlField],
) -> ReviewLedgerStatus {
    if ledger.is_empty() {
        ReviewLedgerStatus::NoLedgerEvents
    } else if total_rows == 0 {
        ReviewLedgerStatus::NoCompleteRows
    } else if !missing_fields.is_empty() {
        ReviewLedgerStatus::PartialEvidence
    } else {
        ReviewLedgerStatus::LedgerBacked
    }
}

fn orders_from_ledger(
    service: &TradingService,
    ledger: &[ExecutionLedgerEvent],
) -> Vec<OrderRecord> {
    ledger
        .iter()
        .map(|event| event.order.identity.internal_order_id.as_str())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .filter_map(|order_id| service.get_order(order_id))
        .collect()
}

pub(super) fn ledger_projected_orders(
    orders: &[OrderRecord],
    realized: &BTreeMap<String, review_domain::RealizedPnlRow>,
) -> Vec<OrderRecord> {
    let filled_ids = realized_order_ids(realized);
    orders
        .iter()
        .filter(|order| filled_ids.contains(&order.intent.id))
        .map(|order| {
            let mut order = order.clone();
            order.state = LiveOrderState::Filled;
            order
        })
        .collect()
}

fn realized_order_ids(
    realized: &BTreeMap<String, review_domain::RealizedPnlRow>,
) -> BTreeSet<String> {
    realized
        .values()
        .flat_map(|row| row.order_ids.iter().cloned())
        .collect()
}
