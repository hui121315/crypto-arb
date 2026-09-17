pub(super) fn verified_cost() -> shared_types::ExecutionCostProfile {
    shared_types::ExecutionCostProfile {
        gross_edge_bps: 30.0,
        fee_bps: 20.0,
        wear_bps: 2.0,
        total_cost_bps: 22.0,
        one_cycle: shared_types::OneCycleCostProfile {
            gross_edge_bps: 30.0,
            open_fee_bps: 5.0,
            close_fee_bps: 5.0,
            open_slippage_bps: 1.0,
            close_slippage_bps: 1.0,
            funding_window_mismatch_buffer_bps: 0.0,
            yield_basis: Some(shared_types::YieldBasis::NativeSettlement),
            long_next_settlement_ms: Some(1_000),
            short_next_settlement_ms: Some(1_000),
            target_buffer_bps: 0.0,
            net_bps: 8.0,
            covers_round_trip_cost: true,
        },
        breakeven_periods: 1,
        breakeven_hours: 8.0,
        recommended_hold_periods: 2,
        recommended_hold_hours: 16.0,
        net_bps_at_recommended_hold: 38.0,
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
            one_cycle_net_bps: 8.0,
            profitability_evidence: profitability_evidence(),
        }),
    }
}

fn profitability_evidence() -> shared_types::ProfitabilityEvidence {
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
