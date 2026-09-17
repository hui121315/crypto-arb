use super::*;

#[test]
fn spot_perp_ticket_repricing_keeps_one_current_native_funding_event() {
    let mut base = base_cost_profile();
    let mut long = depth_cost_leg(HedgeLegRole::Long, 0);
    let mut short = depth_cost_leg(HedgeLegRole::Short, common::time::now_ms() + 3_600_000);
    long.open_vwap_price = Some(100.0);
    short.open_vwap_price = Some(101.0);
    short.funding_bps = Some(4.0);
    short.funding_interval_hours = 1;

    bind_executable_gross_edge(
        &mut base,
        Some(StrategyKind::SpotPerp),
        &long,
        &short,
        common::time::now_ms(),
    );

    assert!((base.gross_edge_bps - 104.0).abs() < 1e-9);

    short.next_funding_time = common::time::now_ms();
    bind_executable_gross_edge(
        &mut base,
        Some(StrategyKind::SpotPerp),
        &long,
        &short,
        common::time::now_ms(),
    );
    assert!((base.gross_edge_bps - 100.0).abs() < 1e-9);
}

#[test]
fn perp_price_spread_reprice_cannot_assume_full_future_convergence() {
    let mut historical = base_cost_profile();
    historical.gross_edge_bps = 40.0;
    historical.one_cycle.gross_edge_bps = 40.0;
    let mut long = depth_cost_leg(HedgeLegRole::Long, 0);
    long.open_vwap_price = Some(100.0);
    let mut short = depth_cost_leg(HedgeLegRole::Short, 0);
    short.open_vwap_price = Some(101.0);

    bind_executable_gross_edge(
        &mut historical,
        Some(StrategyKind::PerpPriceSpread),
        &long,
        &short,
        1,
    );

    assert_eq!(historical.gross_edge_bps, 40.0);
    assert_eq!(historical.one_cycle.gross_edge_bps, 40.0);

    let mut depth_limited = historical;
    short.open_vwap_price = Some(100.2);
    bind_executable_gross_edge(
        &mut depth_limited,
        Some(StrategyKind::PerpPriceSpread),
        &long,
        &short,
        1,
    );

    assert!((depth_limited.gross_edge_bps - 20.0).abs() < 1e-9);
    assert!((depth_limited.one_cycle.gross_edge_bps - 20.0).abs() < 1e-9);
}

#[test]
fn spot_cross_ticket_cost_uses_two_trades_before_transfer_rebalance() -> Result<(), &'static str> {
    let long_fee = fee_snapshot("binance", FeeProduct::Spot, 3.0, 4.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Spot, 2.0, 3.0);
    let cost = cost_from_fee_snapshots_with_slippage(
        &base_cost_profile(),
        &long_fee,
        &short_fee,
        SlippageParts {
            long_open_bps: 1.0,
            short_open_bps: 2.0,
            long_close_bps: 3.0,
            short_close_bps: 4.0,
        },
        0.0,
        Some(StrategyKind::SpotCross),
    );

    assert_eq!(cost.fee_bps, 5.0);
    assert_eq!(cost.one_cycle.close_fee_bps, 0.0);
    assert_eq!(cost.one_cycle.close_slippage_bps, 0.0);
    assert_eq!(cost.total_cost_bps, 8.0);
    let round_trip = cost.round_trip.as_ref().ok_or("cost evidence")?;
    assert_eq!(round_trip.long_leg.close_fee_bps, 0.0);
    assert_eq!(round_trip.short_leg.close_fee_bps, 0.0);
    Ok(())
}
