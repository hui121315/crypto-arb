use super::*;

#[cfg(test)]
pub(super) fn page_rows<T>(
    rows: Vec<T>,
    query: &ReviewPageQuery,
    snapshot_id: &str,
) -> (Vec<T>, ListPage, ListStatus, Vec<ApiProblem>) {
    let total_rows = rows.len();
    let (offset, limit, status, problems) = page_query_parts(query, snapshot_id);
    let rows = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let page = list_page(limit, offset, rows.len(), total_rows, snapshot_id);
    (rows, page, status, problems)
}

pub(super) fn page_query_parts(
    query: &ReviewPageQuery,
    snapshot_id: &str,
) -> (usize, usize, ListStatus, Vec<ApiProblem>) {
    let mut problems = Vec::new();
    let limit = page_limit(query.limit, &mut problems);
    let offset = page_offset(query.cursor.as_deref(), snapshot_id, &mut problems);
    let status = if problems.is_empty() {
        ListStatus::Fresh
    } else {
        ListStatus::Degraded
    };
    (offset, limit, status, problems)
}

pub(super) fn list_page(
    limit: usize,
    offset: usize,
    returned_count: usize,
    total_rows: usize,
    snapshot_id: &str,
) -> ListPage {
    let next_offset = offset.saturating_add(returned_count);
    let has_more = next_offset < total_rows;
    let previous_offset = offset.saturating_sub(limit.max(1));
    let last_offset = total_rows.saturating_sub(1) / limit.max(1) * limit.max(1);
    ListPage {
        limit,
        max_limit: REVIEW_MAX_LIMIT,
        start_offset: offset,
        returned_count,
        total_rows,
        has_more,
        previous_cursor: (offset > 0).then(|| review_cursor(previous_offset, snapshot_id)),
        next_cursor: has_more.then(|| review_cursor(next_offset, snapshot_id)),
        last_cursor: (last_offset > offset).then(|| review_cursor(last_offset, snapshot_id)),
        snapshot_id: Some(snapshot_id.to_owned()),
    }
}

pub(super) fn min_window_ms(now_ms: i64, days: u32) -> i64 {
    now_ms.saturating_sub(i64::from(days.max(1)).saturating_mul(DAY_MS))
}

pub(super) fn review_window_days(days: u32, problems: &mut Vec<ApiProblem>) -> u32 {
    let applied = days.clamp(1, REVIEW_MAX_DAYS);
    if applied != days {
        problems.push(review_problem(
            codes::LIST_FILTER_INVALID,
            "review window days was clamped to runtime budget",
            serde_json::json!({ "requested": days, "applied": applied }),
        ));
    }
    applied
}

pub(super) fn review_snapshot_id(
    namespace: &str,
    parts: impl IntoIterator<Item = String>,
) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    hash_bytes(&mut hash, namespace.as_bytes());
    for part in parts {
        hash_bytes(&mut hash, &[0xff]);
        hash_bytes(&mut hash, part.as_bytes());
    }
    format!("review-{hash:016x}")
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn page_limit(requested: Option<usize>, problems: &mut Vec<ApiProblem>) -> usize {
    match requested {
        None => REVIEW_DEFAULT_LIMIT,
        Some(0) => {
            problems.push(review_problem(
                codes::LIST_LIMIT_CLAMPED,
                "review list limit was raised to minimum",
                serde_json::json!({ "requested": 0, "applied": 1 }),
            ));
            1
        }
        Some(limit) if limit > REVIEW_MAX_LIMIT => {
            problems.push(review_problem(
                codes::LIST_LIMIT_CLAMPED,
                "review list limit was clamped to maximum",
                serde_json::json!({
                    "requested": limit,
                    "applied": REVIEW_MAX_LIMIT,
                }),
            ));
            REVIEW_MAX_LIMIT
        }
        Some(limit) => limit,
    }
}

fn page_offset(cursor: Option<&str>, snapshot_id: &str, problems: &mut Vec<ApiProblem>) -> usize {
    let Some(cursor) = cursor.filter(|value| !value.trim().is_empty()) else {
        return 0;
    };
    if let Some(offset) = decode_review_cursor(cursor, snapshot_id) {
        return offset;
    }
    problems.push(review_problem(
        codes::LIST_CURSOR_INVALID,
        "review list cursor was invalid",
        serde_json::json!({
            "cursor": cursor,
            "currentSnapshotId": snapshot_id,
            "applied": 0,
        }),
    ));
    0
}

fn review_cursor(offset: usize, snapshot_id: &str) -> String {
    format!("rv1:{offset}:{snapshot_id}")
}

fn decode_review_cursor(cursor: &str, snapshot_id: &str) -> Option<usize> {
    let mut parts = cursor.splitn(3, ':');
    if parts.next()? != "rv1" {
        return None;
    }
    let offset = parts.next()?.parse::<usize>().ok()?;
    (parts.next()? == snapshot_id).then_some(offset)
}

pub(super) fn review_problem(
    code: &'static str,
    message: &'static str,
    details: serde_json::Value,
) -> ApiProblem {
    let mut problem = ApiProblem::new(code, message).with_source(REVIEW_SOURCE);
    problem.details = Some(details);
    problem
}
