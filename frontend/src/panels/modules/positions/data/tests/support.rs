//! positions data 层测试夹具（跨 behavior 子模块共享）。

use shared_types::{
    CloseRun, CloseRunStatus, PortfolioSnapshot, PortfolioSnapshotEnvelope, PositionRow,
    PositionSeverity, PositionSide,
};

pub(super) fn close_run(
    status: CloseRunStatus,
    submitted_order_count: usize,
    failed_leg_count: usize,
    naked_exposure_usd: f64,
) -> CloseRun {
    CloseRun {
        id: "close-test".to_owned(),
        scope: shared_types::CloseRunScope::Pair,
        status,
        action_run_id: Some("act-1".to_owned()),
        request_id: Some("req-1".to_owned()),
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 2,
        reason: Some("positions.close_pair".to_owned()),
        legs: Vec::new(),
        submitted_order_count,
        failed_leg_count,
        naked_exposure_usd,
        message: "partial close".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: None,
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 2,
    }
}

pub(super) fn close_run_with_id(id: &str, status: CloseRunStatus, updated_at_ms: i64) -> CloseRun {
    let mut run = close_run(status, 1, 0, 0.0);
    run.id = id.to_owned();
    run.updated_at_ms = updated_at_ms;
    run
}

pub(super) fn portfolio_snapshot(snapshot_version: &str) -> PortfolioSnapshot {
    PortfolioSnapshot {
        summary: shared_types::PortfolioSummary {
            total_nav_usd: 0.0,
            nav_evidence: shared_types::PortfolioNavEvidence::default(),
            nav_change_24h_pct: Some(0.0),
            net_delta_usd: 0.0,
            net_delta_pct_of_nav: 0.0,
            naked_exposure_usd: 0.0,
            naked_position_count: 0,
            realized_pnl_today_usd: 0.0,
            pnl_breakdown: shared_types::PnlBreakdown::default(),
            updated_at_ms: 1,
        },
        positions: Vec::new(),
        balances: Vec::new(),
        risk: shared_types::RiskSnapshot {
            var_99_1d_usd: 0.0,
            var_pct_of_nav: 0.0,
            var_sample_size: 0,
            funding_clustering: Vec::new(),
            delta_concentration: Vec::new(),
            margin_utilization: Vec::new(),
            hard_limits: shared_types::HardLimitsUsage::default(),
            updated_at_ms: 1,
        },
        server_now_ms: 1,
        snapshot_version: snapshot_version.to_owned(),
        degraded: false,
        problems: Vec::new(),
        operation_health: Vec::new(),
        account_state: shared_types::AccountStateSnapshot::default(),
        recent_close_runs: Vec::new(),
    }
}

pub(super) fn portfolio_envelope(snapshot: PortfolioSnapshot) -> PortfolioSnapshotEnvelope {
    PortfolioSnapshotEnvelope {
        status: shared_types::PortfolioSnapshotStatus::Fresh,
        source: "portfolio_lifecycle".to_owned(),
        observed_at_ms: 1,
        snapshot: Some(snapshot),
        problem: None,
        problems: Vec::new(),
        operation_health: Vec::new(),
        retry_after_ms: None,
    }
}

pub(super) fn row(
    venue: &str,
    symbol: &str,
    side: PositionSide,
    paired_with: Option<&str>,
) -> PositionRow {
    PositionRow {
        venue: venue.to_string(),
        symbol: symbol.to_string(),
        origin: Default::default(),
        side,
        quantity: 1.0,
        entry_price: 1.0,
        mark_price: 1.0,
        leverage: 1.0,
        unrealized_pnl_usd: 0.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: None,
        funding_rate_8h: 0.0,
        funding_rate_verified: true,
        maintenance_margin_ratio: 0.0,
        pair_evidence: None,
        paired_with: paired_with.map(str::to_string),
        margin_usd: 1.0,
        severity: PositionSeverity::Ok,
        seconds_until_funding: None,
    }
}

pub(super) fn paired_row(
    venue: &str,
    symbol: &str,
    side: PositionSide,
    paired_with: Option<&str>,
) -> PositionRow {
    let mut row = row(venue, symbol, side, paired_with);
    row.origin = shared_types::PositionOrigin::ExecutionLedger;
    row.pair_evidence = paired_with.map(|pair| shared_types::PositionPairEvidence {
        source: shared_types::PositionPairEvidenceSource::ExecutionRun,
        run_id: "run-1".to_owned(),
        ticket_id: "ticket-1".to_owned(),
        opportunity_id: "opp-1".to_owned(),
        venue: venue.to_string(),
        symbol: symbol.to_string(),
        side,
        partner_venue: pair
            .split_once('@')
            .map(|(venue, _)| venue)
            .unwrap_or(pair)
            .to_owned(),
        partner_symbol: pair
            .split_once('@')
            .map(|(_, symbol)| symbol)
            .unwrap_or(symbol)
            .to_owned(),
        partner_side: match side {
            PositionSide::Long => PositionSide::Short,
            PositionSide::Short => PositionSide::Long,
        },
        leg_filled_quantity: 1.0,
        partner_filled_quantity: 1.0,
        matched_notional_usd: 1.0,
        updated_at_ms: 2,
    });
    row
}
