use super::*;

#[test]
fn stale_list_envelope_retains_rows_and_reports_snapshot_problem() -> Result<(), &'static str> {
    let cached_at =
        Utc::now() - chrono::Duration::milliseconds(snapshot_health::SNAPSHOT_STALE_AFTER_MS + 100);
    let rows = vec![dto("cached-list", StrategyKind::PerpCross, true)];
    let window = OpportunityListWindow::from_query(Some(50), None, Some("score"));
    let env = list_envelope(OpportunityListEnvelopeInput {
        rows: rows.iter().collect(),
        source_rows: &rows,
        request_meta: request_meta(window),
        strategy_scope_count: rows.len(),
        filtered_count: rows.len(),
        symbol_scope_count: None,
        meta: OpportunityScanMeta::default(),
        cached_at,
        snapshot_id: None,
        source: "snapshot",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "test".into(),
        retry_after_ms: None,
        error: None,
        window,
    });

    assert_eq!(
        env.rows.first().map(|row| row.id.as_str()),
        Some("cached-list")
    );
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
fn stale_stream_event_retains_snapshot_ids_and_reports_problem() -> Result<(), &'static str> {
    let cached_at =
        Utc::now() - chrono::Duration::milliseconds(snapshot_health::SNAPSHOT_STALE_AFTER_MS + 100);
    let rows = vec![dto("cached-stream", StrategyKind::PerpCross, true)];
    let event = stream_event(OpportunityStreamEventInput {
        source_rows: &rows,
        meta: OpportunityScanMeta::default(),
        cached_at,
        snapshot_id: None,
        source: "snapshot-replay",
        status: OpportunityEnvelopeStatus::Fresh,
        scope: OpportunityEnvelopeScope::MainP0,
        query_key: "scope=main_p0".into(),
        retry_after_ms: None,
        error: None,
        full_window_rows: false,
    });

    assert_eq!(event.main_p0_counts.total_count, 1);
    assert_eq!(event.top_ids, ["cached-stream"]);
    assert_stale_snapshot_contract(
        event.status,
        event.cached_at,
        event.observed_at_ms,
        event.freshness_ms,
        event.retry_after_ms,
        event.error.as_ref(),
    )
}
