use super::super::super::{missing_portfolio_health, portfolio_health_facts_from_snapshot};
use shared_types::{
    AccountStateSnapshot, HardLimitsUsage, PnlBreakdown, PortfolioNavEvidence, PortfolioSnapshot,
    PortfolioSummary, RiskSnapshot, RiskStatusSlot,
};

#[test]
fn cached_portfolio_facts_reuse_risk_and_delta_without_history_query() {
    let snapshot = PortfolioSnapshot {
        summary: PortfolioSummary {
            total_nav_usd: 10_000.0,
            nav_evidence: PortfolioNavEvidence::default(),
            nav_change_24h_pct: Some(0.0),
            net_delta_usd: 125.0,
            net_delta_pct_of_nav: 1.25,
            naked_exposure_usd: 0.0,
            naked_position_count: 0,
            realized_pnl_today_usd: 0.0,
            pnl_breakdown: PnlBreakdown::default(),
            updated_at_ms: 1_000,
        },
        positions: Vec::new(),
        balances: Vec::new(),
        risk: RiskSnapshot {
            var_99_1d_usd: 50.0,
            var_pct_of_nav: 0.5,
            var_sample_size: 10,
            funding_clustering: Vec::new(),
            delta_concentration: Vec::new(),
            margin_utilization: Vec::new(),
            hard_limits: HardLimitsUsage::default(),
            updated_at_ms: 1_000,
        },
        server_now_ms: 1_000,
        snapshot_version: "portfolio-1".to_owned(),
        degraded: false,
        problems: Vec::new(),
        operation_health: Vec::new(),
        account_state: AccountStateSnapshot::default(),
        recent_close_runs: Vec::new(),
    };

    let facts = portfolio_health_facts_from_snapshot(&snapshot, 1_001);

    assert_eq!(facts.risk, RiskStatusSlot::Ok);
    assert_eq!(facts.net_delta_usd, 125.0);
    assert_eq!(facts.net_delta_pct_of_nav, 1.25);
    assert!(facts.next_funding.is_none());
    assert!(facts.problems.is_empty());
}

#[test]
fn missing_portfolio_snapshot_blocks_risk_without_query_fallback() {
    let facts = missing_portfolio_health(2_000);

    assert!(facts.account_degraded);
    assert_eq!(facts.risk, RiskStatusSlot::Block);
    assert!(facts.next_funding.is_none());
    assert_eq!(facts.problems.len(), 1);
    assert_eq!(
        facts.problems[0].code,
        shared_types::problem::codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE
    );
}
