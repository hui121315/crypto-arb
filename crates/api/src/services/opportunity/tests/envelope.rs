use super::*;

mod freshness;
mod market_status;
mod payload;
mod query;

#[test]
fn counts_only_report_p0_strategy_keys() {
    let rows = vec![
        dto("perp", StrategyKind::PerpCross, true),
        dto("spot", StrategyKind::SpotPerp, false),
        dto("spot-cross", StrategyKind::SpotCross, true),
    ];

    let counts = counts(&rows);

    assert_eq!(counts.executable_count, 2);
    assert_eq!(
        counts.strategy_counts.get(&StrategyKind::PerpCross),
        Some(&1)
    );
    assert_eq!(
        counts.strategy_counts.get(&StrategyKind::SpotPerp),
        Some(&1)
    );
    assert_eq!(
        counts.strategy_counts.get(&StrategyKind::SpotCross),
        Some(&1)
    );
    assert_eq!(
        counts
            .executable_strategy_counts
            .get(&StrategyKind::PerpCross),
        Some(&1)
    );
    assert_eq!(
        counts
            .executable_strategy_counts
            .get(&StrategyKind::SpotPerp),
        None
    );
    assert_eq!(
        counts
            .executable_strategy_counts
            .get(&StrategyKind::SpotCross),
        Some(&1)
    );
}

#[test]
fn envelope_keeps_registry_and_main_p0_counts_separate() {
    let cached_at = Utc::now();
    let scan_started_at = cached_at - chrono::Duration::milliseconds(25);
    let source_rows = vec![
        dto("perp", StrategyKind::PerpCross, true),
        dto("spot-cross", StrategyKind::SpotCross, true),
    ];
    let filtered_rows = vec![source_rows[0].clone()];

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: filtered_rows.clone(),
        source_rows: &source_rows,
        filtered_rows: &filtered_rows,
        meta: OpportunityScanMeta {
            scan_started_at: Some(scan_started_at),
            scan_outcome: shared_types::OpportunityScanOutcome::Found,
            ..OpportunityScanMeta::default()
        },
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "strategy=perp_cross".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.total_count, 2);
    assert_eq!(env.filtered_count, 1);
    assert_eq!(env.returned_count, 1);
    assert_eq!(env.main_p0_counts.total_count, 1);
    assert_eq!(env.main_p0_counts.executable_count, 1);
    assert!(env.observed_at_ms >= cached_at.timestamp_millis());
    assert!(env.freshness_ms.is_some_and(|value| value >= 0));
    assert_eq!(env.scan_started_at, Some(scan_started_at));
    assert_eq!(env.meta.scan_started_at, Some(scan_started_at));
    assert_eq!(
        env.meta.scan_outcome,
        shared_types::OpportunityScanOutcome::Found
    );
    assert_eq!(env.registry_counts.total_count, 2);
    assert_eq!(
        env.registry_counts
            .strategy_counts
            .get(&StrategyKind::SpotCross),
        Some(&1)
    );
}

#[test]
fn stale_wide_envelope_retains_rows_and_reports_snapshot_problem() -> Result<(), &'static str> {
    let cached_at =
        Utc::now() - chrono::Duration::milliseconds(snapshot_health::SNAPSHOT_STALE_AFTER_MS + 100);
    let rows = vec![dto("cached-wide", StrategyKind::PerpCross, true)];

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: rows.clone(),
        source_rows: &rows,
        filtered_rows: &rows,
        meta: OpportunityScanMeta::default(),
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.opportunities.len(), 1);
    assert_eq!(env.opportunities[0].id, "cached-wide");
    assert_stale_snapshot_contract(
        env.status,
        env.cached_at,
        env.observed_at_ms,
        env.freshness_ms,
        env.retry_after_ms,
        env.error.as_ref(),
    )
}

#[test]
fn snapshot_classifier_preserves_threshold_warming_and_error_states() {
    let observed_at = Utc::now();
    let observed_at_ms = observed_at.timestamp_millis();
    let boundary_cached_at =
        observed_at - chrono::Duration::milliseconds(snapshot_health::SNAPSHOT_STALE_AFTER_MS);
    let old_cached_at = boundary_cached_at - chrono::Duration::milliseconds(1);

    let boundary = snapshot_health::classify_snapshot(
        OpportunityEnvelopeStatus::Fresh,
        &boundary_cached_at,
        observed_at_ms,
        0,
    );
    assert_eq!(boundary.status, OpportunityEnvelopeStatus::Fresh);
    assert_eq!(
        boundary.freshness_ms,
        snapshot_health::SNAPSHOT_STALE_AFTER_MS
    );
    assert!(boundary.problem.is_none());

    for status in [
        OpportunityEnvelopeStatus::Warming,
        OpportunityEnvelopeStatus::Error,
    ] {
        let classified =
            snapshot_health::classify_snapshot(status, &old_cached_at, observed_at_ms, 0);
        assert_eq!(classified.status, status);
        assert!(classified.problem.is_none());
    }
}

#[test]
fn warming_error_carries_retry_after() {
    let problem = warming_error();

    assert_eq!(problem.code, "OPPORTUNITY_SNAPSHOT_WARMING");
    assert_eq!(problem.retry_after_ms, Some(WARMING_RETRY_AFTER_MS));
    assert_eq!(problem.source.as_deref(), Some("arbitrage-snapshot"));
}

#[test]
fn legacy_wide_endpoint_problem_points_to_list_endpoint() {
    let problem = legacy_wide_endpoint_problem();

    assert_eq!(problem.code, codes::OPPORTUNITY_LEGACY_WIDE_ENDPOINT);
    assert_eq!(problem.source.as_deref(), Some("arbitrage-opportunities"));
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("replacement"))
            .and_then(serde_json::Value::as_str),
        Some("/api/v3/arbitrage/opportunities/list")
    );
}

#[test]
fn envelope_promotes_market_status_problem_to_partial_failure() {
    let cached_at = Utc::now();
    let meta = rate_limited_market_meta(2_000);

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: Vec::new(),
        source_rows: &[],
        filtered_rows: &[],
        meta,
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.status, OpportunityEnvelopeStatus::Degraded);
    assert_eq!(env.retry_after_ms, Some(2_000));
    assert_eq!(env.partial_failures.len(), 1);
    assert_eq!(env.partial_failures[0].code, "MARKET_DATA_RATE_LIMITED");
    assert_eq!(env.partial_failures[0].retry_after_ms, Some(2_000));
}

#[test]
fn envelope_keeps_expected_unsupported_metadata_out_of_partial_failures() {
    let cached_at = Utc::now();

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: Vec::new(),
        source_rows: &[],
        filtered_rows: &[],
        meta: unsupported_metadata_meta(),
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.status, OpportunityEnvelopeStatus::Fresh);
    assert!(env.partial_failures.is_empty());
}

#[test]
fn envelope_keeps_optional_index_diagnostics_out_of_scan_failures() {
    let cached_at = Utc::now();

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: Vec::new(),
        source_rows: &[],
        filtered_rows: &[],
        meta: optional_index_problem_meta(),
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.status, OpportunityEnvelopeStatus::Fresh);
    assert!(env.partial_failures.is_empty());
}

#[test]
fn envelope_uses_max_retry_after_from_input_and_partial_failures() {
    let cached_at = Utc::now();

    let env = envelope(OpportunityEnvelopeInput {
        opportunities: Vec::new(),
        source_rows: &[],
        filtered_rows: &[],
        meta: rate_limited_market_meta(5_000),
        cached_at,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: Some(2_000),
        error: None,
        query_problems: Vec::new(),
    });

    assert_eq!(env.retry_after_ms, Some(5_000));
}
