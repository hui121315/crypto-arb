use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OpportunityListWindow {
    offset: usize,
    requested_page_size: Option<usize>,
    page_size: usize,
    sort_key: OpportunityListSortKey,
    cursor_scope_hash: Option<u64>,
}
impl OpportunityListWindow {
    #[cfg(test)]
    pub(crate) fn from_query(
        page_size: Option<usize>,
        cursor: Option<&str>,
        sort_key: Option<&str>,
    ) -> Self {
        Self {
            offset: cursor.and_then(parse_cursor).unwrap_or(0),
            requested_page_size: page_size,
            page_size: normalize_page_size(page_size),
            sort_key: parse_sort_key(sort_key),
            cursor_scope_hash: None,
        }
    }

    pub(crate) fn from_bound_query(
        page_size: Option<usize>,
        cursor: Option<&str>,
        sort_key: Option<&str>,
        cursor_scope: &str,
    ) -> Self {
        let requested_page_size = page_size;
        let page_size = normalize_page_size(page_size);
        let sort_key = parse_sort_key(sort_key);
        let cursor_scope_hash = cursor_scope_hash(cursor_scope, page_size, sort_key);
        Self {
            offset: cursor
                .and_then(|value| parse_bound_cursor(value, cursor_scope_hash))
                .unwrap_or(0),
            requested_page_size,
            page_size,
            sort_key,
            cursor_scope_hash: Some(cursor_scope_hash),
        }
    }

    pub(crate) const fn offset(self) -> usize {
        self.offset
    }

    pub(crate) const fn page_size(self) -> usize {
        self.page_size
    }

    pub(crate) const fn requested_page_size(self) -> Option<usize> {
        self.requested_page_size
    }

    pub(crate) const fn max_page_size(self) -> usize {
        MAX_LIST_PAGE_SIZE
    }

    pub(crate) const fn sort_key(self) -> OpportunityListSortKey {
        self.sort_key
    }

    pub(crate) fn clamped_to_total(mut self, total_rows: usize) -> Self {
        let last_offset = total_rows
            .saturating_sub(1)
            .checked_div(self.page_size.max(1))
            .unwrap_or(0)
            .saturating_mul(self.page_size);
        self.offset = self.offset.min(last_offset);
        self
    }

    pub(crate) fn query_problems(self) -> Vec<ApiProblem> {
        page_size_problem(self.requested_page_size, self.page_size)
            .into_iter()
            .collect()
    }

    pub(super) fn page(
        self,
        total_rows: usize,
        returned_count: usize,
        snapshot_id: String,
    ) -> OpportunityListPage {
        let next_offset = self.offset.saturating_add(returned_count);
        let has_next_page = next_offset < total_rows;
        let previous_offset = self.offset.saturating_sub(self.page_size);
        let last_offset = total_rows
            .saturating_sub(1)
            .checked_div(self.page_size.max(1))
            .unwrap_or(0)
            .saturating_mul(self.page_size);
        OpportunityListPage {
            page_size: self.page_size,
            start_offset: self.offset,
            returned_count,
            total_rows,
            has_next_page,
            next_cursor: has_next_page.then(|| self.cursor(next_offset)),
            previous_cursor: (self.offset > 0).then(|| self.cursor(previous_offset)),
            last_cursor: (last_offset > self.offset).then(|| self.cursor(last_offset)),
            sort_key: self.sort_key,
            snapshot_id,
        }
    }

    fn cursor(self, offset: usize) -> String {
        self.cursor_scope_hash.map_or_else(
            || offset.to_string(),
            |hash| format!("v1:{offset}:{hash:016x}"),
        )
    }
}
pub(crate) struct OpportunityWideLimit {
    value: usize,
    problem: Option<ApiProblem>,
}

impl OpportunityWideLimit {
    pub(crate) const fn value(&self) -> usize {
        self.value
    }

    pub(crate) fn into_query_problems(self) -> Vec<ApiProblem> {
        self.problem.into_iter().collect()
    }
}
pub(crate) fn page_refs<'a>(
    rows: &[&'a ArbitrageOpportunityDto],
    window: OpportunityListWindow,
) -> Vec<&'a ArbitrageOpportunityDto> {
    let start = window.offset().min(rows.len());
    let end = start.saturating_add(window.page_size()).min(rows.len());
    rows[start..end].to_vec()
}
pub(crate) fn snapshot_id(cached_at: DateTime<Utc>, meta: &OpportunityScanMeta) -> String {
    format!(
        "{}:{}:{}",
        cached_at.timestamp_millis(),
        meta.candidate_count,
        meta.emitted_count
    )
}

fn normalize_page_size(page_size: Option<usize>) -> usize {
    page_size
        .unwrap_or(DEFAULT_LIST_PAGE_SIZE)
        .clamp(1, MAX_LIST_PAGE_SIZE)
}

fn page_size_problem(requested: Option<usize>, applied: usize) -> Option<ApiProblem> {
    let requested = requested.filter(|requested| *requested != applied)?;
    let message = if requested == 0 {
        "opportunity list page size was raised to minimum"
    } else {
        "opportunity list page size was clamped to maximum"
    };
    let mut problem =
        ApiProblem::new(codes::LIST_LIMIT_CLAMPED, message).with_source(WIDE_LIST_SOURCE);
    problem.details = Some(serde_json::json!({
        "field": "pageSize",
        "requested": requested,
        "applied": applied,
        "maxPageSize": MAX_LIST_PAGE_SIZE,
    }));
    Some(problem)
}
pub(crate) fn wide_limit(requested: Option<usize>) -> OpportunityWideLimit {
    match requested {
        None => OpportunityWideLimit {
            value: DEFAULT_WIDE_LIMIT,
            problem: None,
        },
        Some(0) => OpportunityWideLimit {
            value: 1,
            problem: Some(wide_limit_problem(
                "opportunity list limit was raised to minimum",
                serde_json::json!({ "requested": 0, "applied": 1 }),
            )),
        },
        Some(value) if value > MAX_WIDE_LIMIT => OpportunityWideLimit {
            value: MAX_WIDE_LIMIT,
            problem: Some(wide_limit_problem(
                "opportunity list limit was clamped to maximum",
                serde_json::json!({
                    "requested": value,
                    "applied": MAX_WIDE_LIMIT,
                    "maxLimit": MAX_WIDE_LIMIT,
                }),
            )),
        },
        Some(value) => OpportunityWideLimit {
            value,
            problem: None,
        },
    }
}

fn wide_limit_problem(message: &'static str, details: serde_json::Value) -> ApiProblem {
    let mut problem =
        ApiProblem::new(codes::LIST_LIMIT_CLAMPED, message).with_source(WIDE_LIST_SOURCE);
    problem.details = Some(details);
    problem
}
fn parse_cursor(value: &str) -> Option<usize> {
    let value = value.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok()
}
fn parse_bound_cursor(value: &str, expected_hash: u64) -> Option<usize> {
    let value = value.trim();
    let ("v1", rest) = value.split_once(':')? else {
        return None;
    };
    let (offset, hash) = rest.split_once(':')?;
    let hash = u64::from_str_radix(hash, 16).ok()?;
    (hash == expected_hash).then(|| parse_cursor(offset))?
}
pub(crate) fn list_cursor_scope(filter_key: &str) -> String {
    format!("filter={filter_key}")
}
fn cursor_scope_hash(
    cursor_scope: &str,
    page_size: usize,
    sort_key: OpportunityListSortKey,
) -> u64 {
    let seed = format!("{cursor_scope};pageSize={page_size};sortKey={sort_key:?}");
    stable_hash(&seed)
}
fn stable_hash(value: &str) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in value.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}
fn parse_sort_key(value: Option<&str>) -> OpportunityListSortKey {
    match value.map(str::trim) {
        Some("settlement") => OpportunityListSortKey::Settlement,
        Some("net_single_yield") | Some("netYield") | Some("net_yield") => {
            OpportunityListSortKey::NetSingleYield
        }
        // `score` remains a legacy query alias, but no longer restores score-based ordering.
        _ => OpportunityListSortKey::NetSingleYield,
    }
}
