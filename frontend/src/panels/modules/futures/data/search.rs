use crate::api::rest::{ApiError, OpportunityListResponse};
use crate::panels::modules::futures::mapper::to_futures_opps_from_list_views;
use crate::panels::modules::instrument_search::{
    is_venue_query, normalized_query, symbol_search_problem, symbol_search_query,
};
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_runtime::apply_opportunity_rows_envelope;
use crate::panels::modules::opportunity_view_model::{
    view_models_from_rows, OpportunityListViewRow,
};
use crate::panels::modules::strategy_scope::P0_STRATEGY_KINDS;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::use_debounced_string;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::OpportunityListPage;
use shared_types::StrategyKindInfo;

use super::*;

#[derive(Clone)]
pub(in crate::panels::modules::futures) struct SymbolFuturesSearch {
    pub(in crate::panels::modules::futures) state: RwSignal<LoadState<()>>,
    pub(in crate::panels::modules::futures) rows: Memo<Vec<FuturesOpportunityRow>>,
    pub(in crate::panels::modules::futures) meta: RwSignal<OpportunityCountMeta>,
    pub(in crate::panels::modules::futures) page: RwSignal<Option<OpportunityListPage>>,
    pub(in crate::panels::modules::futures) load_cursor: Callback<Option<String>>,
}

pub(in crate::panels::modules::futures) fn use_symbol_futures_opportunities(
    runtime: FuturesRuntime,
) -> SymbolFuturesSearch {
    let filter = runtime.filter;
    let search = runtime.search;
    let state = search.state;
    let shared_rows = search.rows;
    let rows = Memo::new(move |_| to_futures_opps_from_list_views(shared_rows.get()));
    let meta = search.meta;
    let page = search.page;
    let last_query = search.last_query;
    let cursor = search.cursor;
    let last_strategy = RwSignal::new(filter.get_untracked().strategy.kind());
    let debounced_query = use_debounced_string(
        move || symbol_search_query(&filter.get().query).unwrap_or_default(),
        SEARCH_DEBOUNCE,
    );
    let client = use_global().client;
    Effect::new(move |_| {
        let strategy = filter.get().strategy.kind();
        let strategy_changed = strategy != last_strategy.get_untracked();
        if strategy_changed {
            last_strategy.set(strategy);
        }
        let query = debounced_query.get();
        let cursor_value = cursor.get();
        if query != last_query.get_untracked() || strategy_changed {
            last_query.set(query.clone());
            if cursor_value.is_some() {
                state.set(LoadState::Loading);
                cursor.set(None);
                return;
            }
        }
        if query.is_empty() {
            shared_rows.set(Vec::new());
            page.set(None);
            state.set(LoadState::Ready(()));
            return;
        }
        state.set(LoadState::Loading);
        let client = client.clone();
        spawn_local(async move {
            let result = client
                .scan_opportunity_list_scoped_page(
                    Some(strategy),
                    Some(&query),
                    cursor_value.as_deref(),
                    FUTURES_PAGE_SIZE,
                )
                .await;
            if last_query.get_untracked() != query
                || cursor.get_untracked() != cursor_value
                || filter.get_untracked().strategy.kind() != strategy
            {
                return;
            }
            match result {
                Ok(latest) => {
                    apply_symbol_futures_success(&latest, state, shared_rows, meta, page);
                }
                Err(error) => {
                    apply_symbol_futures_error(error, state, &query, cursor_value.as_deref());
                }
            }
        });
    });
    let load_cursor = Callback::new(move |next| {
        state.set(LoadState::Loading);
        cursor.set(clean_cursor(next));
    });
    SymbolFuturesSearch {
        state,
        rows,
        meta,
        page,
        load_cursor,
    }
}

fn apply_symbol_futures_success(
    latest: &OpportunityListResponse,
    state: RwSignal<LoadState<()>>,
    rows: RwSignal<Vec<OpportunityListViewRow>>,
    meta: RwSignal<OpportunityCountMeta>,
    page: RwSignal<Option<OpportunityListPage>>,
) {
    apply_opportunity_rows_envelope(latest, state, rows, meta, page, |latest| {
        view_models_from_rows(&latest.page.snapshot_id, latest.rows.clone())
    });
}

pub(in crate::panels::modules::futures) fn apply_symbol_futures_error(
    error: ApiError,
    state: RwSignal<LoadState<()>>,
    query: &str,
    cursor: Option<&str>,
) {
    let problem = symbol_search_problem(error.problem, query, cursor);
    state.update(|state| state.apply_result(Err(problem)));
}

pub(in crate::panels::modules::futures) fn futures_chips(
    kinds: &[StrategyKindInfo],
) -> Vec<StrategyFilter> {
    P0_STRATEGY_KINDS
        .into_iter()
        .filter_map(StrategyFilter::from_kind)
        .filter(|filter| {
            kinds
                .iter()
                .any(|item| item.kind == filter.kind() && item.is_main_p0_executable())
        })
        .collect()
}

pub(in crate::panels::modules::futures) fn filter_rows(
    rows: &[FuturesOpportunityRow],
    filter: &FuturesFilter,
) -> Vec<FuturesOpportunityRow> {
    let query = normalized_query(&filter.query);
    rows.iter()
        .filter(|row| matches_filter(row, filter, &query))
        .cloned()
        .collect()
}

fn matches_filter(row: &FuturesOpportunityRow, filter: &FuturesFilter, query: &str) -> bool {
    filter.strategy.matches(row) && matches_metric_filter(row, filter) && matches_query(row, query)
}

fn matches_metric_filter(row: &FuturesOpportunityRow, filter: &FuturesFilter) -> bool {
    filter.min_net_pct <= 0.0
        || row.cost_verified && row.one_cycle_net_bps / 100.0 >= filter.min_net_pct
}

fn matches_query(row: &FuturesOpportunityRow, query: &str) -> bool {
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

fn row_text_matches(row: &FuturesOpportunityRow, query: &str) -> bool {
    [
        row.pair.as_str(),
        row.long_leg.as_str(),
        row.short_leg.as_str(),
        row.strategy_label.as_str(),
        row.spot_leg_mode_label().unwrap_or_default(),
        row.index_composition.label.as_str(),
        row.id.as_str(),
    ]
    .into_iter()
    .any(|value| value.to_ascii_uppercase().contains(query))
}

pub(in crate::panels::modules::futures) fn summarize_rows(
    rows: &[FuturesOpportunityRow],
    filter: &FuturesFilter,
    meta: &OpportunityCountMeta,
) -> FuturesSummary {
    let query = normalized_query(&filter.query);
    let matching_strategy_count = rows
        .iter()
        .filter(|row| filter.strategy.matches(row))
        .count();
    let candidate_count = meta.selected_count(None, matching_strategy_count);
    let mut summary = FuturesSummary {
        candidates: candidate_count,
        filtered_candidates: 0,
        executable_candidates: 0,
        best_monitor_net_bps: None,
        best_monitor_pair: "-".into(),
        best_monitor_profit_detail: "-".into(),
        best_monitor_preview_ready: false,
    };
    for row in rows {
        if !matches_filter(row, filter, &query) {
            continue;
        }
        summary.filtered_candidates += 1;
        if row.execution_eligible {
            summary.executable_candidates += 1;
        }
        let has_monitor_profit = row.cost_verified
            && row.one_cycle_net_bps.is_finite()
            && row.one_cycle_net_bps > 0.0
            && shared_types::monitoring_blockers_are_execution_only(&row.execution_blockers);
        let replaces_best = has_monitor_profit
            && summary
                .best_monitor_net_bps
                .is_none_or(|best| row.one_cycle_net_bps > best);
        if replaces_best {
            summary.best_monitor_net_bps = Some(row.one_cycle_net_bps);
            summary.best_monitor_pair = row.pair.clone();
            summary.best_monitor_profit_detail = format!(
                "{} · 费后边际 {}",
                row.cost_evidence_label(),
                row.one_cycle_net_text()
            );
            summary.best_monitor_preview_ready = row.execution_eligible;
        }
    }
    summary
}
