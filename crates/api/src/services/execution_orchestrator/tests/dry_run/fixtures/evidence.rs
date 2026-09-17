use shared_types::{
    ExecutionCostProfile, FeeProduct, MarketDataHealth, MarketDataQuality, MarketDataSourceKind,
    OneCycleCostProfile, TradeFeeSnapshot, TradeFeeSource,
};

pub(super) fn fee_snapshot(symbol: &str, now_ms: i64) -> TradeFeeSnapshot {
    TradeFeeSnapshot {
        venue: "mock".to_owned(),
        symbol: symbol.to_owned(),
        product: FeeProduct::Spot,
        account_id: None,
        maker_fee_bps: 1.0,
        taker_fee_bps: 1.0,
        open_fee_bps: 1.0,
        close_fee_bps: 1.0,
        source: TradeFeeSource::AccountApi,
        fetched_at_ms: now_ms,
        valid_until_ms: now_ms.saturating_add(60_000),
        freshness_ms: Some(0),
        evidence: None,
        verification_problem: None,
        note: None,
    }
}

pub(super) fn cost_profile() -> ExecutionCostProfile {
    ExecutionCostProfile {
        gross_edge_bps: 100.0,
        fee_bps: 4.0,
        wear_bps: 0.0,
        total_cost_bps: 4.0,
        one_cycle: OneCycleCostProfile {
            gross_edge_bps: 100.0,
            open_fee_bps: 2.0,
            close_fee_bps: 2.0,
            open_slippage_bps: 0.0,
            close_slippage_bps: 0.0,
            funding_window_mismatch_buffer_bps: 0.0,
            yield_basis: Some(shared_types::fees::YieldBasis::NativeSettlement),
            long_next_settlement_ms: None,
            short_next_settlement_ms: None,
            target_buffer_bps: 0.0,
            net_bps: 96.0,
            covers_round_trip_cost: true,
        },
        breakeven_periods: 1,
        breakeven_hours: 0.0,
        recommended_hold_periods: 1,
        recommended_hold_hours: 0.0,
        net_bps_at_recommended_hold: 96.0,
        round_trip: None,
    }
}

pub(super) fn fresh_health(now_ms: i64) -> MarketDataHealth {
    MarketDataHealth {
        quality: MarketDataQuality::Fresh,
        source: MarketDataSourceKind::WsPush,
        freshness_ms: Some(0),
        retry_after_ms: None,
        last_error: None,
        observed_at_ms: now_ms,
        coverage: None,
        problem: None,
    }
}
