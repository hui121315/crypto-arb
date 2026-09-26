use shared_types::{
    ExecutedTrade, LiveOrderState, OrderRecord, OrderSide, OrderSource, ReviewPnlEvidence,
    ReviewPnlField, StrategyKind,
};
use std::collections::BTreeMap;

const DEFAULT_FEE_RATE: f64 = 0.0005;

#[derive(Debug, Clone, PartialEq)]
pub struct ExecutedTradePage {
    pub rows: Vec<ExecutedTrade>,
    pub total_rows: usize,
    pub group_ids: Vec<String>,
}

pub fn normalize_net_pnl(mut trade: ExecutedTrade) -> ExecutedTrade {
    trade.net_pnl_usd = trade.gross_pnl_usd + trade.funding_usd - trade.fee_usd;
    trade
}

pub fn executed_from_orders(rows: &[OrderRecord], now_ms: i64, days: u32) -> Vec<ExecutedTrade> {
    executed_page_from_orders(rows, now_ms, days, 0, usize::MAX).rows
}

pub fn executed_page_from_orders(
    rows: &[OrderRecord],
    now_ms: i64,
    days: u32,
    offset: usize,
    limit: usize,
) -> ExecutedTradePage {
    let groups = executed_groups(rows, now_ms, days);
    let total_rows = groups.len();
    let group_ids = groups.iter().map(|group| group.id.clone()).collect();
    let rows = groups
        .into_iter()
        .skip(offset)
        .take(limit)
        .filter_map(|group| trade_from_group_refs(group.id, &group.orders))
        .collect();
    ExecutedTradePage {
        rows,
        total_rows,
        group_ids,
    }
}

fn executed_groups<'a>(rows: &'a [OrderRecord], now_ms: i64, days: u32) -> Vec<ExecutedGroup<'a>> {
    let min_ms = now_ms - i64::from(days.max(1)) * 24 * 60 * 60_000;
    let mut groups: BTreeMap<String, Vec<&'a OrderRecord>> = BTreeMap::new();
    for row in rows {
        if row.intent.source != OrderSource::ArbitragePreview
            || row.intent.created_at_ms < min_ms
            || !is_executed_state(row.state)
        {
            continue;
        }
        groups
            .entry(hedge_group_id(&row.intent.id))
            .or_default()
            .push(row);
    }

    let mut groups = groups
        .into_iter()
        .filter_map(|(id, orders)| executed_group(id, orders))
        .collect::<Vec<_>>();
    groups.sort_by(|left, right| {
        right
            .opened_at_ms
            .cmp(&left.opened_at_ms)
            .then_with(|| left.id.cmp(&right.id))
    });
    groups
}

#[derive(Debug, Clone)]
struct ExecutedGroup<'a> {
    id: String,
    opened_at_ms: i64,
    orders: Vec<&'a OrderRecord>,
}

fn executed_group(id: String, orders: Vec<&OrderRecord>) -> Option<ExecutedGroup<'_>> {
    let first = orders.first()?;
    let opened_at_ms = orders
        .iter()
        .map(|row| row.intent.created_at_ms)
        .min()
        .unwrap_or(first.intent.created_at_ms);
    let has_long = orders.iter().any(|row| is_side_order(row, OrderSide::Buy));
    let has_short = orders.iter().any(|row| is_side_order(row, OrderSide::Sell));
    (has_long && has_short).then_some(ExecutedGroup {
        id,
        opened_at_ms,
        orders,
    })
}

fn trade_from_group_refs(id: String, orders: &[&OrderRecord]) -> Option<ExecutedTrade> {
    let first = orders.first()?;
    let opened_at_ms = orders
        .iter()
        .map(|row| row.intent.created_at_ms)
        .min()
        .unwrap_or(first.intent.created_at_ms);
    let long_orders = side_orders(orders, OrderSide::Buy);
    let short_orders = side_orders(orders, OrderSide::Sell);
    let gross_pnl_usd = gross_spread_pnl(&long_orders, &short_orders);
    let fee_usd = estimate_group_fee(&long_orders, &short_orders);
    let estimated_fields = estimated_fields(gross_pnl_usd, fee_usd);
    let missing_fields = missing_fields(gross_pnl_usd, fee_usd);
    Some(normalize_net_pnl(ExecutedTrade {
        id,
        strategy: strategy(&long_orders, &short_orders),
        symbol: first.intent.symbol.clone(),
        long_venue: venue(&long_orders)?,
        short_venue: venue(&short_orders)?,
        opened_at_ms,
        closed_at_ms: None,
        holding_minutes: None,
        gross_pnl_usd: gross_pnl_usd.unwrap_or_default(),
        fee_usd: fee_usd.unwrap_or_default(),
        funding_usd: 0.0,
        slippage_usd: 0.0,
        net_pnl_usd: 0.0,
        evidence: ReviewPnlEvidence::default(),
        actual_fields: Vec::new(),
        estimated_fields,
        missing_fields,
        long_orders,
        short_orders,
    }))
}

fn side_orders(rows: &[&OrderRecord], side: OrderSide) -> Vec<OrderRecord> {
    rows.iter()
        .filter(|row| is_side_order(row, side))
        .map(|row| (*row).clone())
        .collect()
}

fn is_side_order(row: &OrderRecord, side: OrderSide) -> bool {
    row.intent.side == side && !row.intent.reduce_only
}

fn venue(rows: &[OrderRecord]) -> Option<String> {
    rows.first().map(|row| row.intent.exchange.clone())
}

fn strategy(long_orders: &[OrderRecord], short_orders: &[OrderRecord]) -> StrategyKind {
    long_orders
        .iter()
        .chain(short_orders)
        .find_map(|row| row.intent.strategy)
        .unwrap_or(StrategyKind::PerpCross)
}

fn gross_spread_pnl(long_orders: &[OrderRecord], short_orders: &[OrderRecord]) -> Option<f64> {
    Some(signed_notional(short_orders)? - signed_notional(long_orders)?)
}

fn signed_notional(rows: &[OrderRecord]) -> Option<f64> {
    rows.iter()
        .try_fold(0.0, |sum, row| Some(sum + order_notional(row)?))
}

fn order_notional(row: &OrderRecord) -> Option<f64> {
    let price = finite_non_negative(row.filled_price.or(row.intent.price)?)?;
    let quantity = finite_non_negative(row.filled_quantity.unwrap_or(row.intent.quantity))?;
    Some(quantity * price)
}

fn finite_non_negative(value: f64) -> Option<f64> {
    value.is_finite().then_some(value.max(0.0))
}

fn estimate_group_fee(long_orders: &[OrderRecord], short_orders: &[OrderRecord]) -> Option<f64> {
    Some(fee_estimate(long_orders)? + fee_estimate(short_orders)?)
}

fn fee_estimate(rows: &[OrderRecord]) -> Option<f64> {
    rows.iter().try_fold(0.0, |sum, row| {
        let notional = row
            .risk
            .as_ref()
            .and_then(|risk| finite_non_negative(risk.computed_notional))
            .or_else(|| order_notional(row))?;
        Some(sum + notional * DEFAULT_FEE_RATE)
    })
}

fn estimated_fields(gross_pnl_usd: Option<f64>, fee_usd: Option<f64>) -> Vec<ReviewPnlField> {
    let mut fields = Vec::with_capacity(3);
    if gross_pnl_usd.is_some() {
        fields.push(ReviewPnlField::Gross);
    }
    if fee_usd.is_some() {
        fields.push(ReviewPnlField::Fee);
    }
    if gross_pnl_usd.is_some() && fee_usd.is_some() {
        fields.push(ReviewPnlField::Net);
    }
    fields
}

fn missing_fields(gross_pnl_usd: Option<f64>, fee_usd: Option<f64>) -> Vec<ReviewPnlField> {
    let mut fields = vec![ReviewPnlField::Funding, ReviewPnlField::Slippage];
    if gross_pnl_usd.is_none() {
        fields.push(ReviewPnlField::Gross);
    }
    if fee_usd.is_none() {
        fields.push(ReviewPnlField::Fee);
    }
    if gross_pnl_usd.is_none() || fee_usd.is_none() {
        fields.push(ReviewPnlField::Net);
    }
    fields
}

fn hedge_group_id(id: &str) -> String {
    id.strip_suffix("-long")
        .or_else(|| id.strip_suffix("-short"))
        .or_else(|| id.strip_suffix("-unwind"))
        .unwrap_or(id)
        .to_owned()
}

fn is_executed_state(state: LiveOrderState) -> bool {
    state == LiveOrderState::Filled
}

#[cfg(test)]
mod tests;
