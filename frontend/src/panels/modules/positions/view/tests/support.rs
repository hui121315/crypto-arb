use shared_types::{
    AccountStateSnapshot, ApiProblem, HardLimitsUsage, ListStatus, PnlBreakdown, PortfolioSnapshot,
    PortfolioSummary, PositionInfo, PositionRow, PositionSeverity, PositionSide, RiskSnapshot,
    VenueBalanceEnvelope, VenueBalanceInfo, VenuePositionEnvelope,
};

pub(super) fn snapshot_with_balance_status(
    status: ListStatus,
    problems: Vec<ApiProblem>,
) -> PortfolioSnapshot {
    let account_state = AccountStateSnapshot {
        balances: VenueBalanceEnvelope::new(
            vec![VenueBalanceInfo {
                venue: "okx".into(),
                currency: "USDT".into(),
                total: 100.0,
                available: 90.0,
                frozen: 10.0,
                unrealized_pnl: 0.0,
            }],
            status,
            "account_balance_runtime",
            42,
            problems,
            Vec::new(),
        ),
        ..Default::default()
    };
    portfolio_snapshot(account_state, Vec::new())
}

pub(super) fn snapshot_with_position_status(
    status: ListStatus,
    problems: Vec<ApiProblem>,
) -> PortfolioSnapshot {
    let account_state = AccountStateSnapshot {
        positions: VenuePositionEnvelope::new(
            vec![PositionInfo {
                symbol: "BTC".into(),
                exchange: "Gate".into(),
                side: "long".into(),
                quantity: 1.0,
                entry_price: 100.0,
                mark_price: 101.0,
                unrealized_pnl: 1.0,
                leverage: 5.0,
                liquidation_price: Some(80.0),
                liquidation_distance_pct: Some(20.79),
                next_funding_ms: None,
                paired_with: None,
                margin: 20.0,
                maintenance_margin_ratio: 0.005,
                position_mode: Some("single".into()),
                margin_mode: Some("isolated".into()),
                risk_rate: None,
                available_position: None,
                frozen_position: None,
            }],
            status,
            "account_position_runtime",
            42,
            problems,
            Vec::new(),
        ),
        ..Default::default()
    };
    let rows = vec![PositionRow {
        venue: "Gate".into(),
        symbol: "BTC".into(),
        origin: Default::default(),
        side: PositionSide::Long,
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 101.0,
        leverage: 5.0,
        unrealized_pnl_usd: 1.0,
        liquidation_price: Some(80.0),
        liquidation_distance_pct: Some(20.79),
        next_funding_ms: None,
        funding_rate_8h: 0.0,
        funding_rate_verified: false,
        maintenance_margin_ratio: 0.005,
        pair_evidence: None,
        paired_with: None,
        margin_usd: 20.0,
        severity: PositionSeverity::Ok,
        seconds_until_funding: None,
    }];
    portfolio_snapshot(account_state, rows)
}

pub(super) fn portfolio_snapshot(
    account_state: AccountStateSnapshot,
    positions: Vec<PositionRow>,
) -> PortfolioSnapshot {
    PortfolioSnapshot {
        summary: PortfolioSummary {
            total_nav_usd: 100.0,
            nav_evidence: shared_types::PortfolioNavEvidence::default(),
            nav_change_24h_pct: Some(0.0),
            net_delta_usd: 0.0,
            net_delta_pct_of_nav: 0.0,
            naked_exposure_usd: 0.0,
            naked_position_count: 0,
            realized_pnl_today_usd: 0.0,
            pnl_breakdown: PnlBreakdown::default(),
            updated_at_ms: 42,
        },
        positions,
        balances: Vec::new(),
        risk: RiskSnapshot {
            var_99_1d_usd: 0.0,
            var_pct_of_nav: 0.0,
            var_sample_size: 0,
            funding_clustering: Vec::new(),
            delta_concentration: Vec::new(),
            margin_utilization: Vec::new(),
            hard_limits: HardLimitsUsage::default(),
            updated_at_ms: 42,
        },
        server_now_ms: 42,
        snapshot_version: "positions-test".into(),
        degraded: false,
        problems: Vec::new(),
        operation_health: Vec::new(),
        account_state,
        recent_close_runs: Vec::new(),
    }
}
