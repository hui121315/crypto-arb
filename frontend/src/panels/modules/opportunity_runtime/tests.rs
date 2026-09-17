use super::*;
use leptos::prelude::Owner;

#[test]
fn pr_dx_shared_opportunity_runtime_locks_list_and_search_state_contract() {
    Owner::new().with(|| {
        let opportunity_list = OpportunityListRuntime::<u8>::new();
        let futures_list = OpportunityListRuntime::<String>::new();
        let opportunity_search = OpportunitySearchRuntime::<u8>::new();
        let futures_search = OpportunitySearchRuntime::<String>::new();

        assert!(matches!(
            opportunity_list.state.get_untracked(),
            LoadState::Loading
        ));
        assert!(matches!(
            futures_list.state.get_untracked(),
            LoadState::Loading
        ));
        assert!(matches!(
            opportunity_search.state.get_untracked(),
            LoadState::Ready(())
        ));
        assert!(matches!(
            futures_search.state.get_untracked(),
            LoadState::Ready(())
        ));
        assert!(opportunity_list.rows.get_untracked().is_empty());
        assert!(futures_list.rows.get_untracked().is_empty());
    });
}

#[test]
fn opportunity_runtime_uses_rest_only_for_an_explicit_cursor_page() {
    assert_eq!(
        clean_opportunity_cursor(Some("  v1:25:scope  ".into())).as_deref(),
        Some("v1:25:scope")
    );
    assert_eq!(clean_opportunity_cursor(Some("  ".into())), None);
    assert!(!opportunity_page_request_required(None));
    assert!(!opportunity_page_request_required(Some("  ")));
    assert!(opportunity_page_request_required(Some("bound:50")));
}

#[test]
fn live_first_page_selects_the_exact_strategy_window() -> Result<(), &'static str> {
    let cached_at = chrono::Utc::now();
    let page = OpportunityListPage {
        page_size: 50,
        start_offset: 0,
        returned_count: 0,
        total_rows: 12,
        has_next_page: true,
        next_cursor: Some("bound:50".into()),
        previous_cursor: None,
        last_cursor: Some("bound:50".into()),
        sort_key: shared_types::OpportunityListSortKey::NetSingleYield,
        snapshot_id: "snap-live".into(),
    };
    let scope_meta = shared_types::OpportunityQueryScopeMeta {
        global_total_count: 80,
        strategy_scope_count: 12,
        symbol_scope_count: 12,
        filtered_count: 12,
        page_count: 1,
        candidate_count: 100,
        emitted_count: 80,
    };
    let event = OpportunityStreamEvent {
        event: shared_types::OpportunityStreamEventKind::SnapshotInvalidated,
        snapshot_id: "snap-live".into(),
        scope_meta: scope_meta.clone(),
        changed_ids: Vec::new(),
        changed_rows: Vec::new(),
        removed_ids: Vec::new(),
        top_ids: Vec::new(),
        windows: vec![shared_types::OpportunityStreamWindow {
            strategy_kind: Some(StrategyKind::SpotCross),
            ids: Vec::new(),
            page: page.clone(),
            scope_meta,
            query_key: "scope=main_p0;strategy=spot_cross".into(),
        }],
        main_p0_counts: shared_types::OpportunityCountBreakdown::default(),
        registry_counts: shared_types::OpportunityCountBreakdown::default(),
        meta: shared_types::OpportunityScanMeta::default(),
        status: shared_types::OpportunityEnvelopeStatus::Fresh,
        scope: shared_types::OpportunityEnvelopeScope::MainP0,
        query_key: "scope=main_p0".into(),
        source: "snapshot".into(),
        cached_at,
        observed_at_ms: cached_at.timestamp_millis(),
        freshness_ms: Some(10),
        retry_after_ms: None,
        error: None,
        partial_failures: Vec::new(),
    };

    let live_rows = std::collections::HashMap::new();
    let envelope = live_first_page_from_stream(&event, &live_rows, Some(StrategyKind::SpotCross))
        .ok_or("spot-cross live first page missing")?;

    assert_eq!(envelope.page, page);
    assert_eq!(envelope.scope_meta.filtered_count, 12);
    assert_eq!(
        envelope.request_meta.filter.strategy_kinds,
        [StrategyKind::SpotCross]
    );
    assert!(
        live_first_page_from_stream(&event, &live_rows, Some(StrategyKind::PerpCross),).is_none()
    );
    Ok(())
}

#[test]
fn pr_dx_search_merge_is_production_shared_deduplicated_and_ranked() {
    let base = vec![("one", 70_u8), ("two", 60_u8)];
    let extra = vec![("two", 99_u8), ("three", 80_u8)];

    let merged = merge_opportunity_projections(
        &base,
        &extra,
        |left, right| left.0 == right.0,
        |left, right| right.1.cmp(&left.1),
    );

    assert_eq!(merged, vec![("three", 80), ("one", 70), ("two", 60)]);
}
