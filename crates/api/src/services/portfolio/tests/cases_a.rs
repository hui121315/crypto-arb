use super::super::*;
use super::*;

mod nav;

#[test]
fn account_state_problem_maps_to_portfolio_runtime_problem() {
    let mut problem = ApiProblem::new(
        shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN,
        "account equity unknown",
    )
    .with_retry_after_ms(Some(1_000));
    problem.details = Some(serde_json::json!({
        "venue": "okx",
        "operation": "account_state",
    }));

    let runtime = account_api_problem(&problem, 42);

    assert_eq!(runtime.scope, "portfolio");
    assert_eq!(runtime.operation, "account_state");
    assert_eq!(
        runtime.code,
        shared_types::problem::codes::ACCOUNT_FIELD_UNKNOWN
    );
    assert_eq!(runtime.venue.as_deref(), Some("okx"));
    assert_eq!(runtime.retry_after_ms, Some(1_000));
    assert_eq!(runtime.observed_at_ms, 42);
}

#[test]
fn maps_adapter_positions_without_execution_evidence() {
    let rows = rows_from_positions(vec![position("long"), position("short")], &[], 0, 15.0, 8.0);

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.paired_with.is_none()));
    assert!(rows.iter().all(|row| row.pair_evidence.is_none()));
    assert_eq!(summary_from_rows(&rows, 0).naked_position_count, 2);
    assert_eq!(rows[0].liquidation_distance_pct, Some(20.0));
    assert_eq!(rows[0].severity, PositionSeverity::Ok);
}

#[test]
fn applies_execution_run_pair_evidence_to_position_rows() -> Result<(), &'static str> {
    let run = execution_run();
    let evidence = execution_run_pair_evidence(&run).ok_or("missing pair evidence")?;
    let rows = rows_from_sources(
        vec![position("long"), position("short")],
        &[],
        &[],
        false,
        &evidence,
        risk_annotation(),
    );

    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|row| row.pair_evidence.is_some()));
    assert_eq!(rows[0].paired_with.as_deref(), Some("OKX@BTC"));
    assert_eq!(
        rows[0]
            .pair_evidence
            .as_ref()
            .map(|item| item.run_id.as_str()),
        Some("run-1")
    );
    assert_eq!(summary_from_rows(&rows, 0).naked_position_count, 0);
    Ok(())
}

#[test]
fn skips_flat_or_unknown_position_rows() {
    let mut flat = position("long");
    flat.quantity = 0.0;

    let rows = rows_from_positions(vec![flat, position("both")], &[], 0, 15.0, 8.0);

    assert!(rows.is_empty());
}

#[test]
fn summary_uses_zero_when_daily_pnl_sources_are_absent() {
    let rows = rows_from_positions(vec![position("long")], &[], 0, 15.0, 8.0);
    let summary = summary_from_rows(&rows, 7);

    assert_eq!(summary.realized_pnl_today_usd, 0.0);
    assert_eq!(summary.pnl_breakdown.funding_usd, 0.0);
    assert_eq!(summary.nav_change_24h_pct, Some(0.0));
}

#[test]
fn summary_uses_journal_derived_pnl_inputs() {
    let rows = rows_from_positions(vec![position("long")], &[], 0, 15.0, 8.0);
    let summary = summary_from_rows_with_nav(
        &rows,
        &fresh_position_account_state(),
        7,
        AccountNav {
            value: position_equity_estimate(&rows),
            evidence: PortfolioNavEvidence {
                status: AccountFieldQualityStatus::Actual,
                ..PortfolioNavEvidence::default()
            },
        },
        Some(position_equity_estimate(&rows)),
        PortfolioPnlToday {
            realized_pnl_usd: 8.0,
            funding_usd: 3.0,
            fee_rebate_usd: -2.0,
            evidence: shared_types::PortfolioPnlEvidence {
                quality: shared_types::ExecutionLedgerQuality::Actual,
                source: "test_execution_ledger".to_owned(),
                ..shared_types::PortfolioPnlEvidence::default()
            },
        },
    );

    assert_eq!(summary.realized_pnl_today_usd, 8.0);
    assert_eq!(summary.pnl_breakdown.funding_usd, 3.0);
    assert_eq!(summary.pnl_breakdown.fee_rebate_usd, -2.0);
    assert_eq!(summary.pnl_breakdown.price_usd, 7.0);
    assert_eq!(
        summary.pnl_breakdown.evidence.quality,
        shared_types::ExecutionLedgerQuality::Actual
    );
}

fn fresh_position_account_state() -> AccountStateSnapshot {
    AccountStateSnapshot {
        positions: shared_types::VenuePositionEnvelope::new(
            Vec::new(),
            ListStatus::Fresh,
            "account_position_runtime",
            7,
            Vec::new(),
            Vec::new(),
        ),
        ..AccountStateSnapshot::default()
    }
}

fn hyperliquid_unified_binding() -> shared_types::AccountBindingEvidence {
    shared_types::AccountBindingEvidence {
        venue: "hyperliquid".to_owned(),
        account_scope: Some("unifiedAccount".to_owned()),
        status: shared_types::AccountBindingStatus::Verified,
        source: "userAbstraction".to_owned(),
        checked_at_ms: Some(7),
        freshness_ms: Some(0),
        credential_fingerprint: None,
        problem: None,
    }
}
