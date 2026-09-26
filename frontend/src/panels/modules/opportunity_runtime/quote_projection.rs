use super::OpportunityCountMeta;
use shared_types::{OpportunityEnvelopeStatus, OpportunityListPage};
use std::collections::HashSet;

pub(crate) struct OpportunityQuoteSnapshot<'a, Row> {
    pub rows: &'a [Row],
    pub meta: &'a OpportunityCountMeta,
    pub page: Option<&'a OpportunityListPage>,
}

#[derive(Clone, PartialEq)]
pub(crate) struct OpportunityQuoteProjection<Row> {
    pub rows: Vec<Row>,
    pub live_ids: HashSet<String>,
    pub complete_live: bool,
}

impl<Row> Default for OpportunityQuoteProjection<Row> {
    fn default() -> Self {
        Self { rows: Vec::new(), live_ids: HashSet::new(), complete_live: false }
    }
}

pub(crate) fn merge_symbol_quote_projections<Row: Clone>(
    live: OpportunityQuoteSnapshot<'_, Row>,
    search: OpportunityQuoteSnapshot<'_, Row>,
    id: impl Fn(&Row) -> &str,
    pair: impl Fn(&Row) -> &str,
    compare: impl Fn(&Row, &Row) -> std::cmp::Ordering,
) -> OpportunityQuoteProjection<Row> {
    let Some(symbol) = search.meta.filter_symbol.as_deref() else {
        return OpportunityQuoteProjection::default();
    };
    let mut rows = search.rows.iter()
        .filter(|row| pair(row).eq_ignore_ascii_case(symbol))
        .cloned().collect::<Vec<_>>();
    let can_overlay = live.page.is_some_and(|page| page.start_offset == 0)
        && search.page.is_some_and(|page| page.start_offset == 0)
        && live.meta.cached_at.zip(search.meta.cached_at).is_some_and(|(live_at, search_at)| {
            (live_at, live.meta.observed_at_ms) >= (search_at, search.meta.observed_at_ms)
        });
    let complete_live = can_overlay
        && !live.meta.rows_retained
        && live.meta.status == OpportunityEnvelopeStatus::Fresh
        && live.meta.error.is_none()
        && live.meta.partial_failures.is_empty()
        && full_window(&live)
        && full_window(&search);
    let mut live_ids = HashSet::new();
    if complete_live {
        rows = live.rows.iter().filter(|row| pair(row).eq_ignore_ascii_case(symbol)).cloned().collect();
        live_ids.extend(rows.iter().map(|row| id(row).to_owned()));
    } else if can_overlay {
        // A first-page eviction is not a market deletion. Preserve the bound search page.
        for row in &mut rows {
            if let Some(latest) = live.rows.iter().find(|candidate| {
                id(candidate) == id(row) && pair(candidate).eq_ignore_ascii_case(symbol)
            }) {
                *row = latest.clone();
                live_ids.insert(id(row).to_owned());
            }
        }
    }
    let rows = super::merge_opportunity_projections(&rows, &[], |left, right| id(left) == id(right), compare);
    OpportunityQuoteProjection { rows, live_ids, complete_live }
}

fn full_window<Row>(snapshot: &OpportunityQuoteSnapshot<'_, Row>) -> bool {
    snapshot.page.is_some_and(|page| {
        !page.has_next_page
            && page.returned_count == snapshot.rows.len()
            && page.total_rows == snapshot.rows.len()
    })
}
