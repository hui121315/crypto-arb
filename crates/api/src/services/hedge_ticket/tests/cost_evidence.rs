use super::*;
use shared_types::fees::YieldBasis;
use shared_types::{
    MarketDataHealth, MarketDataQuality, MarketDataSourceKind, OpportunityLegMarketEvidence,
};

mod perp_cross;
mod strategy_cycles;

#[test]
fn ticket_cost_requires_both_fee_snapshots() -> Result<(), &'static str> {
    let mut opp = opportunity_with_prices(Some(100.0), Some(101.0));
    opp.execution_cost = Some(base_cost_profile());
    let long_fee = fee_snapshot("binance", FeeProduct::Perp, 3.0, 4.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Perp, 2.0, 3.0);
    let long_leg = depth_cost_leg(HedgeLegRole::Long, 1_000);
    let short_leg = depth_cost_leg(HedgeLegRole::Short, 1_010);

    assert!(ticket_cost(&opp, Some(&long_fee), None, &long_leg, &short_leg).is_none());
    assert!(ticket_cost(&opp, None, Some(&short_fee), &long_leg, &short_leg).is_none());
    assert!(ticket_cost(&opp, None, None, &long_leg, &short_leg).is_none());
    let cost = ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &long_leg,
        &short_leg,
    );
    let cost = cost.ok_or("verified depth, fee, and settlement evidence should produce cost")?;
    let basis = cost
        .one_cycle
        .yield_basis
        .ok_or("yield basis should be present")?;
    assert_eq!(basis, YieldBasis::NativeSettlement);
    assert_eq!(cost.one_cycle.long_next_settlement_ms, Some(1_000));
    assert_eq!(cost.one_cycle.short_next_settlement_ms, Some(1_010));
    let serialized = serde_json::to_value(&cost).unwrap_or_default();
    assert_eq!(serialized["oneCycle"]["yieldBasis"], "native_settlement");
    assert_eq!(serialized["oneCycle"]["longNextSettlementMs"], 1_000);
    assert_eq!(serialized["oneCycle"]["shortNextSettlementMs"], 1_010);
    assert_eq!(cost.one_cycle.funding_window_mismatch_buffer_bps, 0.0);
    assert_eq!(cost.total_cost_bps, 17.0);
    assert_eq!(cost.breakeven_periods, 0);
    assert_eq!(cost.recommended_hold_periods, 0);
    assert_eq!(
        (long_leg.next_funding_time, short_leg.next_funding_time),
        (1_000, 1_010)
    );
    Ok(())
}

#[test]
fn ticket_cost_rejects_missing_or_stale_depth_evidence() {
    let mut opp = opportunity_with_prices(Some(100.0), Some(101.0));
    opp.execution_cost = Some(base_cost_profile());
    let long_fee = fee_snapshot("binance", FeeProduct::Perp, 3.0, 4.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Perp, 2.0, 3.0);
    let mut long_leg = depth_cost_leg(HedgeLegRole::Long, 1_000);
    let short_leg = depth_cost_leg(HedgeLegRole::Short, 2_000);

    long_leg.depth_usd_20bps = None;
    assert!(ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &long_leg,
        &short_leg
    )
    .is_none());

    let mut stale_long = depth_cost_leg(HedgeLegRole::Long, 1_000);
    assert!(stale_long.depth_health.is_some());
    if let Some(health) = stale_long.depth_health.as_mut() {
        health.quality = MarketDataQuality::StaleAllowed;
    }
    assert!(ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &stale_long,
        &short_leg
    )
    .is_none());
}

#[test]
fn ticket_cost_rejects_non_monotonic_or_unpriced_depth_evidence() {
    let mut opp = opportunity_with_prices(Some(100.0), Some(101.0));
    opp.execution_cost = Some(base_cost_profile());
    let long_fee = fee_snapshot("binance", FeeProduct::Perp, 3.0, 4.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Perp, 2.0, 3.0);
    let mut long_leg = depth_cost_leg(HedgeLegRole::Long, 1_000);
    let short_leg = depth_cost_leg(HedgeLegRole::Short, 2_000);

    long_leg.depth_usd_5bps = Some(250.0);
    long_leg.depth_usd_10bps = Some(200.0);
    assert!(ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &long_leg,
        &short_leg
    )
    .is_none());

    long_leg.depth_usd_5bps = Some(100.0);
    long_leg.depth_usd_10bps = Some(200.0);
    long_leg.open_vwap_price = None;
    assert!(ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &long_leg,
        &short_leg
    )
    .is_none());
}

#[test]
fn ticket_cost_requires_aligned_native_settlement_evidence() {
    let base = base_cost_profile();
    let mut opp = opportunity_with_prices(Some(100.0), Some(101.0));
    opp.execution_cost = Some(base.clone());
    let long_fee = fee_snapshot("binance", FeeProduct::Perp, 3.0, 4.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Perp, 2.0, 3.0);
    let mut long_leg = depth_cost_leg(HedgeLegRole::Long, 1_000);
    let mut short_leg = depth_cost_leg(HedgeLegRole::Short, 0);
    long_leg.funding_bps = Some(1.25);
    short_leg.funding_bps = Some(4.0);

    assert!(ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &long_leg,
        &short_leg
    )
    .is_none());

    short_leg.next_funding_time = 1_010;
    let evidence = funding_window_mismatch_evidence(&opp, &base, &long_leg, &short_leg);
    assert!(evidence.is_some());
    assert_eq!(
        evidence.as_ref().map(|row| row.yield_basis),
        Some(YieldBasis::NativeSettlement)
    );
    assert_eq!(evidence.as_ref().map(|row| row.buffer_bps), Some(0.0));
    let cost = ticket_cost(
        &opp,
        Some(&long_fee),
        Some(&short_fee),
        &long_leg,
        &short_leg,
    );
    assert!(
        cost.is_some(),
        "aligned native settlement evidence should produce cost"
    );
    let Some(cost) = cost else { return };
    assert_eq!(
        cost.one_cycle.yield_basis,
        Some(YieldBasis::NativeSettlement)
    );
    assert_eq!(cost.one_cycle.long_next_settlement_ms, Some(1_000));
    assert_eq!(cost.one_cycle.short_next_settlement_ms, Some(1_010));
    assert_eq!(cost.one_cycle.funding_window_mismatch_buffer_bps, 0.0);
}

#[test]
fn fee_snapshots_recompute_round_trip_cost() -> Result<(), &'static str> {
    let base = base_cost_profile();
    let long_fee = fee_snapshot("binance", FeeProduct::Perp, 3.0, 4.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Perp, 2.0, 3.0);

    let cost = cost_from_fee_snapshots(&base, &long_fee, &short_fee);

    assert_eq!(cost.fee_bps, 12.0);
    assert_eq!(cost.total_cost_bps, 20.0);
    assert_eq!(cost.one_cycle.open_fee_bps, 5.0);
    assert_eq!(cost.one_cycle.close_fee_bps, 7.0);
    assert_eq!(cost.one_cycle.net_bps, 10.0);
    let round_trip = cost.round_trip.as_ref().ok_or("round trip cost missing")?;
    assert!(round_trip.profitability_evidence.is_cost_verified());
    assert_eq!(
        round_trip.profitability_evidence.source,
        shared_types::PROFITABILITY_EVIDENCE_SOURCE
    );
    assert_eq!(
        round_trip
            .profitability_evidence
            .verified_fee_snapshot_count,
        2
    );
    assert_eq!(
        round_trip.profitability_evidence.fee_sources,
        [TradeFeeSource::AccountApi, TradeFeeSource::AccountApi]
    );
    Ok(())
}

#[test]
fn refreshed_market_cost_preserves_transfer_or_financing_cost() {
    let long_fee = fee_snapshot("binance", FeeProduct::Spot, 3.0, 3.0);
    let short_fee = fee_snapshot("okx", FeeProduct::Spot, 3.0, 3.0);
    let mut base = cost_from_fee_snapshots(&base_cost_profile(), &long_fee, &short_fee);
    if let Some(round_trip) = base.round_trip.as_mut() {
        round_trip.borrow_or_financing_bps = 9.0;
    }

    let refreshed = cost_from_fee_snapshots(&base, &long_fee, &short_fee);

    assert_eq!(refreshed.total_cost_bps, 29.0);
    assert_eq!(
        refreshed
            .round_trip
            .as_ref()
            .map(|round_trip| round_trip.borrow_or_financing_bps),
        Some(9.0)
    );
}

#[test]
fn ticket_cost_can_use_leg_level_vwap_slippage() {
    let base = base_cost_profile();

    let cost = cost_from_parts(
        &base,
        FeeParts {
            open_bps: 5.0,
            close_bps: 7.0,
            financing_bps: 0.0,
            close_trade_costs: true,
        },
        SlippageParts {
            long_open_bps: 1.0,
            short_open_bps: 2.0,
            long_close_bps: 3.0,
            short_close_bps: 4.0,
        },
        0.0,
        None,
    );

    assert_eq!(cost.one_cycle.open_slippage_bps, 3.0);
    assert_eq!(cost.one_cycle.close_slippage_bps, 7.0);
    assert_eq!(cost.total_cost_bps, 22.0);
    assert_eq!(cost.one_cycle.net_bps, 8.0);
}

fn depth_cost_leg(role: HedgeLegRole, next_funding_time: i64) -> HedgeLegQuote {
    HedgeLegQuote {
        role,
        exchange: "binance".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        reference_price: Some(100.0),
        bid: Some(99.0),
        ask: Some(101.0),
        mid: Some(100.0),
        open_vwap_price: Some(101.0),
        open_slippage_bps: Some(1.0),
        close_vwap_price: Some(99.0),
        close_slippage_bps: Some(1.5),
        depth_usd_5bps: Some(100.0),
        depth_usd_10bps: Some(200.0),
        depth_usd_20bps: Some(300.0),
        max_notional_usd: Some(100.0),
        market_evidence: Some(OpportunityLegMarketEvidence {
            venue: "binance".into(),
            symbol: "BTC".into(),
            price: Some(100.0),
            health: fresh_market_health(),
        }),
        depth_health: Some(fresh_market_health()),
        depth_reason: None,
        funding_bps: Some(1.0),
        next_funding_time,
        funding_interval_hours: 8,
        market_timestamp_ms: Some(1),
        blockers: Vec::new(),
    }
}

fn fresh_market_health() -> MarketDataHealth {
    MarketDataHealth {
        quality: MarketDataQuality::Fresh,
        source: MarketDataSourceKind::WsPush,
        freshness_ms: Some(0),
        retry_after_ms: None,
        last_error: None,
        observed_at_ms: 1,
        coverage: None,
        problem: None,
    }
}
