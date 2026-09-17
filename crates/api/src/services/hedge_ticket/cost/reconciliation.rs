use super::*;

#[derive(Clone, Copy)]
pub(in crate::services::hedge_ticket) struct FeeParts {
    pub(in crate::services::hedge_ticket) open_bps: f64,
    pub(in crate::services::hedge_ticket) close_bps: f64,
    pub(in crate::services::hedge_ticket) financing_bps: f64,
    pub(in crate::services::hedge_ticket) close_trade_costs: bool,
}

#[derive(Clone, Copy)]
pub(in crate::services::hedge_ticket) struct SlippageParts {
    pub(in crate::services::hedge_ticket) long_open_bps: f64,
    pub(in crate::services::hedge_ticket) short_open_bps: f64,
    pub(in crate::services::hedge_ticket) long_close_bps: f64,
    pub(in crate::services::hedge_ticket) short_close_bps: f64,
}

impl SlippageParts {
    fn open_bps(self) -> f64 {
        self.long_open_bps + self.short_open_bps
    }

    fn close_bps(self) -> f64 {
        self.long_close_bps + self.short_close_bps
    }
}

pub(in crate::services::hedge_ticket) fn cost_from_parts(
    base: &ExecutionCostProfile,
    fee: FeeParts,
    slippage: SlippageParts,
    mismatch_bps: f64,
    fee_snapshots: Option<(&TradeFeeSnapshot, &TradeFeeSnapshot)>,
) -> ExecutionCostProfile {
    let open_fee_bps = fee.open_bps.max(0.0);
    let close_fee_bps = fee.close_bps.max(0.0);
    let open_slippage_bps = slippage.open_bps().max(0.0);
    let close_slippage_bps = slippage.close_bps().max(0.0);
    let financing_bps = fee.financing_bps.max(0.0);
    let mismatch_bps = mismatch_bps.max(0.0);
    let buffer_bps = base.one_cycle.target_buffer_bps.max(0.0);
    let total_cost_bps = open_fee_bps
        + close_fee_bps
        + open_slippage_bps
        + close_slippage_bps
        + financing_bps
        + mismatch_bps;
    let one_cycle = OneCycleCostProfile {
        gross_edge_bps: base.gross_edge_bps,
        open_fee_bps,
        close_fee_bps,
        open_slippage_bps,
        close_slippage_bps,
        funding_window_mismatch_buffer_bps: mismatch_bps,
        yield_basis: None,
        long_next_settlement_ms: None,
        short_next_settlement_ms: None,
        target_buffer_bps: buffer_bps,
        net_bps: base.gross_edge_bps - total_cost_bps,
        covers_round_trip_cost: base.gross_edge_bps - total_cost_bps > buffer_bps,
    };
    let one_time = one_time_profile(base);
    let breakeven_periods = if one_time {
        1
    } else {
        breakeven_periods(base.gross_edge_bps, total_cost_bps)
    };
    let recommended_hold_periods = if one_time {
        1
    } else {
        breakeven_periods.saturating_add(1)
    };
    let interval_hours = cost_interval_hours(base);
    ExecutionCostProfile {
        gross_edge_bps: base.gross_edge_bps,
        fee_bps: open_fee_bps + close_fee_bps,
        wear_bps: open_slippage_bps + close_slippage_bps + financing_bps,
        total_cost_bps,
        one_cycle: one_cycle.clone(),
        breakeven_periods,
        breakeven_hours: breakeven_periods as f64 * interval_hours,
        recommended_hold_periods,
        recommended_hold_hours: recommended_hold_periods as f64 * interval_hours,
        net_bps_at_recommended_hold: if one_time {
            base.gross_edge_bps - total_cost_bps
        } else {
            base.gross_edge_bps * recommended_hold_periods as f64 - total_cost_bps
        },
        round_trip: fee_snapshots.map(|(long_fee, short_fee)| {
            round_trip_breakdown(
                long_fee,
                short_fee,
                slippage,
                &one_cycle,
                fee.close_trade_costs,
                RoundTripTotals {
                    financing_bps,
                    total_cost_bps,
                    profitability_evidence: ticket_profitability_evidence(
                        base, long_fee, short_fee,
                    ),
                },
            )
        }),
    }
}

fn ticket_profitability_evidence(
    base: &ExecutionCostProfile,
    long_fee: &TradeFeeSnapshot,
    short_fee: &TradeFeeSnapshot,
) -> shared_types::ProfitabilityEvidence {
    shared_types::ProfitabilityEvidence::from_fee_snapshots(
        shared_types::PROFITABILITY_EVIDENCE_SOURCE,
        common::time::now_ms(),
        &[long_fee, short_fee],
        base.round_trip
            .as_ref()
            .and_then(|round_trip| round_trip.profitability_evidence.funding_history.clone()),
    )
}

struct RoundTripTotals {
    financing_bps: f64,
    total_cost_bps: f64,
    profitability_evidence: shared_types::ProfitabilityEvidence,
}

fn round_trip_breakdown(
    long_fee: &TradeFeeSnapshot,
    short_fee: &TradeFeeSnapshot,
    slippage: SlippageParts,
    one_cycle: &OneCycleCostProfile,
    close_trade_costs: bool,
    totals: RoundTripTotals,
) -> RoundTripCostBreakdown {
    RoundTripCostBreakdown {
        long_leg: leg_cost_breakdown(
            HedgeLegRole::Long,
            long_fee,
            slippage.long_open_bps,
            slippage.long_close_bps,
            close_trade_costs,
        ),
        short_leg: leg_cost_breakdown(
            HedgeLegRole::Short,
            short_fee,
            slippage.short_open_bps,
            slippage.short_close_bps,
            close_trade_costs,
        ),
        open_fee_bps: one_cycle.open_fee_bps,
        close_fee_bps: one_cycle.close_fee_bps,
        open_slippage_bps: one_cycle.open_slippage_bps,
        close_slippage_bps: one_cycle.close_slippage_bps,
        borrow_or_financing_bps: totals.financing_bps,
        funding_window_mismatch_buffer_bps: one_cycle.funding_window_mismatch_buffer_bps,
        min_profit_buffer_bps: one_cycle.target_buffer_bps,
        total_cost_bps: totals.total_cost_bps,
        one_cycle_net_bps: one_cycle.net_bps,
        profitability_evidence: totals.profitability_evidence,
    }
}

fn leg_cost_breakdown(
    role: HedgeLegRole,
    fee: &TradeFeeSnapshot,
    open_slippage_bps: f64,
    close_slippage_bps: f64,
    close_trade_costs: bool,
) -> LegCostBreakdown {
    LegCostBreakdown {
        role,
        venue: fee.venue.clone(),
        symbol: fee.symbol.clone(),
        product: fee.product,
        open_fee_bps: fee.open_fee_bps,
        close_fee_bps: if close_trade_costs {
            fee.close_fee_bps
        } else {
            0.0
        },
        open_slippage_bps,
        close_slippage_bps,
        fee_snapshot: Some(fee.clone()),
    }
}
