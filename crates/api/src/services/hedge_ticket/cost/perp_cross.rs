use super::*;

pub(super) fn bind_perp_cross_projection(
    cost: &mut ExecutionCostProfile,
    long_leg: &HedgeLegQuote,
    short_leg: &HedgeLegQuote,
    observed_at_ms: i64,
) {
    let funding = arbitrage::profit_proof::FundingWindowInput {
        long_funding_bps: long_leg.funding_bps,
        short_funding_bps: short_leg.funding_bps,
        long_next_settlement_ms: long_leg.next_funding_time,
        short_next_settlement_ms: short_leg.next_funding_time,
        long_interval_hours: long_leg.funding_interval_hours,
        short_interval_hours: short_leg.funding_interval_hours,
    };
    let fixed_cost_bps = cost.one_cycle.open_fee_bps
        + cost.one_cycle.close_fee_bps
        + cost.one_cycle.open_slippage_bps
        + cost.one_cycle.close_slippage_bps;
    let target_buffer_bps = cost.one_cycle.target_buffer_bps;
    let Some(event) = arbitrage::profit_proof::first_profitable_funding_projection(funding, 0.0)
    else {
        bind_no_positive_event(cost, fixed_cost_bps);
        return;
    };

    let total_cost_bps = fixed_cost_bps + event.mismatch_reserve_bps;
    let net_bps = event.gross_funding_bps - total_cost_bps;
    let breakeven = net_bps > f64::EPSILON;
    let recommended = net_bps > target_buffer_bps + f64::EPSILON;
    let event_hold_hours = hold_hours(observed_at_ms, event.settlement_at_ms);
    cost.gross_edge_bps = event.gross_funding_bps;
    cost.total_cost_bps = total_cost_bps;
    cost.one_cycle.gross_edge_bps = event.gross_funding_bps;
    cost.one_cycle.funding_window_mismatch_buffer_bps = event.mismatch_reserve_bps;
    cost.one_cycle.yield_basis = Some(shared_types::fees::YieldBasis::NativeSettlement);
    cost.one_cycle.long_next_settlement_ms = Some(long_leg.next_funding_time);
    cost.one_cycle.short_next_settlement_ms = Some(short_leg.next_funding_time);
    cost.one_cycle.net_bps = net_bps;
    cost.one_cycle.covers_round_trip_cost = recommended;
    cost.breakeven_periods = u32::from(breakeven);
    cost.breakeven_hours = if breakeven { event_hold_hours } else { 0.0 };
    cost.recommended_hold_periods = u32::from(recommended);
    cost.recommended_hold_hours = if recommended { event_hold_hours } else { 0.0 };
    cost.net_bps_at_recommended_hold = net_bps;
    if let Some(round_trip) = cost.round_trip.as_mut() {
        round_trip.funding_window_mismatch_buffer_bps = event.mismatch_reserve_bps;
        round_trip.total_cost_bps = total_cost_bps;
        round_trip.one_cycle_net_bps = net_bps;
    }
}

fn bind_no_positive_event(cost: &mut ExecutionCostProfile, fixed_cost_bps: f64) {
    cost.gross_edge_bps = 0.0;
    cost.total_cost_bps = fixed_cost_bps;
    cost.one_cycle.gross_edge_bps = 0.0;
    cost.one_cycle.net_bps = -fixed_cost_bps;
    cost.one_cycle.covers_round_trip_cost = false;
    cost.breakeven_periods = 0;
    cost.breakeven_hours = 0.0;
    cost.recommended_hold_periods = 0;
    cost.recommended_hold_hours = 0.0;
    cost.net_bps_at_recommended_hold = -fixed_cost_bps;
    if let Some(round_trip) = cost.round_trip.as_mut() {
        round_trip.funding_window_mismatch_buffer_bps = 0.0;
        round_trip.total_cost_bps = fixed_cost_bps;
        round_trip.one_cycle_net_bps = -fixed_cost_bps;
    }
}

fn hold_hours(observed_at_ms: i64, settlement_at_ms: i64) -> f64 {
    settlement_at_ms.saturating_sub(observed_at_ms).max(0) as f64 / 3_600_000.0
}
