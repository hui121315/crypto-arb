use super::*;

#[test]
fn portfolio_snapshot_defaults_operation_health_for_legacy_payloads() {
    let snapshot: PortfolioSnapshot = serde_json::from_value(serde_json::json!({
        "summary": {
            "totalNavUsd": 0.0,
            "navChange24hPct": 0.0,
            "netDeltaUsd": 0.0,
            "netDeltaPctOfNav": 0.0,
            "nakedExposureUsd": 0.0,
            "nakedPositionCount": 0,
            "realizedPnlTodayUsd": 0.0,
            "pnlBreakdown": {
                "fundingUsd": 0.0,
                "priceUsd": 0.0,
                "feeRebateUsd": 0.0
            },
            "updatedAtMs": 1
        },
        "positions": [],
        "balances": [],
        "risk": {
            "var991dUsd": 0.0,
            "varPctOfNav": 0.0,
            "fundingClustering": [],
            "deltaConcentration": [],
            "marginUtilization": [],
            "hardLimits": {
                "openOrdersUsed": 0,
                "openOrdersMax": 0,
                "maxSymbolNotionalUsd": 0.0,
                "maxOrderNotionalUsd": 0.0,
                "killSwitchActive": false
            },
            "updatedAtMs": 1
        },
        "serverNowMs": 1,
        "snapshotVersion": "pos-0",
        "degraded": false,
        "problems": []
    }))
    .expect("legacy portfolio snapshot");

    assert!(snapshot.operation_health.is_empty());
    assert_eq!(snapshot.account_state.source, "account_state_default");
    assert!(snapshot.recent_close_runs.is_empty());
    assert_eq!(
        snapshot.summary.nav_evidence.breakdown.cash.status,
        AccountFieldQualityStatus::Missing
    );
    assert_eq!(
        snapshot.summary.pnl_breakdown.evidence.quality,
        ExecutionLedgerQuality::Missing
    );
}

#[test]
fn portfolio_snapshot_serializes_operation_health() {
    let snapshot = PortfolioSnapshot {
        summary: PortfolioSummary {
            total_nav_usd: 0.0,
            nav_evidence: PortfolioNavEvidence::default(),
            nav_change_24h_pct: Some(0.0),
            net_delta_usd: 0.0,
            net_delta_pct_of_nav: 0.0,
            naked_exposure_usd: 0.0,
            naked_position_count: 0,
            realized_pnl_today_usd: 0.0,
            pnl_breakdown: PnlBreakdown::default(),
            updated_at_ms: 1,
        },
        positions: Vec::new(),
        balances: Vec::new(),
        risk: RiskSnapshot {
            var_99_1d_usd: 0.0,
            var_pct_of_nav: 0.0,
            var_sample_size: 0,
            funding_clustering: Vec::new(),
            delta_concentration: Vec::new(),
            margin_utilization: Vec::new(),
            hard_limits: HardLimitsUsage::default(),
            updated_at_ms: 1,
        },
        server_now_ms: 1,
        snapshot_version: "pos-0".to_owned(),
        degraded: false,
        problems: Vec::new(),
        operation_health: vec![VenueOperationHealth {
            venue: "system".to_owned(),
            operation: "storage:portfolio_nav".to_owned(),
            status: VenueOperationStatus::Warn,
            source: "portfolio_nav_store".to_owned(),
            message: "storage warning".to_owned(),
            supported: Some(true),
            configured: Some(false),
            requested: Some(0),
            rows: Some(0),
            freshness_ms: None,
            retry_after_ms: None,
            latency_ms: None,
            latency_p95_ms: None,
            error: Some("storage warning".to_owned()),
            evidence: None,
            problem: None,
            observed_at_ms: 1,
        }],
        account_state: AccountStateSnapshot::default(),
        recent_close_runs: vec![CloseRun {
            id: "close-1".to_owned(),
            scope: CloseRunScope::Pair,
            status: CloseRunStatus::UnwindRequired,
            action_run_id: Some("act-1".to_owned()),
            request_id: Some("req-1".to_owned()),
            idempotency_key: None,
            snapshot_version: "pos-1".to_owned(),
            expected_leg_count: 2,
            reason: Some("positions.close_pair".to_owned()),
            legs: Vec::new(),
            submitted_order_count: 2,
            failed_leg_count: 1,
            naked_exposure_usd: 10.0,
            message: "unwind required".to_owned(),
            problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            unwind_plan: None,
            cost_events: Vec::new(),
            cost_reconciliation: None,
            started_at_ms: 1,
            updated_at_ms: 2,
        }],
    };

    let text = serde_json::to_string(&snapshot).expect("serialize snapshot");

    assert!(text.contains("\"operationHealth\""));
    assert!(text.contains("\"storage:portfolio_nav\""));
    assert!(text.contains("\"accountState\""));
    assert!(text.contains("\"recentCloseRuns\""));
    assert!(text.contains("\"unwind_required\""));
    assert!(text.contains("\"breakdown\""));
    assert!(text.contains("\"walletEquity\""));
    assert!(text.contains("\"evidence\""));
    assert!(text.contains("\"quality\":\"missing\""));
}

#[test]
fn portfolio_snapshot_envelope_can_carry_error_without_fake_snapshot() {
    let problem = ApiProblem::new(
        crate::problem::codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE,
        "portfolio snapshot unavailable",
    )
    .with_retry_after_ms(Some(2_000))
    .with_source("portfolio_lifecycle");
    let envelope = PortfolioSnapshotEnvelope {
        status: PortfolioSnapshotStatus::Error,
        source: "portfolio_lifecycle".to_owned(),
        observed_at_ms: 42,
        snapshot: None,
        problem: Some(problem.clone()),
        problems: vec![problem],
        operation_health: vec![VenueOperationHealth {
            venue: "system".to_owned(),
            operation: "portfolio:snapshot".to_owned(),
            status: VenueOperationStatus::Blocked,
            source: "portfolio_lifecycle".to_owned(),
            message: "portfolio snapshot unavailable".to_owned(),
            supported: Some(true),
            configured: None,
            requested: Some(1),
            rows: Some(0),
            freshness_ms: None,
            retry_after_ms: Some(2_000),
            latency_ms: None,
            latency_p95_ms: None,
            error: Some("rate limited".to_owned()),
            evidence: None,
            problem: None,
            observed_at_ms: 42,
        }],
        retry_after_ms: Some(2_000),
    };

    let text = serde_json::to_string(&envelope).expect("serialize portfolio envelope");

    assert!(text.contains("\"status\":\"error\""));
    assert!(!text.contains("\"snapshot\""));
    assert!(text.contains("PORTFOLIO_SNAPSHOT_UNAVAILABLE"));
    assert!(text.contains("\"operationHealth\""));
}

#[test]
fn portfolio_snapshot_envelope_keeps_empty_operation_health_field() {
    let envelope = PortfolioSnapshotEnvelope {
        status: PortfolioSnapshotStatus::Error,
        source: "portfolio_lifecycle".to_owned(),
        observed_at_ms: 42,
        snapshot: None,
        problem: None,
        problems: Vec::new(),
        operation_health: Vec::new(),
        retry_after_ms: None,
    };

    let text = serde_json::to_string(&envelope).expect("serialize portfolio envelope");

    assert!(text.contains("\"operationHealth\":[]"));
    assert!(!text.contains("\"snapshot\""));
}
