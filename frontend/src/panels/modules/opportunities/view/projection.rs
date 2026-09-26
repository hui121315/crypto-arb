use super::*;
use crate::panels::modules::opportunity_runtime::{OpportunityQuoteProjection, OpportunityQuoteSnapshot};
use super::super::data::{OpportunityStore, SymbolSearch};

#[derive(Clone, Copy)]
pub(super) struct OpportunityProjection {
    pub(super) symbol_search_active: Memo<bool>,
    pub(super) rows: Memo<Vec<OpportunityRow>>,
    pub(super) effective_filter: Memo<super::super::data::OpportunityFilter>,
    pub(super) filtered_rows: Memo<Vec<OpportunityRow>>,
    pub(super) eligibility_summary: Memo<OpportunityEligibilitySummary>,
    pub(super) visible_rows: Memo<Vec<OpportunityRow>>,
    pub(super) quotes: Memo<OpportunityQuoteProjection<OpportunityRow>>,
    pub(super) quote_ready_ids: Memo<HashSet<String>>,
}

pub(super) fn opportunity_projection(
    filter: RwSignal<super::super::data::OpportunityFilter>,
    store: &OpportunityStore,
    search: &SymbolSearch,
    live_ready: Memo<bool>,
    search_ready: Memo<bool>,
    eligibility_filter: RwSignal<OpportunityEligibilityFilter>,
) -> OpportunityProjection {
    let (rows_signal, live_meta, live_page) = (store.rows, store.meta, store.page);
    let (search_rows_signal, search_meta_signal, search_page, query_current) =
        (search.rows, search.meta, search.page, search.query_current);
    let symbol_search_active = Memo::new(move |_| opportunity_symbol_search_active(&filter.get()));
    let quotes = Memo::new(move |_| {
        if symbol_search_active.get() {
            if !query_current.get() {
                return OpportunityQuoteProjection::default();
            }
            merge_symbol_opportunity_rows(
                OpportunityQuoteSnapshot {
                    rows: &rows_signal.get(), meta: &live_meta.get(), page: live_page.get().as_ref(),
                },
                OpportunityQuoteSnapshot {
                    rows: &search_rows_signal.get(), meta: &search_meta_signal.get(), page: search_page.get().as_ref(),
                },
            )
        } else {
            OpportunityQuoteProjection { rows: rows_signal.get(), ..Default::default() }
        }
    });
    let rows = Memo::new(move |_| quotes.get().rows);
    let quote_ready_ids = Memo::new(move |_| {
        let searched = symbol_search_active.get();
        let (search_ready, live_ready) = (search_ready.get(), live_ready.get());
        quotes.with(|quotes| quotes.rows.iter().filter(|row| {
            if searched {
                search_ready && (!quotes.live_ids.contains(&row.id) || live_ready)
            } else {
                live_ready
            }
        }).map(|row| row.id.clone()).collect::<HashSet<_>>())
    });
    let effective_filter = Memo::new(move |_| {
        let mut active = filter.get();
        if symbol_search_active.get() {
            active.query.clear();
        }
        active
    });
    let filtered_rows = Memo::new(move |_| filter_rows(&rows.get(), &effective_filter.get()));
    let eligibility_summary = Memo::new(move |_| {
        let ready = quote_ready_ids.get();
        filtered_rows.with(|rows| {
            OpportunityEligibilitySummary::from_rows_with_readiness(rows.iter().map(|row| (row.as_ref(), ready.contains(&row.id))))
        })
    });
    let visible_rows = Memo::new(move |_| {
        let eligibility = eligibility_filter.get();
        let ready = quote_ready_ids.get();
        filtered_rows.with(|rows| {
            rows.iter()
                .filter(|row| eligibility.matches_with_readiness(row.as_ref(), ready.contains(&row.id)))
                .cloned()
                .collect::<Vec<_>>()
        })
    });

    OpportunityProjection {
        symbol_search_active,
        rows,
        effective_filter,
        filtered_rows,
        eligibility_summary,
        visible_rows,
        quotes,
        quote_ready_ids,
    }
}
