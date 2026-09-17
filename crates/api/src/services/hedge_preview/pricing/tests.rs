use super::*;
use shared_types::{HedgeLegQuote, HedgeLegRole, HedgeSizing, OrderSide};

#[test]
fn spot_cross_maps_to_the_p0_strategy() {
    assert_eq!(
        p0_strategy_from_arb_type(ArbitrageType::SpotCross),
        Some(StrategyKind::SpotCross)
    );
}

#[test]
fn perp_price_spread_uses_funding_rates_and_evidence_backed_gross_edge() -> serde_json::Result<()> {
    let mut opportunity: ArbitrageOpportunityDto = serde_json::from_value(serde_json::json!({
        "id": "price-spread",
        "symbol": "BTC",
        "type": "cross_exchange",
        "typeLabel": "永续价差",
        "longExchange": "binance",
        "shortExchange": "okx",
        "spread8h": 0.02,
        "longRate8h": 0.0002,
        "shortRate8h": 0.0005,
        "longRate": 0.0002,
        "shortRate": 0.0005,
        "singleYield": 0.02,
        "netSingleYield": 0.0012,
        "rawSingleYield": 0.02,
        "settlementInterval": 8,
        "riskAdjustedYield": 0.0,
        "tradingCostRate": 0.0028,
        "minHoldingPeriods": 1,
        "riskLevel": "low",
        "volatility": 0.0,
        "sharpeRatio": 0.0,
        "score": 50.0,
        "recommendation": "hold",
        "optimalPosition": 10.0,
        "maxPosition": 10.0,
        "liquidityScore": 100.0,
        "volume24h": 1000000.0,
        "dataSource": "test",
        "confidence": 0.8,
        "updatedAt": "2026-08-01T00:00:00Z",
        "longFundingInterval": 8,
        "shortFundingInterval": 8,
        "strategyKind": "perp_price_spread"
    }))?;
    opportunity.strategy_kind = Some(StrategyKind::PerpPriceSpread);
    opportunity.execution_cost = Some(shared_types::ExecutionCostProfile {
        gross_edge_bps: 40.0,
        fee_bps: 20.0,
        wear_bps: 8.0,
        total_cost_bps: 28.0,
        one_cycle: shared_types::OneCycleCostProfile::default(),
        breakeven_periods: 1,
        breakeven_hours: 0.1,
        recommended_hold_periods: 1,
        recommended_hold_hours: 0.1,
        net_bps_at_recommended_hold: 12.0,
        round_trip: None,
    });

    let ticket = ticket_for(
        StrategyKind::PerpPriceSpread,
        opportunity.execution_cost,
        2.0,
        5.0,
    );
    assert_eq!(preview_funding_yield(&ticket), 0.0);
    assert!((gross_edge_usd(&ticket, 10.0) - 0.04).abs() < 1e-12);
    Ok(())
}

#[test]
fn perp_cross_preview_uses_one_native_joint_settlement() -> serde_json::Result<()> {
    let mut opportunity: ArbitrageOpportunityDto = serde_json::from_value(serde_json::json!({
        "id": "native-funding",
        "symbol": "BTC",
        "type": "cross_exchange",
        "typeLabel": "永续跨所",
        "longExchange": "binance",
        "shortExchange": "okx",
        "spread8h": 0.0008,
        "longRate8h": 0.0001,
        "shortRate8h": 0.0009,
        "longRate": 0.0000125,
        "shortRate": 0.0001125,
        "singleYield": 0.0001,
        "netSingleYield": 0.00005,
        "rawSingleYield": 0.0001,
        "settlementInterval": 1,
        "riskAdjustedYield": 0.0,
        "tradingCostRate": 0.00005,
        "minHoldingPeriods": 1,
        "riskLevel": "low",
        "volatility": 0.0,
        "sharpeRatio": 0.0,
        "score": 50.0,
        "recommendation": "hold",
        "optimalPosition": 10.0,
        "maxPosition": 10.0,
        "liquidityScore": 100.0,
        "volume24h": 1000000.0,
        "dataSource": "test",
        "confidence": 0.8,
        "updatedAt": "2026-08-01T00:00:00Z",
        "longFundingInterval": 1,
        "shortFundingInterval": 1,
        "strategyKind": "perp_cross"
    }))?;
    opportunity.execution_cost = Some(shared_types::ExecutionCostProfile {
        gross_edge_bps: 1.0,
        fee_bps: 0.2,
        wear_bps: 0.2,
        total_cost_bps: 0.4,
        one_cycle: shared_types::OneCycleCostProfile::default(),
        breakeven_periods: 1,
        breakeven_hours: 1.0,
        recommended_hold_periods: 2,
        recommended_hold_hours: 2.0,
        net_bps_at_recommended_hold: 1.6,
        round_trip: None,
    });

    let ticket = ticket_for(
        StrategyKind::PerpCross,
        opportunity.execution_cost,
        0.125,
        1.125,
    );
    assert!((gross_edge_usd(&ticket, 100.0) - 0.01).abs() < 1e-12);
    assert!((preview_funding_yield(&ticket) - 0.0001).abs() < 1e-12);
    Ok(())
}

fn ticket_for(
    strategy: StrategyKind,
    cost: Option<shared_types::ExecutionCostProfile>,
    long_funding_bps: f64,
    short_funding_bps: f64,
) -> HedgeTicket {
    HedgeTicket {
        ticket_id: "ticket-test".into(),
        opportunity_id: "opportunity-test".into(),
        strategy: Some(strategy),
        spot_leg_mode: None,
        symbol: "BTC".into(),
        created_at_ms: 1,
        market_checked_at_ms: 1,
        expires_at_ms: 2,
        long_leg: leg(HedgeLegRole::Long, OrderSide::Buy, long_funding_bps),
        short_leg: leg(HedgeLegRole::Short, OrderSide::Sell, short_funding_bps),
        cost,
        fee_snapshots: Vec::new(),
        sizing: HedgeSizing::default(),
        guards: Vec::new(),
        blockers: Vec::new(),
    }
}

fn leg(role: HedgeLegRole, side: OrderSide, funding_bps: f64) -> HedgeLegQuote {
    HedgeLegQuote {
        role,
        exchange: "test".into(),
        symbol: "BTCUSDT".into(),
        side,
        reference_price: Some(100.0),
        bid: Some(99.9),
        ask: Some(100.1),
        mid: Some(100.0),
        open_vwap_price: Some(100.0),
        open_slippage_bps: Some(0.0),
        close_vwap_price: Some(100.0),
        close_slippage_bps: Some(0.0),
        depth_usd_5bps: Some(1_000.0),
        depth_usd_10bps: Some(1_000.0),
        depth_usd_20bps: Some(1_000.0),
        max_notional_usd: Some(1_000.0),
        market_evidence: None,
        depth_health: None,
        depth_reason: None,
        funding_bps: Some(funding_bps),
        next_funding_time: 1,
        funding_interval_hours: 8,
        market_timestamp_ms: Some(1),
        blockers: Vec::new(),
    }
}
