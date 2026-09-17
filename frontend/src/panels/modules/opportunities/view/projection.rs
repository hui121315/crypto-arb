use super::*;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;

#[derive(Clone, Copy)]
pub(super) struct OpportunityProjection {
    pub(super) symbol_search_active: Memo<bool>,
    pub(super) rows: Memo<Vec<OpportunityRow>>,
    pub(super) effective_filter: Memo<super::super::data::OpportunityFilter>,
    pub(super) filtered_rows: Memo<Vec<OpportunityRow>>,
    pub(super) eligibility_summary: Memo<OpportunityEligibilitySummary>,
    pub(super) visible_rows: Memo<Vec<OpportunityRow>>,
}

pub(super) fn opportunity_projection(
    filter: RwSignal<super::super::data::OpportunityFilter>,
    rows_signal: RwSignal<Vec<OpportunityRow>>,
    search_rows_signal: RwSignal<Vec<OpportunityRow>>,
    search_meta_signal: RwSignal<OpportunityCountMeta>,
    eligibility_filter: RwSignal<OpportunityEligibilityFilter>,
) -> OpportunityProjection {
    let symbol_search_active = Memo::new(move |_| opportunity_symbol_search_active(&filter.get()));
    let rows = Memo::new(move |_| {
        if symbol_search_active.get() {
            let canonical_symbol = search_meta_signal.get().filter_symbol;
            merge_symbol_opportunity_rows(
                &rows_signal.get(),
                &search_rows_signal.get(),
                canonical_symbol.as_deref(),
            )
        } else {
            rows_signal.get()
        }
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
        filtered_rows.with(|rows| {
            OpportunityEligibilitySummary::from_rows(rows.iter().map(|row| row.as_ref()))
        })
    });
    let visible_rows = Memo::new(move |_| {
        let eligibility = eligibility_filter.get();
        filtered_rows.with(|rows| {
            rows.iter()
                .filter(|row| eligibility.matches(row.as_ref()))
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
    }
}
