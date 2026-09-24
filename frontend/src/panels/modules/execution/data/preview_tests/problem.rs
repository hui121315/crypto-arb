use super::*;

#[test]
fn execution_preview_problem_context_build_error() {
    let preview = failed_preview(
        &PreviewSeed::from_selection(&ExecutionSelection::empty()),
        &preview_input(),
        &ApiProblem::new("RATE_LIMITED", "slow")
            .with_source("preview-rest")
            .with_status(429)
            .with_request_id(Some("req-1".into()))
            .with_retry_after_ms(Some(2_000)),
    );

    assert!(!preview.can_submit());
    assert_eq!(preview.source, "预检错误");
    assert!(preview.risk.note.contains("code RATE_LIMITED"));
    assert!(preview.risk.note.contains("source preview-rest"));
    assert!(preview.risk.note.contains("HTTP 429"));
    assert!(preview.risk.note.contains("request_id req-1"));
    assert!(preview.risk.note.contains("retry 2000ms"));
    assert!(preview.risk.blockers[0].contains("code RATE_LIMITED"));
    assert!(preview.risk.blockers[0].contains("source preview-rest"));
}

#[test]
fn execution_preview_problem_context_build_stale() {
    let mut preview = pending_preview(
        &PreviewSeed::from_selection(&ExecutionSelection::empty()),
        &preview_input(),
    );
    preview.readiness = PreviewReadiness::Ready;
    preview.idempotency_key = Some("idem-1".into());
    preview.long_allowed = true;
    preview.short_allowed = true;
    preview.risk.blockers.clear();

    let stale = stale_preview(
        preview,
        &ApiProblem::new("TIMEOUT", "preview timeout")
            .with_source("preview-cache")
            .with_status(504)
            .with_request_id(Some("req-stale-preview".into()))
            .with_retry_after_ms(Some(3_000)),
    );

    assert!(!stale.can_submit());
    assert_eq!(stale.source, "预检失效");
    assert_eq!(stale.readiness, PreviewReadiness::Stale);
    assert!(stale.risk.note.contains("preview timeout"));
    assert!(stale.risk.note.contains("code TIMEOUT"));
    assert!(stale.risk.note.contains("source preview-cache"));
    assert!(stale.risk.note.contains("HTTP 504"));
    assert!(stale.risk.note.contains("request_id req-stale-preview"));
    assert!(stale.risk.note.contains("retry 3000ms"));
    assert!(stale.risk.blockers[0].contains("code TIMEOUT"));
}

#[test]
fn stale_opportunity_snapshot_refreshes_the_same_selection_once() {
    Owner::new().with(|| {
        let query = preview_query_fixture();
        let mut selected = ExecutionSelection::empty();
        selected.opportunity_id = query.seed.opportunity_id.clone();
        selected.opportunity_snapshot_id = query.seed.opportunity_snapshot_id.clone();
        let selection = RwSignal::new(selected);
        let mut problem = ApiProblem::new(
            codes::OPPORTUNITY_SNAPSHOT_STALE,
            "opportunity list snapshot changed before hedge preview",
        );
        problem.details = Some(serde_json::json!({ "actualSnapshotId": "snapshot-2" }));

        assert!(refresh_stale_selection_snapshot(
            selection, &query, &problem
        ));
        assert_eq!(
            selection.get_untracked().opportunity_snapshot_id,
            "snapshot-2"
        );
        assert!(!refresh_stale_selection_snapshot(
            selection, &query, &problem
        ));
    });
}

#[test]
fn snapshot_recovery_is_bounded_and_manual_refresh_starts_a_new_budget() {
    Owner::new().with(|| {
        let mut query = preview_query_fixture();
        let mut selected = ExecutionSelection::empty();
        selected.opportunity_id = query.seed.opportunity_id.clone();
        selected.opportunity_snapshot_id = query.seed.opportunity_snapshot_id.clone();
        let selection = RwSignal::new(selected);
        let mut budget = SnapshotRefreshBudget::default();
        for attempt in 1..=3 {
            let mut problem = ApiProblem::new(codes::OPPORTUNITY_SNAPSHOT_STALE, "changed");
            problem.details = Some(serde_json::json!({ "actualSnapshotId": format!("next-{attempt}") }));
            assert_eq!(budget.refresh(selection, &query, 0, &mut problem), attempt <= 2);
            if attempt == 3 {
                assert!(problem.message.contains("停止自动重试"));
                assert!(budget.refresh(selection, &query, 1, &mut problem));
            }
            query.seed.opportunity_snapshot_id = selection.get_untracked().opportunity_snapshot_id;
        }
        let mut late = ApiProblem::new(codes::OPPORTUNITY_SNAPSHOT_STALE, "late");
        late.details = Some(serde_json::json!({ "actualSnapshotId": "old-response" }));
        query.seed.opportunity_snapshot_id = "snapshot-1".into();
        assert!(!refresh_stale_selection_snapshot(selection, &query, &late));
        assert_eq!(selection.get_untracked().opportunity_snapshot_id, "next-3");
    });
}
