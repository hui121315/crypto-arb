use super::super::paging::snapshot_id;
use super::*;

#[test]
fn zero_page_size_reports_minimum_clamp() {
    let window = OpportunityListWindow::from_query(Some(0), None, Some("score"));
    let problems = window.query_problems();

    assert_eq!(window.page_size(), 1);
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].code, codes::LIST_LIMIT_CLAMPED);
    assert_eq!(
        problems[0]
            .details
            .as_ref()
            .and_then(|details| details["requested"].as_u64()),
        Some(0)
    );
    assert_eq!(
        problems[0]
            .details
            .as_ref()
            .and_then(|details| details["applied"].as_u64()),
        Some(1)
    );
}

#[test]
fn bound_cursor_survives_snapshot_refresh_but_restarts_for_query_changes(
) -> Result<(), &'static str> {
    let cached_at = Utc::now();
    let meta = OpportunityScanMeta {
        candidate_count: 250,
        emitted_count: 250,
        ..OpportunityScanMeta::default()
    };
    let scope = list_cursor_scope("scope=main_p0;symbol=*");
    let first = OpportunityListWindow::from_bound_query(Some(120), None, Some("score"), &scope);
    let page = first.page(250, 120, snapshot_id(cached_at, &meta));
    let cursor = page.next_cursor.ok_or("first page should expose cursor")?;

    assert!(cursor.starts_with("v1:120:"));
    assert_eq!(
        OpportunityListWindow::from_bound_query(Some(120), Some(&cursor), Some("score"), &scope)
            .offset(),
        120
    );

    let changed_filter = list_cursor_scope("scope=main_p0;symbol=MU");
    assert_eq!(
        OpportunityListWindow::from_bound_query(
            Some(120),
            Some(&cursor),
            Some("score"),
            &changed_filter
        )
        .offset(),
        0
    );

    let changed_meta = OpportunityScanMeta {
        emitted_count: 251,
        ..meta
    };
    let changed_snapshot = snapshot_id(cached_at + chrono::Duration::seconds(1), &changed_meta);
    let refreshed_scope = list_cursor_scope("scope=main_p0;symbol=*");
    assert_eq!(
        OpportunityListWindow::from_bound_query(
            Some(120),
            Some(&cursor),
            Some("score"),
            &refreshed_scope
        )
        .offset(),
        120
    );
    assert_ne!(changed_snapshot, page.snapshot_id);
    assert_eq!(
        OpportunityListWindow::from_bound_query(
            Some(120),
            Some(&cursor),
            Some("settlement"),
            &scope
        )
        .offset(),
        0
    );
    Ok(())
}

#[test]
fn bound_cursor_clamps_to_the_last_available_page_when_rows_shrink() -> Result<(), &'static str> {
    let scope = list_cursor_scope("scope=main_p0;symbol=*");
    let first = OpportunityListWindow::from_bound_query(Some(50), None, Some("score"), &scope);
    let cursor = first
        .page(180, 50, "snapshot-a".into())
        .last_cursor
        .ok_or("first page should expose last cursor")?;

    let window =
        OpportunityListWindow::from_bound_query(Some(50), Some(&cursor), Some("score"), &scope)
            .clamped_to_total(71);

    assert_eq!(window.offset(), 50);
    Ok(())
}

#[test]
fn opportunity_server_cursor_navigation_signs_previous_and_last_windows() -> Result<(), &'static str>
{
    let cached_at = Utc::now();
    let meta = OpportunityScanMeta {
        candidate_count: 250,
        emitted_count: 250,
        ..OpportunityScanMeta::default()
    };
    let scope = list_cursor_scope("scope=main_p0;symbol=*");
    let first = OpportunityListWindow::from_bound_query(Some(120), None, Some("score"), &scope);
    let first_page = first.page(250, 120, snapshot_id(cached_at, &meta));
    let next = first_page
        .next_cursor
        .as_deref()
        .ok_or("first page should expose next cursor")?;

    assert!(first_page.previous_cursor.is_none());
    assert!(first_page
        .last_cursor
        .as_deref()
        .is_some_and(|cursor| cursor.starts_with("v1:240:")));

    let second =
        OpportunityListWindow::from_bound_query(Some(120), Some(next), Some("score"), &scope);
    let second_page = second.page(250, 120, snapshot_id(cached_at, &meta));
    assert!(second_page
        .previous_cursor
        .as_deref()
        .is_some_and(|cursor| cursor.starts_with("v1:0:")));
    assert!(second_page
        .last_cursor
        .as_deref()
        .is_some_and(|cursor| cursor.starts_with("v1:240:")));
    Ok(())
}
