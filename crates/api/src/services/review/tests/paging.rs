use super::super::*;
use super::*;

#[test]
fn missed_envelope_pages_rows_and_clamps_bad_query() {
    let now_ms = common::time::now_ms();
    let rows = (0..3)
        .map(|idx| MissedOpportunity {
            id: format!("miss-{idx}"),
            opportunity_id: format!("opp-{idx}"),
            strategy: StrategyKind::PerpCross,
            symbol: "BTC".into(),
            detected_at_ms: now_ms - i64::from(idx) * 1_000,
            expected_pnl_usd: 1.0,
            reason: shared_types::MissReason::ManualSkip,
            detail: "manual".into(),
        })
        .collect::<Vec<_>>();

    let envelope = missed_envelope(
        &rows,
        1,
        &ReviewPageQuery::new(Some(usize::MAX), Some("bad".into())),
    );

    assert_eq!(envelope.page.limit, REVIEW_MAX_LIMIT);
    assert_eq!(envelope.page.start_offset, 0);
    assert_eq!(envelope.page.total_rows, 3);
    assert_eq!(envelope.rows.len(), 3);
    assert_eq!(envelope.status, ListStatus::Degraded);
    assert_review_storage_health(
        &envelope,
        "storage:review_missed_opportunities",
        envelope.page.total_rows,
    );
    assert!(envelope
        .problems
        .iter()
        .any(|problem| problem.code == codes::LIST_LIMIT_CLAMPED));
    assert!(envelope
        .problems
        .iter()
        .any(|problem| problem.code == codes::LIST_CURSOR_INVALID));
}

#[test]
fn missed_store_envelope_clones_only_requested_page() {
    let now_ms = common::time::now_ms();
    let store = DashMap::new();
    for idx in 0..3 {
        let row = missed_row(format!("miss-{idx}"), now_ms - i64::from(idx) * 1_000);
        store.insert(row.id.clone(), row);
    }

    let first = missed_envelope_from_store(&store, 1, &ReviewPageQuery::new(Some(1), None));
    let envelope = missed_envelope_from_store(
        &store,
        1,
        &ReviewPageQuery::new(Some(1), first.page.next_cursor),
    );

    assert_eq!(envelope.page.limit, 1);
    assert_eq!(envelope.page.start_offset, 1);
    assert_eq!(envelope.page.returned_count, 1);
    assert_eq!(envelope.page.total_rows, 3);
    assert!(envelope
        .page
        .next_cursor
        .as_deref()
        .is_some_and(|cursor| cursor.starts_with("rv1:2:review-")));
    assert_eq!(envelope.rows.len(), 1);
    assert_eq!(envelope.rows[0].id, "miss-1");
    assert_review_storage_health(
        &envelope,
        "storage:review_missed_opportunities",
        envelope.page.total_rows,
    );
}

#[test]
fn stale_snapshot_cursor_resets_to_first_page_with_typed_problem() {
    let query = ReviewPageQuery::new(Some(50), Some("rv1:50:review-stale".into()));

    let (offset, limit, status, problems) = page_query_parts(&query, "review-current");

    assert_eq!((offset, limit), (0, 50));
    assert_eq!(status, ListStatus::Degraded);
    assert!(problems.iter().any(|problem| {
        problem.code == codes::LIST_CURSOR_INVALID
            && problem.details.as_ref().is_some_and(|details| {
                details["currentSnapshotId"] == "review-current" && details["applied"] == 0
            })
    }));
}

#[test]
fn review_page_exposes_server_navigation_and_snapshot_budget() {
    let page = list_page(50, 100, 50, 1_000, "review-snapshot");

    assert_eq!(
        page.previous_cursor.as_deref(),
        Some("rv1:50:review-snapshot")
    );
    assert_eq!(page.next_cursor.as_deref(), Some("rv1:150:review-snapshot"));
    assert_eq!(page.last_cursor.as_deref(), Some("rv1:950:review-snapshot"));
    assert_eq!(page.snapshot_id.as_deref(), Some("review-snapshot"));
}

#[test]
fn executed_snapshot_changes_when_evidence_changes_without_value_change() {
    let mut venue_fill = review_domain::RealizedPnlRow {
        group_id: "group-1".into(),
        ..Default::default()
    };
    venue_fill
        .evidence
        .record_fill_confidence(shared_types::ExecutionFillConfidence::VenueFill);
    let mut order_query = venue_fill.clone();
    order_query.evidence = Default::default();
    order_query
        .evidence
        .record_fill_confidence(shared_types::ExecutionFillConfidence::OrderQuery);
    let venue_fill = BTreeMap::from([("group-1".into(), venue_fill)]);
    let order_query = BTreeMap::from([("group-1".into(), order_query)]);

    assert_ne!(
        executed_review_snapshot_id(&venue_fill),
        executed_review_snapshot_id(&order_query)
    );
}

#[test]
fn review_window_is_clamped_to_runtime_budget() {
    let mut problems = Vec::new();

    let days = review_window_days(u32::MAX, &mut problems);

    assert_eq!(days, REVIEW_MAX_DAYS);
    assert!(problems
        .iter()
        .any(|problem| problem.code == codes::LIST_FILTER_INVALID));
}

#[test]
fn executed_envelope_preserves_window_clamp_problem() {
    let envelope = executed_envelope(&[], &[], u32::MAX, &ReviewPageQuery::new(Some(50), None));

    assert_eq!(envelope.days, REVIEW_MAX_DAYS);
    assert!(envelope
        .problems
        .iter()
        .any(|problem| problem.code == codes::LIST_FILTER_INVALID));
}

#[test]
fn only_default_first_page_uses_runtime_projection() {
    assert!(ReviewPageQuery::default().is_default_first_page());
    assert!(
        ReviewPageQuery::new(Some(REVIEW_DEFAULT_LIMIT), Some(String::new()))
            .is_default_first_page()
    );
    assert!(!ReviewPageQuery::new(Some(10), None).is_default_first_page());
    assert!(
        !ReviewPageQuery::new(Some(REVIEW_DEFAULT_LIMIT), Some("rv1:50:snapshot".into()))
            .is_default_first_page()
    );
}
