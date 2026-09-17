use super::*;

#[test]
fn list_envelope_projects_light_rows_and_cursor_page() {
    let cached_at = Utc::now();
    let scan_started_at = cached_at - chrono::Duration::milliseconds(20);
    let env = authoritative_page_envelope(cached_at, scan_started_at);

    assert_authoritative_page(&env);
    assert_authoritative_counts(&env, scan_started_at);
    assert_compact_list_meta(&env);
    assert_eq!(env.rows[0].cost.fee_evidence_count, 2);
    assert_eq!(
        env.rows[0].cost.fee_evidence_ids,
        ["fee:a:perp:vip0", "fee:b:perp:vip0"]
    );
}

fn authoritative_page_envelope(
    cached_at: chrono::DateTime<Utc>,
    scan_started_at: chrono::DateTime<Utc>,
) -> OpportunityListEnvelope {
    let rows = (0..150)
        .map(|idx| dto(&format!("row-{idx}"), StrategyKind::PerpCross, true))
        .collect::<Vec<_>>();
    let window = OpportunityListWindow::from_query(Some(500), Some("120"), Some("score"));
    let refs = rows.iter().collect::<Vec<_>>();
    let page_refs = page_refs(&refs, window);
    list_envelope(OpportunityListEnvelopeInput {
        rows: page_refs,
        source_rows: &rows,
        request_meta: request_meta(window),
        strategy_scope_count: rows.len(),
        filtered_count: rows.len(),
        symbol_scope_count: None,
        meta: OpportunityScanMeta {
            candidate_count: 150,
            emitted_count: 150,
            scan_started_at: Some(scan_started_at),
            funding_row_evidence: vec![funding_row_evidence()],
            ..OpportunityScanMeta::default()
        },
        cached_at,
        snapshot_id: Some("authoritative-snapshot"),
        source: "test",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        window,
    })
}

fn assert_authoritative_page(env: &OpportunityListEnvelope) {
    assert_eq!(env.rows.len(), 30);
    assert_eq!(env.page.page_size, MAX_LIST_PAGE_SIZE);
    assert_eq!(env.page.snapshot_id, "authoritative-snapshot");
    assert_page_size_clamp(env);
    assert_eq!(env.page.start_offset, 120);
    assert!(!env.page.has_next_page);
}

fn assert_authoritative_counts(
    env: &OpportunityListEnvelope,
    scan_started_at: chrono::DateTime<Utc>,
) {
    assert_eq!(env.scope_meta.global_total_count, 150);
    assert_eq!(env.scope_meta.strategy_scope_count, 150);
    assert_eq!(env.scope_meta.filtered_count, 150);
    assert_eq!(env.scope_meta.page_count, 2);
    assert_eq!(env.main_p0_counts.total_count, 150);
    assert_eq!(env.main_p0_counts.executable_count, 150);
    assert_eq!(env.registry_counts.total_count, 150);
    assert_eq!(env.meta.scan_started_at, Some(scan_started_at));
}

fn assert_compact_list_meta(envelope: &OpportunityListEnvelope) {
    assert!(envelope.meta.funding_row_evidence.is_empty());
}

fn funding_row_evidence() -> shared_types::MarketDataRowEvidence {
    shared_types::MarketDataRowEvidence {
        venue: "binance".into(),
        symbol: "BTCUSDT".into(),
        operation: shared_types::MarketDataSnapshotOperation::FundingRates,
        health: shared_types::MarketDataHealth {
            quality: shared_types::MarketDataQuality::Fresh,
            source: shared_types::MarketDataSourceKind::RestBaseline,
            freshness_ms: Some(1),
            retry_after_ms: None,
            last_error: None,
            observed_at_ms: 1,
            coverage: Some(shared_types::MarketDataCoverage::new(1, 1)),
            problem: None,
        },
    }
}

#[test]
fn list_row_from_dto_preserves_spot_leg_mode() {
    let mut row = dto("spot-reverse", StrategyKind::SpotPerp, true);
    row.spot_leg_mode = Some(shared_types::SpotLegMode::BorrowAndSell);

    let projected = list_row_from_dto(&row);

    assert_eq!(
        projected.spot_leg_mode,
        Some(shared_types::SpotLegMode::BorrowAndSell)
    );
}

#[test]
fn list_envelope_promotes_partial_failure_retry_after() {
    let cached_at = Utc::now();
    let window = OpportunityListWindow::from_query(Some(50), None, Some("score"));

    let env = list_envelope(OpportunityListEnvelopeInput {
        rows: Vec::new(),
        source_rows: &[],
        request_meta: request_meta(window),
        strategy_scope_count: 0,
        filtered_count: 0,
        symbol_scope_count: None,
        meta: rate_limited_market_meta(2_000),
        cached_at,
        snapshot_id: None,
        source: "test",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        window,
    });

    assert_eq!(env.status, OpportunityEnvelopeStatus::Degraded);
    assert_eq!(env.retry_after_ms, Some(2_000));
    assert_eq!(env.partial_failures[0].retry_after_ms, Some(2_000));
}

#[test]
fn perp_cross_list_ignores_unneeded_spot_tick_failure() {
    let env = scoped_market_health_envelope(
        StrategyKind::PerpCross,
        shared_types::MarketDataSnapshotOperation::SpotTicks,
    );

    assert_eq!(env.status, OpportunityEnvelopeStatus::Fresh);
    assert_eq!(env.meta.market_data_problem_count, 0);
    assert!(env.meta.degraded_venues.is_empty());
    assert!(env.partial_failures.is_empty());
}

#[test]
fn spot_perp_list_keeps_required_spot_tick_failure() {
    let env = scoped_market_health_envelope(
        StrategyKind::SpotPerp,
        shared_types::MarketDataSnapshotOperation::SpotTicks,
    );

    assert_eq!(env.status, OpportunityEnvelopeStatus::Degraded);
    assert_eq!(env.meta.market_data_problem_count, 1);
    assert_eq!(env.meta.degraded_venues, ["all:spot_ticks"]);
    assert_eq!(env.partial_failures.len(), 1);
}

fn scoped_market_health_envelope(
    strategy: StrategyKind,
    operation: shared_types::MarketDataSnapshotOperation,
) -> OpportunityListEnvelope {
    let cached_at = Utc::now();
    let window = OpportunityListWindow::from_query(Some(50), None, Some("score"));
    let mut request_meta = request_meta(window);
    request_meta.filter.strategy_kinds = vec![strategy];

    list_envelope(OpportunityListEnvelopeInput {
        rows: Vec::new(),
        source_rows: &[],
        request_meta,
        strategy_scope_count: 0,
        filtered_count: 0,
        symbol_scope_count: None,
        meta: missing_aggregate_market_meta(operation),
        cached_at,
        snapshot_id: None,
        source: "test",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        window,
    })
}

fn missing_aggregate_market_meta(
    operation: shared_types::MarketDataSnapshotOperation,
) -> OpportunityScanMeta {
    OpportunityScanMeta {
        market_data_problem_count: 1,
        degraded_venues: vec![format!("all:{}", operation.as_str())],
        market_data_status: Some(shared_types::MarketDataSnapshotStatus {
            observed_at_ms: 1,
            rows: vec![shared_types::MarketDataSnapshotStatusRow {
                venue: crate::services::market_data::cache::MARKET_AGGREGATE_VENUE.into(),
                operation,
                health: shared_types::MarketDataHealth {
                    quality: shared_types::MarketDataQuality::Missing,
                    source: shared_types::MarketDataSourceKind::RestBaseline,
                    freshness_ms: None,
                    retry_after_ms: None,
                    last_error: Some("aggregate input missing".into()),
                    observed_at_ms: 1,
                    coverage: Some(shared_types::MarketDataCoverage::new(1, 0)),
                    problem: Some(
                        ApiProblem::new("MARKET_DATA_MISSING", "aggregate input missing")
                            .with_source("rest_baseline"),
                    ),
                },
            }],
        }),
        ..OpportunityScanMeta::default()
    }
}

#[test]
fn list_row_from_dto_uses_cost_evidence_without_score_override() {
    let mut row = dto("score-cost", StrategyKind::PerpCross, true);
    row.score_breakdown = Some(shared_types::ScoreBreakdown {
        one_cycle_net_bps: -7.0,
        fee_evidence_ids: vec!["fee:score:long".into(), "fee:score:short".into()],
        fee_evidence_complete: true,
        one_cycle_penalty: 4.2,
        ..shared_types::ScoreBreakdown::default()
    });

    let projected = list_row_from_dto(&row);

    assert_eq!(
        projected.cost.fee_evidence_ids,
        ["fee:a:perp:vip0", "fee:b:perp:vip0"]
    );
    assert!(projected.metrics.one_cycle_net_bps.is_some());
    assert_eq!(
        projected.metrics.one_cycle_net_bps,
        projected.cost.one_cycle_net_bps
    );
    assert_eq!(projected.cost.one_cycle_penalty, 0.0);
    assert!(projected.cost.fee_evidence_complete);
}

#[test]
fn legacy_score_evidence_cannot_verify_or_penalize_cost() -> Result<(), serde_json::Error> {
    let mut row = dto("score-only-cost", StrategyKind::PerpCross, true);
    if let Some(cost) = row.execution_cost.as_mut() {
        cost.round_trip = None;
    }
    row.score_breakdown = Some(shared_types::ScoreBreakdown {
        one_cycle_net_bps: -7.0,
        fee_evidence_ids: vec!["fee:score:long".into(), "fee:score:short".into()],
        fee_evidence_complete: true,
        one_cycle_penalty: 4.2,
        ..shared_types::ScoreBreakdown::default()
    });

    let projected = list_row_from_dto(&row);

    assert!(!projected.cost.verified);
    assert!(!projected.cost.fee_evidence_complete);
    assert_eq!(projected.cost.fee_evidence_count, 0);
    assert!(projected.cost.fee_evidence_ids.is_empty());
    assert_eq!(projected.metrics.one_cycle_net_bps, None);
    assert_eq!(projected.cost.one_cycle_net_bps, None);
    assert_eq!(projected.cost.one_cycle_penalty, 0.0);

    let json = serde_json::to_value(&projected)?;
    assert!(json["metrics"].get("oneCycleNetBps").is_none());
    assert!(json["cost"].get("oneCycleNetBps").is_none());
    Ok(())
}

#[test]
fn list_row_fails_closed_when_typed_profitability_evidence_is_missing() -> Result<(), &'static str>
{
    let mut row = dto(
        "missing-profitability-evidence",
        StrategyKind::PerpCross,
        true,
    );
    let round_trip = row
        .execution_cost
        .as_mut()
        .and_then(|cost| cost.round_trip.as_mut())
        .ok_or("round trip fixture missing")?;
    round_trip.profitability_evidence = shared_types::ProfitabilityEvidence::default();

    let projected = list_row_from_dto(&row);

    assert!(!projected.cost.verified);
    assert!(!projected.cost.fee_evidence_complete);
    assert_eq!(projected.cost.fee_evidence_count, 2);
    assert!(projected.cost.fee_evidence_ids.is_empty());
    assert_eq!(projected.cost.one_cycle_net_bps, None);
    assert_eq!(projected.metrics.one_cycle_net_bps, None);
    Ok(())
}

#[test]
fn sort_refs_orders_by_settlement_without_legacy_score() {
    let mut slow = dto("slow", StrategyKind::PerpCross, true);
    slow.score = 99.0;
    slow.settlement_countdown_seconds = Some(600);
    let mut fast = dto("fast", StrategyKind::PerpCross, true);
    fast.score = 10.0;
    fast.settlement_countdown_seconds = Some(60);
    let rows = [slow, fast];
    let mut refs = rows.iter().collect::<Vec<_>>();

    sort_refs(
        refs.as_mut_slice(),
        OpportunityListSortKey::Settlement,
        common::time::now_ms(),
    );

    assert_eq!(refs[0].id, "fast");
}

fn assert_page_size_clamp(envelope: &OpportunityListEnvelope) {
    assert_eq!(envelope.request_meta.requested_page_size, Some(500));
    assert_eq!(envelope.request_meta.applied_page_size, MAX_LIST_PAGE_SIZE);
    assert_eq!(envelope.request_meta.max_page_size, MAX_LIST_PAGE_SIZE);
    assert_eq!(envelope.status, OpportunityEnvelopeStatus::Degraded);
    assert_eq!(envelope.partial_failures.len(), 1);
    assert_eq!(envelope.partial_failures[0].code, codes::LIST_LIMIT_CLAMPED);
    assert_eq!(
        envelope.partial_failures[0].details,
        Some(serde_json::json!({
            "field": "pageSize",
            "requested": 500,
            "applied": MAX_LIST_PAGE_SIZE,
            "maxPageSize": MAX_LIST_PAGE_SIZE,
        }))
    );
}
