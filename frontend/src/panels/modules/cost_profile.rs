use shared_types::ExecutionCostProfile;

pub(crate) fn one_cycle_net_bps(cost: &ExecutionCostProfile) -> f64 {
    if has_explicit_one_cycle(cost) {
        cost.one_cycle.net_bps
    } else {
        cost.gross_edge_bps - cost.total_cost_bps
    }
}

pub(crate) fn one_cycle_covers_cost(cost: &ExecutionCostProfile) -> bool {
    if has_explicit_one_cycle(cost) {
        cost.one_cycle.covers_round_trip_cost
    } else {
        one_cycle_net_bps(cost) > 0.0
    }
}

pub(crate) fn fee_evidence_count(cost: &ExecutionCostProfile) -> usize {
    cost.round_trip.as_ref().map_or(0, |round_trip| {
        round_trip
            .profitability_evidence
            .verified_fee_snapshot_count
    })
}

fn has_explicit_one_cycle(cost: &ExecutionCostProfile) -> bool {
    let one_cycle = &cost.one_cycle;
    [
        one_cycle.gross_edge_bps,
        one_cycle.open_fee_bps,
        one_cycle.close_fee_bps,
        one_cycle.open_slippage_bps,
        one_cycle.close_slippage_bps,
        one_cycle.funding_window_mismatch_buffer_bps,
        one_cycle.target_buffer_bps,
        one_cycle.net_bps,
    ]
    .into_iter()
    .any(|value| value.abs() > f64::EPSILON)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::OneCycleCostProfile;

    #[test]
    fn derives_one_cycle_net_when_backend_snapshot_lacks_one_cycle() {
        let cost = cost_profile(132.12, 22.5, OneCycleCostProfile::default());

        assert!((one_cycle_net_bps(&cost) - 109.62).abs() < 1e-9);
        assert!(one_cycle_covers_cost(&cost));
    }

    #[test]
    fn prefers_explicit_one_cycle_when_present() {
        let cost = cost_profile(
            132.12,
            22.5,
            OneCycleCostProfile {
                gross_edge_bps: 132.12,
                net_bps: -3.0,
                covers_round_trip_cost: false,
                ..OneCycleCostProfile::default()
            },
        );

        assert_eq!(one_cycle_net_bps(&cost), -3.0);
        assert!(!one_cycle_covers_cost(&cost));
    }

    #[test]
    fn missing_round_trip_has_zero_fee_evidence() {
        assert_eq!(
            fee_evidence_count(&cost_profile(132.12, 22.5, OneCycleCostProfile::default())),
            0
        );
    }

    fn cost_profile(
        gross_edge_bps: f64,
        total_cost_bps: f64,
        one_cycle: OneCycleCostProfile,
    ) -> ExecutionCostProfile {
        ExecutionCostProfile {
            gross_edge_bps,
            fee_bps: 0.0,
            wear_bps: 0.0,
            total_cost_bps,
            one_cycle,
            breakeven_periods: 1,
            breakeven_hours: 8.0,
            recommended_hold_periods: 2,
            recommended_hold_hours: 16.0,
            net_bps_at_recommended_hold: 0.0,
            round_trip: None,
        }
    }
}
