use shared_types::{
    CloseRun, ExecutedTrade, ExecutionLedgerEvent, ExecutionLedgerEventType,
    ExecutionLedgerPayload, ExecutionLedgerQuality, FeeLedgerSnapshot, FillLedgerSnapshot,
    FundingPaymentLedgerRecord, OrderRecord, OrderSide, OrderSource, OrderbookDepthLedgerRecord,
    ReviewLedgerEventEvidence, ReviewLedgerPayloadEvidence, ReviewPnlEvidence, ReviewPnlField,
    SlippageLedgerRecord,
};
use std::collections::{BTreeMap, BTreeSet};

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RealizedPnlRow {
    pub group_id: String,
    pub realized_at_ms: i64,
    pub realized_day_ms: i64,
    pub buy_notional_usd: f64,
    pub sell_notional_usd: f64,
    pub price_pnl_usd: f64,
    pub fee_usd: f64,
    pub funding_usd: f64,
    pub slippage_usd: f64,
    pub net_pnl_usd: f64,
    pub closed_at_ms: Option<i64>,
    pub close_price_quality: Option<ExecutionLedgerQuality>,
    pub order_ids: BTreeSet<String>,
    pub evidence: ReviewPnlEvidence,
}

pub fn realized_pnl_by_group(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    from_ms: i64,
    to_ms: i64,
) -> BTreeMap<String, RealizedPnlRow> {
    if to_ms <= from_ms {
        return BTreeMap::new();
    }
    let order_index = eligible_order_index(orders);
    let fills = fill_events_for_pnl(ledger, &order_index, to_ms);
    let fill_event_ids = fills
        .iter()
        .map(|event| event.event_id.clone())
        .collect::<BTreeSet<_>>();
    let fee_snapshots = latest_fee_snapshots(ledger, &order_index, to_ms);
    let funding_payments = funding_payments(ledger, &order_index, to_ms);
    let slippage_records = slippage_records(ledger, &order_index, to_ms, &fill_event_ids);
    let orderbook_events = orderbook_evidence_events(ledger, &order_index, to_ms);
    let mut groups = BTreeMap::<String, PnlGroup>::new();
    add_slippage_records(&mut groups, slippage_records, &order_index);
    add_fills(&mut groups, fills, &order_index);
    add_fee_snapshots(&mut groups, fee_snapshots, &order_index);
    add_funding_payments(&mut groups, funding_payments, &order_index);
    add_orderbook_evidence(&mut groups, orderbook_events, &order_index);
    groups
        .into_iter()
        .filter_map(|(group_id, group)| {
            group
                .into_row(group_id, from_ms, to_ms)
                .map(|row| (row.group_id.clone(), row))
        })
        .collect()
}

pub fn realized_pnl_by_group_with_close_runs(
    orders: &[OrderRecord],
    ledger: &[ExecutionLedgerEvent],
    close_runs: &[CloseRun],
    from_ms: i64,
    to_ms: i64,
) -> BTreeMap<String, RealizedPnlRow> {
    let mut rows = realized_pnl_by_group(orders, ledger, from_ms, to_ms);
    apply_close_run_realization(&mut rows, close_runs);
    apply_close_run_costs(&mut rows, close_runs);
    rows
}

pub fn apply_realized_pnl(
    mut trades: Vec<ExecutedTrade>,
    realized: &BTreeMap<String, RealizedPnlRow>,
) -> Vec<ExecutedTrade> {
    trades.retain(|trade| realized.contains_key(&trade.id));
    for trade in &mut trades {
        if let Some(row) = realized.get(&trade.id) {
            trade.gross_pnl_usd = row.price_pnl_usd;
            trade.fee_usd = row.fee_usd;
            trade.funding_usd = row.funding_usd;
            trade.slippage_usd = row.slippage_usd;
            trade.net_pnl_usd = row.net_pnl_usd;
            trade.closed_at_ms = row.closed_at_ms;
            trade.holding_minutes = row
                .closed_at_ms
                .and_then(|closed_at_ms| holding_minutes(trade.opened_at_ms, closed_at_ms));
            trade.evidence = row.evidence.clone();
            let fields = realized_pnl_field_quality(row);
            trade.actual_fields = fields.actual;
            trade.estimated_fields = fields.estimated;
            trade.missing_fields = fields.missing;
        }
    }
    trades
}

mod quality;
pub use quality::{realized_pnl_field_quality, RealizedPnlFieldQuality};

mod amounts;
mod close_run_costs;
mod close_run_realization;
mod events;
mod group;
#[cfg(test)]
mod tests;

use amounts::holding_minutes;
use close_run_costs::{apply_close_run_costs, close_run_cost_missing};
use close_run_realization::apply_close_run_realization;
use events::{
    fill_events_for_pnl, funding_payments, latest_fee_snapshots, orderbook_evidence_events,
    slippage_records,
};
use group::{
    add_fee_snapshots, add_fills, add_funding_payments, add_orderbook_evidence,
    add_slippage_records, eligible_order_index, PnlGroup,
};
