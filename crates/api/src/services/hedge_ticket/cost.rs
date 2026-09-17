use super::*;

pub(super) fn ticket_cost(
    opp: &ArbitrageOpportunityDto,
    long_fee: Option<&TradeFeeSnapshot>,
    short_fee: Option<&TradeFeeSnapshot>,
    long_leg: &HedgeLegQuote,
    short_leg: &HedgeLegQuote,
) -> Option<ExecutionCostProfile> {
    let mut base = opp.execution_cost.clone()?;
    bind_executable_gross_edge(
        &mut base,
        opp.strategy_kind,
        long_leg,
        short_leg,
        common::time::now_ms(),
    );
    let slippage = ticket_slippage(long_leg, short_leg)?;
    let mismatch_evidence = funding_window_mismatch_evidence(opp, &base, long_leg, short_leg)?;
    let mut cost = match (long_fee, short_fee) {
        (Some(long_fee), Some(short_fee)) => Some(cost_from_fee_snapshots_with_slippage(
            &base,
            long_fee,
            short_fee,
            slippage,
            mismatch_evidence.buffer_bps,
            opp.strategy_kind,
        )),
        _ => None,
    }?;
    cost.one_cycle.yield_basis = Some(mismatch_evidence.yield_basis);
    cost.one_cycle.long_next_settlement_ms = mismatch_evidence.long_next_settlement_ms;
    cost.one_cycle.short_next_settlement_ms = mismatch_evidence.short_next_settlement_ms;
    if opp.strategy_kind == Some(StrategyKind::PerpCross) {
        bind_perp_cross_projection(&mut cost, long_leg, short_leg, common::time::now_ms());
    }
    Some(cost)
}

pub(super) fn refresh_ticket_cost(
    ticket: &HedgeTicket,
    long_leg: &HedgeLegQuote,
    short_leg: &HedgeLegQuote,
    now_ms: i64,
) -> Option<ExecutionCostProfile> {
    let mut base = ticket.cost.clone()?;
    bind_executable_gross_edge(&mut base, ticket.strategy, long_leg, short_leg, now_ms);
    let long_fee = ticket_fee_snapshot(ticket, long_leg, now_ms)?;
    let short_fee = ticket_fee_snapshot(ticket, short_leg, now_ms)?;
    let slippage = ticket_slippage(long_leg, short_leg)?;
    let mismatch = ticket_mismatch_evidence(ticket, &base, long_leg, short_leg)?;
    let mut cost = cost_from_fee_snapshots_with_slippage(
        &base,
        long_fee,
        short_fee,
        slippage,
        mismatch.buffer_bps,
        ticket.strategy,
    );
    cost.one_cycle.yield_basis = Some(mismatch.yield_basis);
    cost.one_cycle.long_next_settlement_ms = mismatch.long_next_settlement_ms;
    cost.one_cycle.short_next_settlement_ms = mismatch.short_next_settlement_ms;
    if ticket.strategy == Some(StrategyKind::PerpCross) {
        bind_perp_cross_projection(&mut cost, long_leg, short_leg, now_ms);
    }
    Some(cost)
}

fn ticket_fee_snapshot<'a>(
    ticket: &'a HedgeTicket,
    leg: &HedgeLegQuote,
    now_ms: i64,
) -> Option<&'a TradeFeeSnapshot> {
    let product = p0_hedge_leg_product(ticket.strategy, ticket.spot_leg_mode, leg.role)?;
    ticket.fee_snapshots.iter().find(|snapshot| {
        snapshot.product == product
            && shared_types::venue_names_equal(&snapshot.venue, &leg.exchange)
            && snapshot.symbol.eq_ignore_ascii_case(&leg.symbol)
            && snapshot.is_fresh_verified(now_ms)
    })
}

fn ticket_mismatch_evidence(
    ticket: &HedgeTicket,
    base: &ExecutionCostProfile,
    long_leg: &HedgeLegQuote,
    short_leg: &HedgeLegQuote,
) -> Option<shared_types::fees::FundingWindowMismatchEvidence> {
    let buffer_bps = if ticket.strategy == Some(StrategyKind::PerpCross) {
        arbitrage::profit_proof::funding_window_mismatch_buffer_bps(
            arbitrage::profit_proof::FundingWindowInput {
                long_funding_bps: long_leg.funding_bps,
                short_funding_bps: short_leg.funding_bps,
                long_next_settlement_ms: long_leg.next_funding_time,
                short_next_settlement_ms: short_leg.next_funding_time,
                long_interval_hours: long_leg.funding_interval_hours,
                short_interval_hours: short_leg.funding_interval_hours,
            },
        )?
    } else {
        base.one_cycle.funding_window_mismatch_buffer_bps.max(0.0)
    };
    let evidence = shared_types::fees::FundingWindowMismatchEvidence {
        yield_basis: base
            .one_cycle
            .yield_basis
            .unwrap_or(shared_types::fees::YieldBasis::NativeSettlement),
        buffer_bps,
        long_next_settlement_ms: (long_leg.next_funding_time > 0)
            .then_some(long_leg.next_funding_time),
        short_next_settlement_ms: (short_leg.next_funding_time > 0)
            .then_some(short_leg.next_funding_time),
    };
    evidence.is_traceable().then_some(evidence)
}

pub(super) fn bind_executable_gross_edge(
    base: &mut ExecutionCostProfile,
    strategy: Option<StrategyKind>,
    long_leg: &HedgeLegQuote,
    short_leg: &HedgeLegQuote,
    observed_at_ms: i64,
) {
    if !matches!(
        strategy,
        Some(
            StrategyKind::PerpPriceSpread
                | StrategyKind::SpotPerp
                | StrategyKind::CrossSpotPerp
                | StrategyKind::SpotCross
        )
    ) {
        return;
    }
    let Some((long_price, short_price)) = long_leg
        .open_vwap_price
        .zip(short_leg.open_vwap_price)
        .filter(|(long, short)| long.is_finite() && *long > 0.0 && short.is_finite())
    else {
        return;
    };
    let executable_edge_bps = (short_price - long_price) / long_price * 10_000.0;
    let gross_edge_bps = match strategy {
        Some(StrategyKind::PerpPriceSpread) => {
            let proven_edge_bps = if base.gross_edge_bps.is_finite() {
                base.gross_edge_bps.max(0.0)
            } else {
                0.0
            };
            proven_edge_bps.min(executable_edge_bps)
        }
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp) => {
            executable_edge_bps
                + arbitrage::profit_proof::current_native_funding_bps(
                    short_leg.funding_bps,
                    short_leg.next_funding_time,
                    short_leg.funding_interval_hours,
                    observed_at_ms,
                )
                .unwrap_or_default()
        }
        _ => executable_edge_bps,
    };
    base.gross_edge_bps = gross_edge_bps;
    base.one_cycle.gross_edge_bps = gross_edge_bps;
}

#[cfg(test)]
pub(super) fn cost_from_fee_snapshots(
    base: &ExecutionCostProfile,
    long_fee: &TradeFeeSnapshot,
    short_fee: &TradeFeeSnapshot,
) -> ExecutionCostProfile {
    cost_from_fee_snapshots_with_slippage(
        base,
        long_fee,
        short_fee,
        base_slippage(base),
        base.one_cycle.funding_window_mismatch_buffer_bps,
        None,
    )
}

pub(super) fn cost_from_fee_snapshots_with_slippage(
    base: &ExecutionCostProfile,
    long_fee: &TradeFeeSnapshot,
    short_fee: &TradeFeeSnapshot,
    mut slippage: SlippageParts,
    mismatch_bps: f64,
    strategy: Option<StrategyKind>,
) -> ExecutionCostProfile {
    let cycle = strategy_execution_cycle(strategy);
    let open_fee_bps = long_fee.open_fee_bps + short_fee.open_fee_bps;
    let close_fee_bps = match cycle {
        StrategyExecutionCycle::PairedOpenClose => long_fee.close_fee_bps + short_fee.close_fee_bps,
        StrategyExecutionCycle::PairedOpenRebalance => {
            slippage.long_close_bps = 0.0;
            slippage.short_close_bps = 0.0;
            0.0
        }
    };
    cost_from_parts(
        base,
        FeeParts {
            open_bps: open_fee_bps,
            close_bps: close_fee_bps,
            financing_bps: base
                .round_trip
                .as_ref()
                .map_or(0.0, |round_trip| round_trip.borrow_or_financing_bps),
            close_trade_costs: cycle == StrategyExecutionCycle::PairedOpenClose,
        },
        slippage,
        mismatch_bps,
        Some((long_fee, short_fee)),
    )
}

mod depth;
mod perp_cross;
mod reconciliation;
#[cfg(test)]
mod tests;

#[cfg(test)]
use depth::base_slippage;
pub(super) use depth::funding_window_mismatch_evidence;
use depth::ticket_slippage;
use perp_cross::bind_perp_cross_projection;
pub(super) use reconciliation::{cost_from_parts, FeeParts, SlippageParts};

pub(super) fn breakeven_periods(gross_edge_bps: f64, total_cost_bps: f64) -> u32 {
    if gross_edge_bps <= f64::EPSILON {
        return 1_000;
    }
    (total_cost_bps / gross_edge_bps).ceil().clamp(1.0, 1_000.0) as u32
}

pub(super) fn cost_interval_hours(base: &ExecutionCostProfile) -> f64 {
    if base.recommended_hold_periods > 0 && base.recommended_hold_hours > 0.0 {
        return base.recommended_hold_hours / base.recommended_hold_periods as f64;
    }
    if base.breakeven_periods > 0 && base.breakeven_hours > 0.0 {
        return base.breakeven_hours / base.breakeven_periods as f64;
    }
    8.0
}

fn one_time_profile(base: &ExecutionCostProfile) -> bool {
    base.breakeven_periods == 1 && base.recommended_hold_periods == 1
}
