use super::*;
use shared_types::{
    AccountStateSnapshot, HardLimitsUsage, PnlBreakdown, PortfolioSummary, RiskSnapshot,
};
use std::time::Duration;

#[test]
fn empty_lifecycle_cache_returns_typed_warming_error() {
    let envelope = lifecycle_cache_response(None, Duration::from_secs(2), chrono::Utc::now());

    assert_eq!(envelope.status, PortfolioSnapshotStatus::Error);
    assert!(envelope.snapshot.is_none());
    assert_eq!(
        envelope
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE)
    );
    assert_eq!(envelope.retry_after_ms, Some(2_000));
}

#[test]
fn fresh_lifecycle_cache_is_returned_without_status_rewrite() {
    let now = chrono::Utc::now();
    let entry = realtime::SnapshotEntry {
        value: snapshot_envelope(snapshot("fresh"), SOURCE_LIFECYCLE, now.timestamp_millis()),
        cached_at: now - chrono::Duration::seconds(1),
    };

    let envelope = lifecycle_cache_response(Some(&entry), Duration::from_secs(2), now);

    assert_eq!(envelope.status, PortfolioSnapshotStatus::Fresh);
    assert_eq!(envelope.source, SOURCE_LIFECYCLE);
    assert!(envelope.problem.is_none());
}

#[test]
fn stalled_lifecycle_cache_serves_last_snapshot_as_stale() {
    let now = chrono::Utc::now();
    let entry = realtime::SnapshotEntry {
        value: snapshot_envelope(
            snapshot("last-good"),
            SOURCE_LIFECYCLE,
            now.timestamp_millis(),
        ),
        cached_at: now - chrono::Duration::seconds(5),
    };

    let envelope = lifecycle_cache_response(Some(&entry), Duration::from_secs(2), now);

    assert_eq!(envelope.status, PortfolioSnapshotStatus::Stale);
    assert_eq!(envelope.source, SOURCE_LIFECYCLE_STALE);
    assert_eq!(
        envelope
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::PORTFOLIO_SNAPSHOT_STALE)
    );
    assert!(envelope
        .snapshot
        .as_ref()
        .is_some_and(|snapshot| snapshot.degraded));
    assert_eq!(
        envelope
            .operation_health
            .last()
            .and_then(|row| row.freshness_ms),
        Some(5_000)
    );
}

#[test]
fn cold_error_envelope_has_no_fake_snapshot() {
    let error = AppError::RateLimited {
        retry_after_secs: 2,
    };
    let envelope = error_envelope(&error, SOURCE_LIFECYCLE, 1_000);

    assert_eq!(envelope.status, PortfolioSnapshotStatus::Error);
    assert!(envelope.snapshot.is_none());
    assert_eq!(
        envelope
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::PORTFOLIO_SNAPSHOT_UNAVAILABLE)
    );
    assert_eq!(envelope.retry_after_ms, Some(2_000));
    assert_eq!(envelope.operation_health.len(), 1);
    assert_eq!(
        envelope.operation_health[0].status,
        VenueOperationStatus::Blocked
    );
}

#[test]
fn stale_envelope_keeps_snapshot_and_adds_problem() {
    let error = AppError::Upstream {
        exchange: "gate".to_owned(),
        message: "positions parse failed".to_owned(),
    };
    let envelope = stale_envelope(snapshot("pos-1"), &error, SOURCE_LIFECYCLE_STALE, 2_000);

    assert_eq!(envelope.status, PortfolioSnapshotStatus::Stale);
    assert_eq!(
        envelope
            .snapshot
            .as_ref()
            .map(|snapshot| snapshot.snapshot_version.as_str()),
        Some("pos-1")
    );
    assert!(envelope.problem.is_some());
}

#[test]
fn degraded_envelope_surfaces_nav_and_runtime_problems_without_duplicates() {
    let mut snapshot = snapshot("pos-degraded");
    let nav_problem = ApiProblem::new(codes::ACCOUNT_FIELD_UNKNOWN, "NAV equity missing")
        .with_retry_after_ms(Some(2_000))
        .with_source("account_state.account_summaries");
    snapshot.degraded = true;
    snapshot.summary.nav_evidence.problem = Some(nav_problem.clone());
    snapshot.problems.push(RuntimeProblem {
        scope: "portfolio".to_owned(),
        operation: "positions".to_owned(),
        code: codes::POSITION_READ_DEGRADED.to_owned(),
        message: "positions partial".to_owned(),
        venue: Some("gate".to_owned()),
        retry_after_ms: Some(1_000),
        problem: None,
        observed_at_ms: 7,
    });
    snapshot.account_state.problems.push(nav_problem);

    let envelope = snapshot_envelope(snapshot, SOURCE_LIFECYCLE, 8);

    assert_eq!(envelope.status, PortfolioSnapshotStatus::Degraded);
    assert_eq!(envelope.retry_after_ms, Some(2_000));
    assert_eq!(envelope.problems.len(), 2);
    assert_eq!(
        envelope
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::ACCOUNT_FIELD_UNKNOWN)
    );
    assert!(envelope.problems.iter().any(|problem| {
        problem.code == codes::POSITION_READ_DEGRADED
            && problem
                .details
                .as_ref()
                .and_then(|details| details.get("venue"))
                .and_then(|venue| venue.as_str())
                == Some("gate")
    }));
}

#[test]
fn field_quality_only_degradation_gets_typed_envelope_problem() {
    let mut snapshot = snapshot("pos-quality");
    snapshot.degraded = true;
    snapshot.summary.nav_evidence.status = AccountFieldQualityStatus::Actual;

    let envelope = snapshot_envelope(snapshot, SOURCE_LIFECYCLE, 9);

    assert_eq!(envelope.status, PortfolioSnapshotStatus::Degraded);
    assert_eq!(
        envelope
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::PORTFOLIO_SNAPSHOT_DEGRADED)
    );
}

#[test]
fn envelope_deduplicates_same_visible_problem_with_projection_only_detail_drift() {
    let mut snapshot = snapshot("pos-deduplicated");
    snapshot.degraded = true;
    snapshot.summary.nav_evidence.status = AccountFieldQualityStatus::Actual;
    let mut detailed = ApiProblem::new(
        codes::POSITION_EVIDENCE_MISSING,
        "position rows are empty and no fresh position evidence is available",
    )
    .with_source("account_position_runtime");
    detailed.details = Some(serde_json::json!({
        "operation": "positions",
        "venue": null,
        "observedAtMs": 1_000,
    }));
    let mut projected = detailed.clone();
    projected.details = Some(serde_json::json!({
        "operation": "positions",
        "observedAtMs": 1_001,
    }));
    snapshot.account_state.problems.push(detailed);
    snapshot.account_state.positions.problems.push(projected);

    let envelope = snapshot_envelope(snapshot, SOURCE_LIFECYCLE, 1_002);

    assert_eq!(envelope.problems.len(), 1);
    assert_eq!(envelope.problems[0].code, codes::POSITION_EVIDENCE_MISSING);
}

fn snapshot(version: &str) -> PortfolioSnapshot {
    PortfolioSnapshot {
        summary: PortfolioSummary {
            total_nav_usd: 1.0,
            nav_evidence: shared_types::PortfolioNavEvidence::default(),
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
        snapshot_version: version.to_owned(),
        degraded: false,
        problems: Vec::new(),
        operation_health: Vec::new(),
        account_state: AccountStateSnapshot::default(),
        recent_close_runs: Vec::new(),
    }
}
