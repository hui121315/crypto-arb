use super::amounts::day_start_ms;
use super::RealizedPnlRow;
use shared_types::{
    venue_names_equal, CloseLeg, CloseLegStatus, CloseRun, ExecutionLedgerQuality, ExecutionMode,
    LiveOrderState, OrderRecord, OrderSide, OrderUpdateSource, PositionSide,
    ReviewLedgerPayloadEvidence,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn apply_close_run_realization(
    rows: &mut BTreeMap<String, RealizedPnlRow>,
    orders: &[OrderRecord],
    close_runs: &[&CloseRun],
    to_ms: i64,
    position_symbol: fn(&str) -> String,
) {
    for row in rows.values_mut() {
        if let Some(realization) =
            complete_realization(row, orders, close_runs, to_ms, position_symbol)
        {
            row.buy_notional_usd += realization.buy_notional_usd;
            row.sell_notional_usd += realization.sell_notional_usd;
            row.price_pnl_usd = row.sell_notional_usd - row.buy_notional_usd;
            row.net_pnl_usd += realization.sell_notional_usd - realization.buy_notional_usd;
            row.realized_at_ms = row.realized_at_ms.max(realization.closed_at_ms);
            row.realized_day_ms = day_start_ms(row.realized_at_ms);
            row.closed_at_ms = Some(realization.closed_at_ms);
            row.close_price_quality = Some(realization.quality);
        }
    }
}

pub(super) fn row_close_run_link_keys(row: &RealizedPnlRow) -> BTreeSet<(String, String)> {
    row.evidence
        .ledger_events
        .iter()
        .filter(|event| row.evidence.fill_event_ids.contains(&event.event_id))
        .filter_map(|event| Some((event.order.run_id.clone()?, event.order.ticket_id.clone()?)))
        .collect()
}

#[derive(Debug, Clone, Copy)]
struct ClosePriceRealization {
    buy_notional_usd: f64,
    sell_notional_usd: f64,
    closed_at_ms: i64,
    quality: ExecutionLedgerQuality,
}

fn complete_realization(
    row: &RealizedPnlRow,
    orders: &[OrderRecord],
    close_runs: &[&CloseRun],
    to_ms: i64,
    position_symbol: fn(&str) -> String,
) -> Option<ClosePriceRealization> {
    let openings = opening_fills(row, orders, position_symbol)?;
    let keys = row_close_run_link_keys(row);
    let mut fills = BTreeMap::<&str, CloseLegFill>::new();
    for run in close_runs {
        for leg in &run.legs {
            let Some(pair) = leg
                .pair_evidence
                .as_ref()
                .filter(|pair| keys.contains(&(pair.run_id.clone(), pair.ticket_id.clone())))
            else {
                continue;
            };
            let index = openings.iter().position(|open| {
                open.matches(&pair.venue, &pair.symbol, pair.side)
                    && open.run_id == pair.run_id
                    && open.ticket_id == pair.ticket_id
            })?;
            let open = &openings[index];
            if !open.matches(&leg.venue, &leg.symbol, leg.side)
                || !openings[1 - index].matches(
                    &pair.partner_venue,
                    &pair.partner_symbol,
                    pair.partner_side,
                )
            {
                return None;
            }
            // Compensation here reopens positions; it is not another reducing close.
            if run.unwind_plan.as_ref().is_some_and(|plan| !plan.compensation_attempts.is_empty()) {
                return None;
            }
            let fill = close_leg_fill(leg, index, open, to_ms)?;
            let id = leg.order.as_ref()?.intent.id.as_str();
            if id.trim().is_empty() {
                return None;
            }
            if let Some(previous) = fills.insert(id, fill) {
                if previous != fill {
                    return None;
                }
            }
        }
    }
    let mut quantities = [0.0; 2];
    let mut realization = ClosePriceRealization {
        buy_notional_usd: 0.0,
        sell_notional_usd: 0.0,
        closed_at_ms: 0,
        quality: ExecutionLedgerQuality::Actual,
    };
    for fill in fills.values() {
        quantities[fill.opening_index] += fill.quantity;
        match fill.side {
            OrderSide::Buy => realization.buy_notional_usd += fill.notional_usd,
            OrderSide::Sell => realization.sell_notional_usd += fill.notional_usd,
        }
        realization.closed_at_ms = realization.closed_at_ms.max(fill.confirmed_at_ms);
        realization.quality = combine_quality(realization.quality, fill.quality);
    }
    (openings
        .iter()
        .enumerate()
        .all(|(index, open)| quantity_matches(quantities[index], open.quantity))
        && realization.buy_notional_usd.is_finite()
        && realization.sell_notional_usd.is_finite()
        && realization.buy_notional_usd > 0.0
        && realization.sell_notional_usd > 0.0
        && realization.closed_at_ms > 0)
        .then_some(realization)
}

struct OpeningFill<'a> {
    order: &'a OrderRecord,
    position_symbol: String,
    run_id: &'a str,
    ticket_id: &'a str,
    quantity: f64,
    last_fill_at_ms: i64,
}

impl OpeningFill<'_> {
    fn matches(&self, venue: &str, symbol: &str, side: PositionSide) -> bool {
        venue_names_equal(&self.order.intent.exchange, venue)
            && self.matches_symbol(symbol)
            && self.order.intent.side
                == match side {
                    PositionSide::Long => OrderSide::Buy,
                    PositionSide::Short => OrderSide::Sell,
                }
    }

    fn matches_symbol(&self, symbol: &str) -> bool {
        self.order.intent.symbol.eq_ignore_ascii_case(symbol)
            || self.position_symbol.eq_ignore_ascii_case(symbol)
    }
}

fn opening_fills<'a>(
    row: &'a RealizedPnlRow,
    orders: &'a [OrderRecord],
    position_symbol: fn(&str) -> String,
) -> Option<Vec<OpeningFill<'a>>> {
    if row.order_ids.len() != 2 {
        return None;
    }
    let mut openings = Vec::new();
    for id in &row.order_ids {
        let order = orders.iter().find(|order| &order.intent.id == id)?;
        let mut open: Option<OpeningFill<'_>> = None;
        let mut seen = BTreeSet::new();
        for event in row.evidence.ledger_events.iter().filter(|event| {
            &event.order.identity.internal_order_id == id
                && row.evidence.fill_event_ids.contains(&event.event_id)
        }) {
            if !seen.insert(&event.event_id) {
                continue;
            }
            let ReviewLedgerPayloadEvidence::Fill { quantity, .. } = &event.payload else {
                return None;
            };
            positive_finite(*quantity)?;
            let run_id = event.order.run_id.as_deref().filter(|id| !id.is_empty())?;
            let ticket_id = event
                .order
                .ticket_id
                .as_deref()
                .filter(|id| !id.is_empty())?;
            if !venue_names_equal(&event.order.exchange, &order.intent.exchange)
                || !event
                    .order
                    .symbol
                    .eq_ignore_ascii_case(&order.intent.symbol)
                || event.order.side != order.intent.side
            {
                return None;
            }
            let open = open.get_or_insert(OpeningFill {
                order,
                position_symbol: position_symbol(&order.intent.symbol),
                run_id,
                ticket_id,
                quantity: 0.0,
                last_fill_at_ms: 0,
            });
            if open.run_id != run_id || open.ticket_id != ticket_id {
                return None;
            }
            open.quantity += quantity;
            open.last_fill_at_ms = open.last_fill_at_ms.max(event.timing.occurred_at_ms);
        }
        let open = open?;
        if !quantity_matches(
            open.quantity,
            order.filled_quantity.unwrap_or(order.intent.quantity),
        ) {
            return None;
        }
        openings.push(open);
    }
    (openings[0].order.intent.side != openings[1].order.intent.side
        && openings[0].run_id == openings[1].run_id
        && openings[0].ticket_id == openings[1].ticket_id)
        .then_some(openings)
}

fn quantity_matches(actual: f64, expected: f64) -> bool {
    actual.is_finite()
        && expected.is_finite()
        && actual > 0.0
        && expected > 0.0
        && (actual - expected).abs() <= f64::EPSILON * 32.0 * expected.abs()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CloseLegFill {
    opening_index: usize,
    quantity: f64,
    side: OrderSide,
    notional_usd: f64,
    confirmed_at_ms: i64,
    quality: ExecutionLedgerQuality,
}

fn close_leg_fill(
    leg: &CloseLeg,
    opening_index: usize,
    open: &OpeningFill<'_>,
    to_ms: i64,
) -> Option<CloseLegFill> {
    let order = leg.order.as_ref()?;
    let expected_side = match leg.side {
        PositionSide::Long => OrderSide::Sell,
        PositionSide::Short => OrderSide::Buy,
    };
    if !order.intent.reduce_only
        || order.intent.side != expected_side
        || !open.matches_symbol(&order.intent.symbol)
        || order.intent.mode != open.order.intent.mode
        || !venue_names_equal(&order.intent.exchange, &leg.venue)
        || !quantity_matches(order.intent.quantity, leg.quantity)
    {
        return None;
    }
    let quantity = order.filled_quantity.filter(|quantity| quantity.is_finite() && *quantity >= 0.0)?;
    let (notional_usd, confirmed_at_ms) = if leg.has_complete_fill() {
        (quantity * positive_finite(order.filled_price?)?, leg.confirmed_filled_at_ms?)
    } else {
        let remote = |source| matches!(source, OrderUpdateSource::PrivateWs | OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile);
        let terminal = matches!((leg.status, order.state),
            (CloseLegStatus::Cancelled, LiveOrderState::Cancelled)
                | (CloseLegStatus::Rejected, LiveOrderState::Rejected));
        if !terminal || !remote(order.last_update_source) || !leg.finality_source.is_some_and(remote)
            || (quantity > leg.quantity && !quantity_matches(quantity, leg.quantity)) {
            return None;
        }
        if quantity == 0.0 {
            return Some(CloseLegFill {
                opening_index, quantity, side: order.intent.side, notional_usd: 0.0,
                confirmed_at_ms: 0, quality: close_fill_quality(order.intent.mode, leg.finality_source),
            });
        }
        if order.state != LiveOrderState::Cancelled { return None; }
        // A later cancel/query time must not move already executed cash flow to another day.
        let fills = leg.ledger_fills.as_ref()?;
        if fills.event_ids.is_empty() || fills.event_ids.iter().any(|id| id.trim().is_empty())
            || !quantity_matches(fills.totals.quantity, quantity) {
            return None;
        }
        (positive_finite(fills.totals.notional)?, fills.last_fill_at_ms?)
    };
    if confirmed_at_ms < open.last_fill_at_ms || confirmed_at_ms <= 0 || confirmed_at_ms >= to_ms {
        return None;
    }
    Some(CloseLegFill {
        opening_index,
        quantity,
        side: order.intent.side,
        notional_usd,
        confirmed_at_ms,
        quality: close_fill_quality(order.intent.mode, leg.finality_source),
    })
}

fn close_fill_quality(
    mode: ExecutionMode,
    source: Option<OrderUpdateSource>,
) -> ExecutionLedgerQuality {
    match mode {
        ExecutionMode::DryRun | ExecutionMode::Testnet => ExecutionLedgerQuality::Estimated,
        ExecutionMode::Live
            if matches!(
                source,
                Some(
                    OrderUpdateSource::PrivateWs
                        | OrderUpdateSource::OrderQuery
                        | OrderUpdateSource::Reconcile
                )
            ) =>
        {
            ExecutionLedgerQuality::Actual
        }
        ExecutionMode::Live => ExecutionLedgerQuality::Missing,
    }
}

fn combine_quality(
    left: ExecutionLedgerQuality,
    right: ExecutionLedgerQuality,
) -> ExecutionLedgerQuality {
    match (left, right) {
        (ExecutionLedgerQuality::Missing, _) | (_, ExecutionLedgerQuality::Missing) => {
            ExecutionLedgerQuality::Missing
        }
        (ExecutionLedgerQuality::Estimated, _) | (_, ExecutionLedgerQuality::Estimated) => {
            ExecutionLedgerQuality::Estimated
        }
        (ExecutionLedgerQuality::Actual, ExecutionLedgerQuality::Actual) => {
            ExecutionLedgerQuality::Actual
        }
    }
}

fn positive_finite(value: f64) -> Option<f64> {
    (value.is_finite() && value > 0.0).then_some(value)
}
