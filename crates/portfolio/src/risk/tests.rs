use super::*;

#[test]
fn computes_var_delta_and_margin() {
    let rows = vec![
        row("OKX", PositionSide::Long),
        row("HL", PositionSide::Short),
    ];
    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[-300.0, -100.0, 50.0],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage {
            open_orders_used: 2,
            open_orders_max: 10,
            max_order_notional_usd: 1000.0,
            ..HardLimitsUsage::default()
        },
        now_ms: 0,
    });

    assert_eq!(risk.var_99_1d_usd, 300.0);
    assert_eq!(risk.delta_concentration[0].net_qty, 0.0);
    assert_eq!(risk.margin_utilization.len(), 2);
    assert_eq!(risk.hard_limits.max_symbol_notional_usd, 200.0);
}

#[test]
fn var_percentage_uses_account_nav_instead_of_position_margin() {
    let rows = vec![row("OKX", PositionSide::Long)];
    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[-10.0],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.var_99_1d_usd, 10.0);
    assert_eq!(risk.var_pct_of_nav, 1.0);
}

#[test]
fn positive_pnl_history_has_zero_downside_var() {
    let risk = compute_risk(RiskInputs {
        positions: &[],
        account_summaries: &[],
        historical_pnl_usd: &[10.0, 20.0, 30.0],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.var_99_1d_usd, 0.0);
}

#[test]
fn var_reports_sample_size_and_flags_insufficient_history() {
    let risk = compute_risk(RiskInputs {
        positions: &[],
        account_summaries: &[],
        historical_pnl_usd: &[-300.0, -100.0, 50.0],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.var_sample_size, 3);
    assert!(!risk.var_sample_sufficient());
}

#[test]
fn var_marks_sample_sufficient_with_enough_history() {
    let history: Vec<f64> = (0..shared_types::VAR_99_MIN_SAMPLES)
        .map(|i| -(i as f64))
        .collect();

    let risk = compute_risk(RiskInputs {
        positions: &[],
        account_summaries: &[],
        historical_pnl_usd: &history,
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.var_sample_size, shared_types::VAR_99_MIN_SAMPLES);
    assert!(risk.var_sample_sufficient());
}

#[test]
fn ignores_expired_funding_timestamps() {
    let mut rows = vec![row("OKX", PositionSide::Long)];
    rows[0].next_funding_ms = Some(1_000);

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 60_000,
    });

    assert!(risk
        .funding_clustering
        .iter()
        .all(|cluster| cluster.position_count == 0));
}

#[test]
fn funding_cluster_uses_row_funding_rate() {
    let mut rows = vec![row("OKX", PositionSide::Long)];
    rows[0].funding_rate_8h = 0.002;

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.funding_clustering[0].total_outflow_usd, 0.2);
}

#[test]
fn funding_cluster_excludes_long_receiving_leg() {
    let mut rows = vec![row("OKX", PositionSide::Long)];
    rows[0].funding_rate_8h = -0.002;

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.funding_clustering[0].position_count, 1);
    assert_eq!(risk.funding_clustering[0].total_outflow_usd, 0.0);
}

#[test]
fn funding_cluster_excludes_short_receiving_leg() {
    let mut rows = vec![row("OKX", PositionSide::Short)];
    rows[0].funding_rate_8h = 0.002;

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.funding_clustering[0].position_count, 1);
    assert_eq!(risk.funding_clustering[0].total_outflow_usd, 0.0);
}

#[test]
fn funding_cluster_counts_short_paying_leg() {
    let mut rows = vec![row("OKX", PositionSide::Short)];
    rows[0].funding_rate_8h = -0.002;

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.funding_clustering[0].total_outflow_usd, 0.2);
}

#[test]
fn funding_cluster_skips_unverified_rate() {
    let mut rows = vec![row("OKX", PositionSide::Long)];
    rows[0].funding_rate_8h = 0.002;
    rows[0].funding_rate_verified = false;

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.funding_clustering[0].position_count, 0);
    assert_eq!(risk.funding_clustering[0].total_outflow_usd, 0.0);
}

#[test]
fn margin_utilization_uses_account_initial_margin_over_equity() {
    let rows = vec![row("OKX", PositionSide::Long)];
    let summaries = vec![account_summary("okx", 100.0, 40.0, 5.0)];

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &summaries,
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    let margin = &risk.margin_utilization[0];
    assert_eq!(margin.utilization_pct, Some(40.0));
    assert_eq!(margin.initial_margin_usd, Some(40.0));
    assert_eq!(margin.maintenance_margin_usd, 5.0);
    assert_eq!(margin.equity_usd, 100.0);
    assert!(!margin.estimated);
}

#[test]
fn margin_utilization_uses_position_scope_estimate_without_account_summary() {
    let rows = vec![row("OKX", PositionSide::Long)];

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &[],
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    let margin = &risk.margin_utilization[0];
    assert_eq!(margin.utilization_pct, Some(100.0));
    assert_eq!(margin.initial_margin_usd, Some(50.0));
    assert!(margin.estimated);
}

#[test]
fn zero_account_initial_margin_does_not_hide_an_open_position() {
    let rows = vec![row("Gate", PositionSide::Long)];
    let summaries = vec![account_summary("gate", 100.0, 0.0, 0.0)];

    let risk = compute_risk(RiskInputs {
        positions: &rows,
        account_summaries: &summaries,
        historical_pnl_usd: &[],
        total_nav_usd: 1_000.0,
        hard_limits: HardLimitsUsage::default(),
        now_ms: 0,
    });

    assert_eq!(risk.margin_utilization[0].utilization_pct, Some(100.0));
    assert!(risk.margin_utilization[0].estimated);
}

fn account_summary(
    venue: &str,
    equity_usd: f64,
    initial_margin_usd: f64,
    maintenance_margin_usd: f64,
) -> VenueAccountSummary {
    VenueAccountSummary {
        venue: venue.to_owned(),
        account_type: "test".to_owned(),
        equity_scope: Default::default(),
        total_equity_usd: equity_usd,
        total_available_balance_usd: equity_usd - initial_margin_usd,
        withdrawable_balance_usd: None,
        total_initial_margin_usd: initial_margin_usd,
        total_maintenance_margin_usd: maintenance_margin_usd,
        account_im_rate: initial_margin_usd / equity_usd,
        account_mm_rate: maintenance_margin_usd / equity_usd,
        source: "test.account_summary".to_owned(),
        observed_at_ms: 1,
        freshness_ms: Some(0),
        problem: None,
    }
}

fn row(venue: &str, side: PositionSide) -> PositionRow {
    PositionRow {
        venue: venue.into(),
        symbol: "BTC".into(),
        origin: Default::default(),
        side,
        quantity: 1.0,
        entry_price: 100.0,
        mark_price: 100.0,
        leverage: 2.0,
        unrealized_pnl_usd: 0.0,
        liquidation_price: None,
        liquidation_distance_pct: None,
        next_funding_ms: Some(20 * 60_000),
        funding_rate_8h: 0.0,
        funding_rate_verified: true,
        maintenance_margin_ratio: 0.05,
        pair_evidence: None,
        paired_with: None,
        margin_usd: 50.0,
        severity: shared_types::PositionSeverity::Ok,
        seconds_until_funding: Some(20 * 60),
    }
}
