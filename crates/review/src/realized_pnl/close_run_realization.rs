use super::amounts::day_start_ms;
use super::RealizedPnlRow;
use shared_types::{
    CloseLeg, CloseLegStatus, CloseRun, CloseRunScope, CloseRunStatus, ExecutionLedgerQuality,
    ExecutionMode, LiveOrderState, OrderSide, OrderUpdateSource, PositionSide,
};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn apply_close_run_realization(
    rows: &mut BTreeMap<String, RealizedPnlRow>,
    close_runs: &[CloseRun],
) {
    for row in rows.values_mut() {
        apply_row_close_run_realization(row, close_runs);
    }
}

fn apply_row_close_run_realization(row: &mut RealizedPnlRow, close_runs: &[CloseRun]) {
    let keys = row_close_run_link_keys(row);
    if keys.is_empty() {
        return;
    }
    let realizations = close_runs
        .iter()
        .filter_map(|run| complete_pair_realization(run, &keys))
        .collect::<Vec<_>>();
    let [realization] = realizations.as_slice() else {
        return;
    };

    row.buy_notional_usd += realization.buy_notional_usd;
    row.sell_notional_usd += realization.sell_notional_usd;
    row.price_pnl_usd = row.sell_notional_usd - row.buy_notional_usd;
    row.net_pnl_usd += realization.sell_notional_usd - realization.buy_notional_usd;
    row.realized_at_ms = row.realized_at_ms.max(realization.closed_at_ms);
    row.realized_day_ms = day_start_ms(row.realized_at_ms);
    row.closed_at_ms = Some(realization.closed_at_ms);
    row.close_price_quality = Some(realization.quality);
}

fn row_close_run_link_keys(row: &RealizedPnlRow) -> BTreeSet<(String, String)> {
    row.evidence
        .ledger_events
        .iter()
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

fn complete_pair_realization(
    run: &CloseRun,
    keys: &BTreeSet<(String, String)>,
) -> Option<ClosePriceRealization> {
    if run.scope != CloseRunScope::Pair
        || run.status != CloseRunStatus::Succeeded
        || run.expected_leg_count != 2
        || run.failed_leg_count != 0
        || run.naked_exposure_usd.abs() > f64::EPSILON
    {
        return None;
    }
    let legs = run
        .legs
        .iter()
        .filter(|leg| {
            leg.pair_evidence
                .as_ref()
                .is_some_and(|pair| keys.contains(&(pair.run_id.clone(), pair.ticket_id.clone())))
        })
        .collect::<Vec<_>>();
    if legs.len() != 2
        || !legs.iter().any(|leg| leg.side == PositionSide::Long)
        || !legs.iter().any(|leg| leg.side == PositionSide::Short)
    {
        return None;
    }

    let mut realization = ClosePriceRealization {
        buy_notional_usd: 0.0,
        sell_notional_usd: 0.0,
        closed_at_ms: 0,
        quality: ExecutionLedgerQuality::Actual,
    };
    for leg in legs {
        let fill = close_leg_fill(leg)?;
        match fill.side {
            OrderSide::Buy => realization.buy_notional_usd += fill.notional_usd,
            OrderSide::Sell => realization.sell_notional_usd += fill.notional_usd,
        }
        realization.closed_at_ms = realization.closed_at_ms.max(fill.confirmed_at_ms);
        realization.quality = combine_quality(realization.quality, fill.quality);
    }
    (realization.buy_notional_usd > 0.0
        && realization.sell_notional_usd > 0.0
        && realization.closed_at_ms > 0)
        .then_some(realization)
}

#[derive(Debug, Clone, Copy)]
struct CloseLegFill {
    side: OrderSide,
    notional_usd: f64,
    confirmed_at_ms: i64,
    quality: ExecutionLedgerQuality,
}

fn close_leg_fill(leg: &CloseLeg) -> Option<CloseLegFill> {
    if leg.status != CloseLegStatus::Filled {
        return None;
    }
    let order = leg.order.as_ref()?;
    let expected_side = match leg.side {
        PositionSide::Long => OrderSide::Sell,
        PositionSide::Short => OrderSide::Buy,
    };
    if order.state != LiveOrderState::Filled
        || !order.intent.reduce_only
        || order.intent.side != expected_side
    {
        return None;
    }
    let quantity = positive_finite(order.filled_quantity?)?;
    let price = positive_finite(order.filled_price?)?;
    let confirmed_at_ms = leg.confirmed_filled_at_ms?;
    Some(CloseLegFill {
        side: order.intent.side,
        notional_usd: quantity * price,
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
