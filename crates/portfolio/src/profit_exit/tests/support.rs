use super::super::*;
use shared_types::{
    AccountDataHealth, AccountFieldQuality, AccountFieldQualityStatus, AccountFieldSubject,
    AccountStateSnapshot, AutoProfitCloseConfig, ExecutionCostReconciliation, ExecutionRunEvidence,
    ExecutionRunLeg, ExecutionRunState, HedgeLegRole, ListStatus, LiveOrderState, PnlBreakdown,
    PortfolioNavEvidence, PortfolioSummary, PositionPairEvidence, PositionPairEvidenceSource,
    RiskSnapshot,
};

pub(super) fn config() -> AutoProfitCloseConfig {
    AutoProfitCloseConfig {
        enabled: true,
        min_net_profit_usd: 5.0,
        min_roi_bps: 10.0,
        exit_buffer_bps: 5.0,
        confirmation_samples: 3,
        cooldown_secs: 60,
        ..Default::default()
    }
}

pub(super) fn with_liquidation_distances(
    mut snapshot: PortfolioSnapshot,
    first_distance_pct: f64,
    second_distance_pct: f64,
    actual_evidence: bool,
) -> PortfolioSnapshot {
    snapshot.positions[0].liquidation_distance_pct = Some(first_distance_pct);
    snapshot.positions[1].liquidation_distance_pct = Some(second_distance_pct);
    if actual_evidence {
        snapshot = with_actual_liquidation_evidence(snapshot, &[0, 1]);
    }
    snapshot
}

pub(super) fn with_actual_liquidation_evidence(
    mut snapshot: PortfolioSnapshot,
    position_indexes: &[usize],
) -> PortfolioSnapshot {
    let quality = position_indexes
        .iter()
        .filter_map(|index| snapshot.positions.get(*index))
        .map(|row| {
            AccountFieldQuality::new(
                AccountFieldSubject::position(
                    row.venue.clone(),
                    row.symbol.clone(),
                    side_key(row.side),
                ),
                "liquidationDistancePct",
                AccountFieldQualityStatus::Actual,
                "exchange_position_liquidation_distance",
                Some(snapshot.server_now_ms),
            )
        })
        .collect();
    snapshot.account_state.field_quality = quality;
    snapshot
}

pub(super) fn with_position_health(
    mut snapshot: PortfolioSnapshot,
    last_success_ms: i64,
) -> PortfolioSnapshot {
    snapshot.account_state.status = ListStatus::Degraded;
    snapshot.account_state.positions.status = ListStatus::Degraded;
    snapshot.account_state.positions.row_health = snapshot
        .positions
        .iter()
        .map(|row| {
            let mut health = AccountDataHealth::new(
                AccountFieldSubject::position(
                    row.venue.clone(),
                    row.symbol.clone(),
                    side_key(row.side),
                ),
                "profit_exit_test",
                snapshot.server_now_ms,
            );
            health.last_success_ms = Some(last_success_ms);
            health
        })
        .collect();
    snapshot
}

fn side_key(side: PositionSide) -> &'static str {
    match side {
        PositionSide::Long => "long",
        PositionSide::Short => "short",
    }
}

pub(super) fn snapshot(now_ms: i64, long_pnl: f64, short_pnl: f64) -> PortfolioSnapshot {
    PortfolioSnapshot {
        summary: PortfolioSummary {
            total_nav_usd: 1_000.0,
            nav_evidence: PortfolioNavEvidence::default(),
            nav_change_24h_pct: Some(0.0),
            net_delta_usd: 0.0,
            net_delta_pct_of_nav: 0.0,
            naked_exposure_usd: 0.0,
            naked_position_count: 0,
            realized_pnl_today_usd: 0.0,
            pnl_breakdown: PnlBreakdown::default(),
            updated_at_ms: now_ms,
        },
        positions: vec![
            position(
                "okx",
                PositionSide::Long,
                "binance",
                PositionSide::Short,
                long_pnl,
            ),
            position(
                "binance",
                PositionSide::Short,
                "okx",
                PositionSide::Long,
                short_pnl,
            ),
        ],
        balances: Vec::new(),
        risk: RiskSnapshot {
            var_99_1d_usd: 0.0,
            var_pct_of_nav: 0.0,
            var_sample_size: 0,
            funding_clustering: Vec::new(),
            delta_concentration: Vec::new(),
            margin_utilization: Vec::new(),
            hard_limits: Default::default(),
            updated_at_ms: now_ms,
        },
        server_now_ms: now_ms,
        snapshot_version: "pos-2-test".to_owned(),
        degraded: false,
        problems: Vec::new(),
        operation_health: Vec::new(),
        account_state: fresh_account_state(now_ms),
        recent_close_runs: Vec::new(),
    }
}

fn fresh_account_state(now_ms: i64) -> AccountStateSnapshot {
    AccountStateSnapshot {
        positions: shared_types::VenuePositionEnvelope::new(
            Vec::new(),
            ListStatus::Fresh,
            "profit_exit_test",
            now_ms,
            Vec::new(),
            Vec::new(),
        ),
        status: ListStatus::Fresh,
        source: "profit_exit_test".to_owned(),
        observed_at_ms: now_ms,
        ..Default::default()
    }
}

fn position(
    venue: &str,
    side: PositionSide,
    partner_venue: &str,
    partner_side: PositionSide,
    pnl: f64,
) -> PositionRow {
    PositionRow {
        venue: venue.to_owned(),
        symbol: "BTCUSDT".to_owned(),
        origin: Default::default(),
        side,
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 100.0,
        leverage: 2.0,
        unrealized_pnl_usd: pnl,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        funding_rate_8h: 0.0,
        funding_rate_verified: true,
        maintenance_margin_ratio: 0.05,
        pair_evidence: Some(PositionPairEvidence {
            source: PositionPairEvidenceSource::ExecutionRun,
            run_id: "run-1".to_owned(),
            ticket_id: "ticket-1".to_owned(),
            opportunity_id: "opp-1".to_owned(),
            venue: venue.to_owned(),
            symbol: "BTCUSDT".to_owned(),
            side,
            partner_venue: partner_venue.to_owned(),
            partner_symbol: "BTCUSDT".to_owned(),
            partner_side,
            leg_filled_quantity: 1.0,
            partner_filled_quantity: 1.0,
            matched_notional_usd: 200.0,
            updated_at_ms: 10,
        }),
        paired_with: Some(format!("{partner_venue}@BTCUSDT")),
        margin_usd: 50.0,
        severity: Default::default(),
        seconds_until_funding: None,
    }
}

pub(super) fn run(funding_pnl_usd: f64) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".to_owned(),
        ticket_id: "ticket-1".to_owned(),
        opportunity_id: "opp-1".to_owned(),
        state: ExecutionRunState::Hedged,
        long_leg: execution_leg(HedgeLegRole::Long, "okx"),
        short_leg: execution_leg(HedgeLegRole::Short, "binance"),
        net_exposure_usd: 0.0,
        cost_reconciliation: Some(ExecutionCostReconciliation {
            estimated_open_cost_usd: 1.5,
            estimated_close_cost_usd: 2.0,
            estimated_slippage_usd: 1.0,
            estimated_total_cost_usd: 4.5,
            filled_fee_usd: Some(1.0),
            actual_slippage_usd: Some(0.0),
            actual_open_cost_usd: Some(1.0),
            actual_funding_usd: Some(funding_pnl_usd),
            funding_event_ids: vec!["funding-1".to_owned()],
            actual_unwind_fee_usd: None,
            actual_unwind_slippage_usd: None,
            actual_unwind_cost_usd: None,
            unwind_event_ids: Vec::new(),
            missing_fields: Vec::new(),
            actual_cost_usd: None,
            cost_delta_usd: None,
        }),
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(10),
        evidence: ExecutionRunEvidence::default(),
        recovery_action: None,
        status_reason: "hedged".to_owned(),
        created_at_ms: 1,
        updated_at_ms: 10,
    }
}

fn execution_leg(role: HedgeLegRole, exchange: &str) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: exchange.to_owned(),
        symbol: "BTCUSDT".to_owned(),
        order_ids: vec![format!("{exchange}-order")],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: Some(10),
        state: LiveOrderState::Filled,
        target_quantity: 1.0,
        filled_quantity: Some(1.0),
        target_notional_usd: 100.0,
        filled_notional_usd: Some(100.0),
        filled_fee: Some(0.5),
    }
}
