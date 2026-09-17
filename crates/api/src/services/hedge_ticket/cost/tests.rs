use super::*;

#[test]
fn refreshed_fees_do_not_multiply_a_convergence_edge_by_holding_periods() {
    let base = ExecutionCostProfile {
        gross_edge_bps: 60.0,
        fee_bps: 20.0,
        wear_bps: 8.0,
        total_cost_bps: 28.0,
        one_cycle: OneCycleCostProfile::default(),
        breakeven_periods: 1,
        breakeven_hours: 0.1,
        recommended_hold_periods: 1,
        recommended_hold_hours: 0.1,
        net_bps_at_recommended_hold: 32.0,
        round_trip: None,
    };

    let refreshed = cost_from_parts(
        &base,
        FeeParts {
            open_bps: 10.0,
            close_bps: 10.0,
            financing_bps: 0.0,
            close_trade_costs: true,
        },
        SlippageParts {
            long_open_bps: 2.0,
            short_open_bps: 2.0,
            long_close_bps: 2.0,
            short_close_bps: 2.0,
        },
        0.0,
        None,
    );

    assert_eq!(refreshed.recommended_hold_periods, 1);
    assert!((refreshed.recommended_hold_hours - 0.1).abs() < 1e-12);
    assert!((refreshed.net_bps_at_recommended_hold - 32.0).abs() < 1e-12);
}
