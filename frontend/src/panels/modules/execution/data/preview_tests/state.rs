use super::*;

#[test]
fn execution_preview_context_does_not_reuse_ready_value_for_changed_query() {
    let query_a = preview_query_fixture();
    let mut query_b = query_a.clone();
    query_b.input.leverage = 3.0;
    let ready = from_api_preview(preview_response(), &query_a.seed, &query_a.input);
    let last_ready = Some(ReadyPreview::new(&query_a, ready));

    let state = preview_problem_state(
        &last_ready,
        &query_b,
        ApiProblem::new("PREVIEW_FAILED", "query B failed"),
    );

    assert!(matches!(state, LoadState::Error(_)));
}

#[test]
fn execution_preview_context_retains_stale_value_for_same_query() {
    let query = preview_query_fixture();
    let ready = from_api_preview(preview_response(), &query.seed, &query.input);
    let last_ready = Some(ReadyPreview::new(&query, ready));

    let state = preview_problem_state(
        &last_ready,
        &query,
        ApiProblem::new("PREVIEW_FAILED", "same query failed"),
    );

    assert!(matches!(
        state,
        LoadState::Stale { value, .. } if value.opportunity_id == "opp-1"
    ));
}

#[test]
fn execution_preview_request_is_cold_without_matching_ready_value() {
    let query = preview_query_fixture();

    let state = preview_request_state(&None, &query);

    assert!(matches!(state, LoadState::Loading));
}

#[test]
fn execution_preview_refresh_keeps_same_query_as_explicit_stale() {
    let query = preview_query_fixture();
    let ready = from_api_preview(preview_response(), &query.seed, &query.input);
    let last_ready = Some(ReadyPreview::new(&query, ready));

    let state = preview_request_state(&last_ready, &query);

    assert!(matches!(
        state,
        LoadState::Stale { problem, .. }
            if problem.code == "PREVIEW_REFRESHING"
                && problem.source.as_deref() == Some("frontend-preview")
    ));
}

#[test]
fn execution_preview_context_rejects_old_request_version_writeback() {
    assert!(preview_request_is_current(2, 2));
    assert!(!preview_request_is_current(2, 1));
}

#[test]
fn execution_preview_deduplicates_identical_in_flight_query() {
    let query = preview_query_fixture();

    assert!(preview_request_is_in_flight(&Some(query.clone()), &query));

    let mut changed = query.clone();
    changed.seed.opportunity_snapshot_id = "snapshot-2".into();
    assert!(!preview_request_is_in_flight(&Some(query), &changed));
}
