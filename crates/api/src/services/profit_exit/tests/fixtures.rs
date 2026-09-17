use super::super::*;

pub(super) fn test_execution_run(created_at_ms: i64) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-convergence".into(),
        ticket_id: "ticket-convergence".into(),
        opportunity_id: "opp-convergence".into(),
        state: shared_types::ExecutionRunState::Hedged,
        long_leg: test_execution_leg(shared_types::HedgeLegRole::Long),
        short_leg: test_execution_leg(shared_types::HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "hedged".into(),
        created_at_ms,
        updated_at_ms: created_at_ms,
    }
}

fn test_execution_leg(role: shared_types::HedgeLegRole) -> shared_types::ExecutionRunLeg {
    shared_types::ExecutionRunLeg {
        role,
        exchange: "paper".into(),
        symbol: "BTC".into(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: Some(1_000),
        state: shared_types::LiveOrderState::Filled,
        target_quantity: 1.0,
        filled_quantity: Some(1.0),
        target_notional_usd: 10.0,
        filled_notional_usd: Some(10.0),
        filled_fee: Some(0.01),
    }
}

pub(super) fn test_ticket(
    strategy: StrategyKind,
    recommended_hold_periods: u32,
) -> serde_json::Result<HedgeTicket> {
    serde_json::from_value(serde_json::json!({
        "ticketId": "ticket-convergence",
        "opportunityId": "opp-convergence",
        "strategy": strategy,
        "symbol": "BTC",
        "createdAtMs": 1,
        "expiresAtMs": 100000,
        "longLeg": test_leg_json("long"),
        "shortLeg": test_leg_json("short"),
        "cost": {
            "grossEdgeBps": 60.0,
            "feeBps": 20.0,
            "wearBps": 8.0,
            "totalCostBps": 28.0,
            "breakevenPeriods": 1,
            "breakevenHours": 0.0166666667,
            "recommendedHoldPeriods": recommended_hold_periods,
            "recommendedHoldHours": 0.0166666667,
            "netBpsAtRecommendedHold": 32.0
        },
        "sizing": {
            "requestedCapitalUsd": 10.0,
            "leverage": 1.0,
            "targetNotionalUsd": 10.0
        },
        "guards": [],
        "blockers": []
    }))
}

fn test_leg_json(role: &str) -> serde_json::Value {
    serde_json::json!({
        "role": role,
        "exchange": "paper",
        "symbol": "BTC",
        "side": if role == "long" { "buy" } else { "sell" },
        "referencePrice": 100.0,
        "bid": 99.9,
        "ask": 100.1,
        "mid": 100.0,
        "openVwapPrice": 100.1,
        "openSlippageBps": 1.0,
        "closeVwapPrice": 99.9,
        "closeSlippageBps": 1.0,
        "depthUsd5bps": 1000.0,
        "depthUsd10bps": 2000.0,
        "depthUsd20bps": 3000.0,
        "maxNotionalUsd": 1000.0,
        "fundingBps": 0.0,
        "nextFundingTime": 0,
        "marketTimestampMs": 1,
        "blockers": []
    })
}

pub(super) fn test_stop_candidate(gross_unrealized_pnl_usd: f64) -> ProfitExitCandidate {
    ProfitExitCandidate {
        run_id: "run-convergence".into(),
        venue: "paper".into(),
        symbol: "BTC".into(),
        side: shared_types::PositionSide::Long,
        snapshot_version: "snapshot".into(),
        observed_at_ms: 2_000,
        valuation: Some(portfolio::ProfitExitValuation {
            matched_notional_usd: 10.0,
            gross_unrealized_pnl_usd,
            open_fee_usd: 0.01,
            funding_pnl_usd: 0.0,
            estimated_exit_cost_usd: 0.01,
            safety_buffer_usd: 0.0,
            estimated_net_profit_usd: gross_unrealized_pnl_usd - 0.02,
            estimated_roi_bps: -30.0,
        }),
        trigger: ProfitExitTrigger::StopLoss,
        minimum_liquidation_distance_pct: None,
        risk_venue: None,
    }
}

pub(super) fn test_close_run(status: CloseRunStatus, action_run_id: Option<String>) -> CloseRun {
    CloseRun {
        id: "close-run-convergence".into(),
        scope: shared_types::CloseRunScope::Pair,
        status,
        action_run_id,
        request_id: None,
        idempotency_key: Some("auto-exit-test".into()),
        snapshot_version: "snapshot".into(),
        expected_leg_count: 2,
        reason: Some("test".into()),
        legs: vec![shared_types::CloseLeg {
            venue: "bitget".into(),
            symbol: "BTC".into(),
            side: shared_types::PositionSide::Long,
            status: shared_types::CloseLegStatus::Failed,
            quantity: 0.1,
            mark_price: 100.0,
            notional_usd: 10.0,
            order: None,
            finality_source: None,
            confirmed_filled_at_ms: None,
            problem: None,
            pair_evidence: Some(shared_types::PositionPairEvidence {
                source: shared_types::PositionPairEvidenceSource::ExecutionRun,
                run_id: "run-convergence".into(),
                ticket_id: "ticket-convergence".into(),
                opportunity_id: "opp-convergence".into(),
                venue: "bitget".into(),
                symbol: "BTC".into(),
                side: shared_types::PositionSide::Long,
                partner_venue: "hyperliquid".into(),
                partner_symbol: "BTC".into(),
                partner_side: shared_types::PositionSide::Short,
                leg_filled_quantity: 0.1,
                partner_filled_quantity: 0.1,
                matched_notional_usd: 10.0,
                updated_at_ms: 2_000,
            }),
            cost_events: Vec::new(),
        }],
        submitted_order_count: 0,
        failed_leg_count: 1,
        naked_exposure_usd: 10.0,
        message: "test".into(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 2_000,
        updated_at_ms: 2_000,
    }
}
