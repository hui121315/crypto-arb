use super::*;
use crate::panels::modules::instrument_search::{
    is_venue_query, normalized_query, symbol_search_query,
};
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_view_model::OpportunityListViewModel;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::use_debounced_string;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{OpportunityListPage, StrategyKind};

#[derive(Clone)]
pub(in crate::panels::modules::opportunities) struct SymbolSearch {
    pub(in crate::panels::modules::opportunities) state: RwSignal<LoadState<()>>,
    pub(in crate::panels::modules::opportunities) rows: Memo<Vec<OpportunityRow>>,
    pub(in crate::panels::modules::opportunities) meta: RwSignal<OpportunityCountMeta>,
    pub(in crate::panels::modules::opportunities) page: RwSignal<Option<OpportunityListPage>>,
    pub(in crate::panels::modules::opportunities) load_cursor: Callback<Option<String>>,
    pub(in crate::panels::modules::opportunities) query_current: Memo<bool>,
}

pub(in crate::panels::modules::opportunities) fn use_symbol_opportunities(
    runtime: OpportunitiesRuntime,
) -> SymbolSearch {
    let filter = runtime.filter;
    let search = runtime.search;
    let state = search.state;
    let shared_rows = search.rows;
    let meta = search.meta;
    let page = search.page;
    let last_query = search.last_query;
    let cursor = search.cursor;
    let last_strategy = RwSignal::new(filter.get_untracked().strategy);
    let strategy = Memo::new(move |_| filter.get().strategy);
    let query = Memo::new(move |_| symbol_search_query(&filter.get().query).unwrap_or_default());
    let query_current = Memo::new(move |_| {
        query.get() == last_query.get() && strategy.get() == last_strategy.get()
    });
    let rows = Memo::new(move |_| {
        if query_current.get() {
            shared_rows.get()
        } else {
            Vec::new()
        }
    });
    let request_revision = RwSignal::new(0_u64);
    let refresh = RwSignal::new(0_u64);
    let debounced_query = use_debounced_string(move || query.get(), SEARCH_DEBOUNCE);
    let client = use_global().client;
    Effect::new(move |_| {
        let strategy = strategy.get();
        let query = query.get();
        let debounced = debounced_query.get();
        let _ = refresh.get();
        request_revision.update(|revision| *revision += 1);
        let revision = request_revision.get_untracked();
        let strategy_changed = strategy != last_strategy.get_untracked();
        if strategy_changed {
            last_strategy.set(strategy);
        }
        let cursor_value = cursor.get();
        if query != last_query.get_untracked() || strategy_changed {
            last_query.set(query.clone());
            shared_rows.set(Vec::new());
            meta.set(OpportunityCountMeta::default());
            page.set(None);
            if cursor_value.is_some() {
                state.set(LoadState::Loading);
                cursor.set(None);
                return;
            }
        }
        if query.is_empty() {
            shared_rows.set(Vec::new());
            meta.set(OpportunityCountMeta::default());
            page.set(None);
            state.set(LoadState::Ready(()));
            return;
        }
        state.set(LoadState::Loading);
        if query != debounced {
            return;
        }
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .scan_opportunity_list_scoped_page(
                    strategy,
                    Some(&query),
                    cursor_value.as_deref(),
                    OPPORTUNITY_PAGE_SIZE,
                )
                .await;
            if request_revision.try_get_untracked() != Some(revision)
                || symbol_search_query(&filter.get_untracked().query).as_deref()
                    != Some(query.as_str())
                || last_query.get_untracked() != query
                || cursor.get_untracked() != cursor_value
                || filter.get_untracked().strategy != strategy
            {
                return;
            }
            match result {
                Ok(latest) => {
                    apply_symbol_opportunities_success(&latest, state, shared_rows, meta, page);
                }
                Err(error) => {
                    apply_symbol_opportunities_error(error, state, &query, cursor_value.as_deref());
                }
            }
        });
    });
    let load_cursor = Callback::new(move |next| {
        state.set(LoadState::Loading);
        let next = clean_cursor(next);
        if next == cursor.get_untracked() {
            refresh.update(|value| *value += 1);
        } else {
            cursor.set(next);
        }
    });
    SymbolSearch {
        state,
        rows,
        meta,
        page,
        load_cursor,
        query_current,
    }
}

pub(in crate::panels::modules::opportunities) fn filter_rows(
    rows: &[OpportunityRow],
    filter: &OpportunityFilter,
) -> Vec<OpportunityRow> {
    let query = normalized_query(&filter.query);
    rows.iter()
        .filter(|row| matches_filter(row.as_ref(), filter, &query))
        .cloned()
        .collect()
}

pub(in crate::panels::modules::opportunities) fn summarize_rows(
    rows: &[OpportunityRow],
    filter: &OpportunityFilter,
    meta: &OpportunityCountMeta,
) -> OpportunitySummary {
    let query = normalized_query(&filter.query);
    let matching_strategy_count = rows
        .iter()
        .filter(|row| matches_strategy(row.as_ref(), filter.strategy))
        .count();
    let matching_executable_count = rows
        .iter()
        .filter(|row| matches_strategy(row.as_ref(), filter.strategy) && row.execution_eligible)
        .count();
    let candidate_count = meta.selected_count(filter.strategy, matching_strategy_count);
    let executable_count =
        meta.selected_executable_count(filter.strategy, matching_executable_count);
    let local_filter_active = !query.is_empty() || filter.min_net_pct > 0.0;
    let mut summary = OpportunitySummary {
        candidates: candidate_count,
        filtered_candidates: 0,
        executable_candidates: 0,
        best_executable_net_bps: None,
        best_executable_pair: "-".into(),
    };
    for row in rows {
        let row = row.as_ref();
        if !matches_filter(row, filter, &query) {
            continue;
        }
        summary.filtered_candidates += 1;
        if row.execution_eligible {
            summary.executable_candidates += 1;
            if row.cost_verified
                && row.one_cycle_net_bps.is_finite()
                && row.one_cycle_net_bps > 0.0
                && summary
                    .best_executable_net_bps
                    .is_none_or(|best| row.one_cycle_net_bps > best)
            {
                summary.best_executable_net_bps = Some(row.one_cycle_net_bps);
                summary.best_executable_pair = row.pair.clone();
            }
        }
    }
    if !local_filter_active {
        summary.filtered_candidates = candidate_count;
        summary.executable_candidates = executable_count;
    }
    summary
}

pub(in crate::panels::modules::opportunities) fn matches_filter(
    row: &OpportunityListViewModel,
    filter: &OpportunityFilter,
    query: &str,
) -> bool {
    (filter.min_net_pct <= 0.0
        || row.cost_verified && row.one_cycle_net_bps / 100.0 >= filter.min_net_pct)
        && matches_strategy(row, filter.strategy)
        && matches_query(row, query)
}

pub(in crate::panels::modules::opportunities) fn matches_strategy(
    row: &OpportunityListViewModel,
    selected: Option<StrategyKind>,
) -> bool {
    selected.is_none_or(|kind| row.strategy_kind == Some(kind))
}

pub(in crate::panels::modules::opportunities) fn matches_query(
    row: &OpportunityListViewModel,
    query: &str,
) -> bool {
    if query.is_empty() {
        return true;
    }
    if is_venue_query(query) {
        return row_text_matches(row, query);
    }
    // Symbol queries are scoped by the backend canonical parser before this local filter runs.
    if symbol_search_query(query).is_some() {
        return row.pair.eq_ignore_ascii_case(query);
    }
    row_text_matches(row, query)
}

pub(in crate::panels::modules::opportunities) fn row_text_matches(
    row: &OpportunityListViewModel,
    query: &str,
) -> bool {
    [
        row.pair.as_str(),
        row.strategy_label.as_str(),
        row.data_source.as_str(),
        row.long_venue.as_str(),
        row.short_venue.as_str(),
        row.long_leg.as_str(),
        row.short_leg.as_str(),
        row.spot_leg_mode_label().unwrap_or_default(),
        row.id.as_str(),
    ]
    .into_iter()
    .any(|value| value.to_ascii_uppercase().contains(query))
}
