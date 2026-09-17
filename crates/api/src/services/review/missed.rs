use super::*;
use std::cmp::Reverse;

#[cfg(test)]
pub(crate) fn missed_from_rows(rows: &[MissedOpportunity], days: u32) -> Vec<MissedOpportunity> {
    let mut ignored_problems = Vec::new();
    let days = review_window_days(days, &mut ignored_problems);
    let mut rows = review_domain::filter_recent_missed(rows, common::time::now_ms(), days);
    rows.sort_by_key(|row| Reverse(row.detected_at_ms));
    rows
}

#[cfg(test)]
pub(crate) fn missed_envelope(
    rows: &[MissedOpportunity],
    days: u32,
    page_query: &ReviewPageQuery,
) -> ReviewEnvelope<MissedOpportunity> {
    let now_ms = common::time::now_ms();
    let mut window_problems = Vec::new();
    let days = review_window_days(days, &mut window_problems);
    let rows = missed_from_rows(rows, days);
    let snapshot_id = missed_review_snapshot_id(&rows);
    let (rows, page, mut status, mut problems) = page_rows(rows, page_query, &snapshot_id);
    problems.extend(window_problems);
    if !problems.is_empty() {
        status = ListStatus::Degraded;
    }
    let total_rows = page.total_rows;
    with_review_storage_health(
        ReviewEnvelope::new(
            rows,
            now_ms,
            days,
            ReviewDataSource::MissedOpportunityStore,
            None,
            Vec::new(),
        )
        .with_page(page, status, problems),
        review_storage_health(ReviewDataSource::MissedOpportunityStore, total_rows, now_ms),
    )
}

pub(crate) fn missed_envelope_from_store(
    store: &DashMap<String, MissedOpportunity>,
    days: u32,
    page_query: &ReviewPageQuery,
) -> ReviewEnvelope<MissedOpportunity> {
    let now_ms = common::time::now_ms();
    let mut window_problems = Vec::new();
    let days = review_window_days(days, &mut window_problems);
    let from_ms = min_window_ms(now_ms, days);
    let mut keys = store
        .iter()
        .filter(|entry| entry.value().detected_at_ms >= from_ms)
        .map(|entry| (Reverse(entry.value().detected_at_ms), entry.key().clone()))
        .collect::<Vec<_>>();
    keys.sort_unstable();

    let total_rows = keys.len();
    let snapshot_id = review_snapshot_id(
        "missed",
        keys.iter()
            .map(|(Reverse(detected_at_ms), id)| format!("{id}:{detected_at_ms}")),
    );
    let (offset, limit, mut status, mut problems) = page_query_parts(page_query, &snapshot_id);
    problems.extend(window_problems);
    if !problems.is_empty() {
        status = ListStatus::Degraded;
    }
    let rows = keys
        .into_iter()
        .skip(offset)
        .take(limit)
        .filter_map(|(_, id)| store.get(&id).map(|entry| entry.value().clone()))
        .collect::<Vec<_>>();
    let page = list_page(limit, offset, rows.len(), total_rows, &snapshot_id);
    with_review_storage_health(
        ReviewEnvelope::new(
            rows,
            now_ms,
            days,
            ReviewDataSource::MissedOpportunityStore,
            None,
            Vec::new(),
        )
        .with_page(page, status, problems),
        review_storage_health(ReviewDataSource::MissedOpportunityStore, total_rows, now_ms),
    )
}

#[cfg(test)]
fn missed_review_snapshot_id(rows: &[MissedOpportunity]) -> String {
    review_snapshot_id(
        "missed",
        rows.iter()
            .map(|row| format!("{}:{}", row.id, row.detected_at_ms)),
    )
}
