use super::*;

#[test]
fn ticket_rebinds_cost_to_one_aligned_native_event() {
    let now_ms = common::time::now_ms();
    let mut opp = opportunity_with_prices(Some(100.0), Some(100.05));
    opp.execution_cost = Some(base_cost_profile());
    let long_fee = fee_snapshot("binance", FeeProduct::Perp, 0.0, 0.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Perp, 0.0, 0.0);
    let mut long_leg = depth_cost_leg(HedgeLegRole::Long, now_ms + 3_600_000);
    long_leg.funding_bps = Some(1.0);
    long_leg.funding_interval_hours = 1;
    let mut short_leg = depth_cost_leg(HedgeLegRole::Short, now_ms + 3_600_010);
    short_leg.funding_bps = Some(14.0);
    short_leg.funding_interval_hours = 8;

    let cost = ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &long_leg,
        &short_leg,
    );
    assert!(
        cost.is_some(),
        "aligned native event should produce ticket cost"
    );
    let Some(cost) = cost else { return };

    assert_eq!(cost.breakeven_periods, 1);
    assert_eq!(cost.recommended_hold_periods, 1);
    assert!((cost.recommended_hold_hours - 1.0).abs() < 0.01);
    assert_eq!(cost.gross_edge_bps, 13.0);
    assert_eq!(cost.one_cycle.funding_window_mismatch_buffer_bps, 0.0);
    assert_eq!(cost.total_cost_bps, 5.0);
    assert_eq!(cost.one_cycle.net_bps, 8.0);
}

#[test]
fn profit_proof_uses_target_size_vwap_alignment() {
    let now_ms = common::time::now_ms();
    let mut opp = opportunity_with_prices(Some(100.0), Some(100.03));
    opp.execution_cost = Some(base_cost_profile());
    let long_fee = fee_snapshot("binance", FeeProduct::Perp, 0.0, 0.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Perp, 0.0, 0.0);
    let mut long_leg = depth_cost_leg(HedgeLegRole::Long, now_ms + 3_600_000);
    long_leg.funding_bps = Some(1.0);
    long_leg.funding_interval_hours = 1;
    long_leg.open_vwap_price = Some(100.0);
    long_leg.market_timestamp_ms = Some(now_ms);
    let mut short_leg = depth_cost_leg(HedgeLegRole::Short, now_ms + 3_600_010);
    short_leg.funding_bps = Some(14.0);
    short_leg.funding_interval_hours = 8;
    short_leg.open_vwap_price = Some(100.03);
    short_leg.market_timestamp_ms = Some(now_ms + 10);
    let cost = ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &long_leg,
        &short_leg,
    );
    assert!(
        cost.is_some(),
        "aligned native event should produce ticket cost"
    );
    let Some(cost) = cost else { return };

    let aligned = strategy_profit_proof(
        Some(StrategyKind::PerpCross),
        None,
        &long_leg,
        &short_leg,
        Some(&cost),
        now_ms,
    );
    assert!(aligned.passed);

    short_leg.open_vwap_price = Some(100.2);
    let wide = strategy_profit_proof(
        Some(StrategyKind::PerpCross),
        None,
        &long_leg,
        &short_leg,
        Some(&cost),
        now_ms,
    );
    assert!(!wide.passed);
    assert!(wide.detail.contains("应转入永续价差观察"));
}
