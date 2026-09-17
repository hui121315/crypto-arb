use super::*;
use shared_types::StrategyKind;

mod filters;
mod response;

fn test_opp() -> ArbitrageOpportunityDto {
    ArbitrageOpportunityDto {
        id: "test".into(),
        symbol: "BTC".into(),
        arb_type: shared_types::ArbitrageType::CrossExchange,
        type_label: "永续跨所".into(),
        long_exchange: "a".into(),
        short_exchange: "b".into(),
        spread_8h: 0.0,
        long_rate_8h: 0.0,
        short_rate_8h: 0.0,
        long_rate: 0.0,
        short_rate: 0.0,
        single_yield: 0.0,
        net_single_yield: 0.0,
        raw_single_yield: 0.0,
        settlement_interval: 8,
        risk_adjusted_yield: 0.0,
        trading_cost_rate: 0.0,
        min_holding_periods: 1,
        risk_level: shared_types::RiskLevel::Low,
        volatility: 0.0,
        sharpe_ratio: 0.0,
        score: 0.0,
        score_breakdown: None,
        ranking_key: None,
        recommendation: shared_types::Recommendation::Hold,
        optimal_position: 0.0,
        max_position: 0.0,
        liquidity_score: 0.0,
        volume_24h: 0.0,
        long_volume_24h: 0.0,
        short_volume_24h: 0.0,
        data_source: "test".into(),
        confidence: 0.0,
        updated_at: chrono::Utc::now(),
        long_funding_interval: 8,
        short_funding_interval: 8,
        settlement_time_diff: false,
        strategy_description: String::new(),
        long_action: String::new(),
        short_action: String::new(),
        long_next_funding_time: 0,
        short_next_funding_time: 0,
        time_to_settlement_ms: 0,
        is_snipe_ready: false,
        long_price: None,
        short_price: None,
        long_leg_market_evidence: None,
        short_leg_market_evidence: None,
        quote_conversions: Vec::new(),
        price_deviation: None,
        basis_spread: None,
        basis_annual_cost: None,
        risk_warnings: Vec::new(),
        execution_eligible: true,
        execution_blockers: Vec::new(),
        execution_cost: None,
        index_composition: None,
        strategy_kind: Some(StrategyKind::PerpCross),
        strategy_category: Some(shared_types::StrategyCategory::Futures),
        spot_leg_mode: None,
        basis_bps: None,
        annualized_funding_bps: None,
        triangular_path: None,
        onchain_metadata: None,
        predicted_next_funding: None,
        funding_diff_window: None,
        funding_diff_windows: Vec::new(),
        borrow_cost_bps_per_day: None,
        funding_window_alignment_minutes: None,
        funding_cap_distance_bps: None,
        min_hold_hours: None,
        settlement_countdown_seconds: None,
    }
}

fn test_opp_with_kind(kind: StrategyKind) -> ArbitrageOpportunityDto {
    let mut opp = ArbitrageOpportunityDto {
        strategy_kind: Some(kind),
        strategy_category: Some(kind.category()),
        ..test_opp()
    };
    make_ready(&mut opp);
    opp
}

fn make_ready(opp: &mut ArbitrageOpportunityDto) {
    opp.long_price = Some(100.0);
    opp.short_price = Some(100.1);
    opp.long_leg_market_evidence = Some(market_evidence("a"));
    opp.short_leg_market_evidence = Some(market_evidence("b"));
    opp.execution_cost = Some(verified_cost());
}

fn market_evidence(venue: &str) -> shared_types::OpportunityLegMarketEvidence {
    let observed_at_ms = chrono::Utc::now().timestamp_millis();
    shared_types::OpportunityLegMarketEvidence {
        venue: venue.into(),
        symbol: "BTCUSDT".into(),
        price: Some(100.0),
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Fresh,
            source: shared_types::MarketDataSourceKind::WsPush,
            freshness_ms: Some(20),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms,
            coverage: Some(shared_types::MarketDataCoverage::new(1, 1)),
            problem: None,
        },
    }
}

fn verified_cost() -> shared_types::ExecutionCostProfile {
    shared_types::ExecutionCostProfile {
        gross_edge_bps: 20.0,
        fee_bps: 10.0,
        wear_bps: 2.0,
        total_cost_bps: 12.0,
        one_cycle: shared_types::OneCycleCostProfile {
            gross_edge_bps: 50.0,
            open_fee_bps: 10.0,
            close_fee_bps: 10.0,
            open_slippage_bps: 1.0,
            close_slippage_bps: 1.0,
            funding_window_mismatch_buffer_bps: 0.0,
            yield_basis: None,
            long_next_settlement_ms: None,
            short_next_settlement_ms: None,
            target_buffer_bps: 0.0,
            net_bps: 28.0,
            covers_round_trip_cost: true,
        },
        breakeven_periods: 1,
        breakeven_hours: 8.0,
        recommended_hold_periods: 2,
        recommended_hold_hours: 16.0,
        net_bps_at_recommended_hold: 28.0,
        round_trip: Some(shared_types::RoundTripCostBreakdown {
            long_leg: leg_cost(shared_types::HedgeLegRole::Long, "a"),
            short_leg: leg_cost(shared_types::HedgeLegRole::Short, "b"),
            open_fee_bps: 10.0,
            close_fee_bps: 10.0,
            open_slippage_bps: 1.0,
            close_slippage_bps: 1.0,
            borrow_or_financing_bps: 0.0,
            funding_window_mismatch_buffer_bps: 0.0,
            min_profit_buffer_bps: 0.0,
            total_cost_bps: 22.0,
            one_cycle_net_bps: -2.0,
            profitability_evidence: test_profitability_evidence(),
        }),
    }
}

fn test_profitability_evidence() -> shared_types::ProfitabilityEvidence {
    shared_types::ProfitabilityEvidence {
        status: shared_types::ProfitabilityEvidenceStatus::Partial,
        source: "test:fee_schedule".into(),
        observed_at_ms: chrono::Utc::now().timestamp_millis(),
        verified_fee_snapshot_count: 2,
        fee_sources: vec![
            shared_types::TradeFeeSource::OfficialSchedule,
            shared_types::TradeFeeSource::OfficialSchedule,
        ],
        fee_evidence_ids: vec!["fee:a:perp:vip0".into(), "fee:b:perp:vip0".into()],
        funding_history: None,
        problem: None,
    }
}

fn leg_cost(role: shared_types::HedgeLegRole, venue: &str) -> shared_types::LegCostBreakdown {
    shared_types::LegCostBreakdown {
        role,
        venue: venue.into(),
        symbol: "BTCUSDT".into(),
        product: shared_types::FeeProduct::Perp,
        open_fee_bps: 5.0,
        close_fee_bps: 5.0,
        open_slippage_bps: 0.5,
        close_slippage_bps: 0.5,
        fee_snapshot: Some(fee_snapshot(venue)),
    }
}

fn fee_snapshot(venue: &str) -> shared_types::TradeFeeSnapshot {
    // Anchor freshness to the current clock so the fixture never expires as
    // wall-clock time advances past a hardcoded validity window.
    let now_ms = chrono::Utc::now().timestamp_millis();
    shared_types::TradeFeeSnapshot {
        venue: venue.into(),
        symbol: "BTCUSDT".into(),
        product: shared_types::FeeProduct::Perp,
        account_id: None,
        maker_fee_bps: 2.0,
        taker_fee_bps: 5.0,
        open_fee_bps: 5.0,
        close_fee_bps: 5.0,
        source: shared_types::TradeFeeSource::OfficialSchedule,
        fetched_at_ms: now_ms - 60_000,
        valid_until_ms: now_ms + 86_400_000,
        freshness_ms: Some(1_000),
        evidence: Some(shared_types::TradeFeeEvidence {
            evidence_id: format!("fee:{venue}:perp:vip0"),
            source_name: format!("{venue} unit test fee fixture"),
            source_url: "https://example.com/fee-fixture".into(),
            checked_at_ms: now_ms - 60_000,
            effective_at_ms: None,
            schedule_version: Some("2026-05-31".into()),
            tier: Some("VIP0".into()),
            scope: Some("perp taker".into()),
            problem: None,
        }),
        verification_problem: None,
        note: None,
    }
}

fn funding(symbol: &str, exchange: &str) -> shared_types::FundingRateData {
    shared_types::FundingRateData {
        symbol: symbol.to_owned(),
        exchange: exchange.to_owned(),
        rate: 0.0001,
        rate_8h: 0.0001,
        predicted_rate: None,
        next_funding_time: 0,
        funding_interval: 8,
        volume_24h: 1_000_000.0,
        timestamp: common::time::now_ms(),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    }
}
