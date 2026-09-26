use super::*;
use crate::api::rest::{ApiError, OpportunityListResponse};
use crate::panels::modules::instrument_search::symbol_search_problem;
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_runtime::{
    apply_live_first_page_from_stream, apply_opportunity_rows_envelope,
    merge_symbol_quote_projections, opportunity_page_request_required,
    OpportunityQuoteProjection, OpportunityQuoteSnapshot, OpportunityRowsTarget,
};
use crate::panels::modules::opportunity_view_model::view_models_from_rows;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::read_scope::bounded_read;
use leptos::prelude::*;
use shared_types::{OpportunityListPage, StrategyKind};

pub(in crate::panels::modules::opportunities) fn use_opportunities(
    runtime: OpportunitiesRuntime,
) -> OpportunityStore {
    let list = runtime.list;
    let strategy = Memo::new(move |_| runtime.filter.get().strategy);
    let rows = list.rows;
    let meta = list.meta;
    let page = list.page;
    let loading = list.loading;
    let global = use_global();
    let scope = global.arbitrage_stream.scope;
    let read = scope.request();
    let stream_state = global.arbitrage_stream.state;
    let live_rows = global.arbitrage_stream.live_rows;
    let state = list.state;
    let cursor = list.cursor;
    let refresh = RwSignal::new(0_u64);
    Effect::new(move |previous: Option<Option<StrategyKind>>| {
        let current = strategy.get();
        if previous.is_some_and(|previous| previous != current) {
            state.set(LoadState::Loading);
            rows.set(Vec::new());
            meta.set(OpportunityCountMeta::default());
            page.set(None);
            loading.set(true);
            cursor.set(None);
        }
        current
    });
    Effect::new(move |_| {
        scope.track();
        refresh.track();
        read.cancel();
        let cursor_value = cursor.get();
        let strategy_value = strategy.get();
        if !opportunity_page_request_required(cursor_value.as_deref()) {
            return;
        }
        loading.set(true);
        let request_cursor = cursor_value.clone();
        read.run(move |client| async move {
            bounded_read(client
                .scan_opportunity_list_scoped_page(
                    strategy_value,
                    None,
                    cursor_value.as_deref(),
                    OPPORTUNITY_PAGE_SIZE,
                ))
                .await.map_err(ApiError::from_problem)
        }, move |result| {
            if !opportunity_list_request_is_current(
                strategy_value,
                request_cursor.as_deref(),
                strategy.get_untracked(),
                cursor.get_untracked().as_deref(),
            ) {
                return;
            }
            loading.set(false);
            apply_opportunity_list_result(result, state, rows, meta, page);
        });
    });
    Effect::new(move |_| {
        let selected_strategy = strategy.get();
        if cursor.get().is_some() {
            return;
        }
        let handled = stream_state.with(|stream| {
            live_rows.with(|live_rows| {
                apply_live_first_page_from_stream(
                    stream,
                    live_rows,
                    selected_strategy,
                    &OpportunityRowsTarget::new(state, rows, meta, page),
                    opportunities_from_list_response,
                )
            })
        });
        if handled {
            loading.set(false);
        }
    });
    let load_cursor = Callback::new(move |next| {
        loading.set(true);
        let next = clean_cursor(next);
        if next.is_some() && next == cursor.get_untracked() {
            refresh.update(|revision| *revision = revision.wrapping_add(1));
        } else {
            cursor.set(next);
        }
    });
    OpportunityStore {
        state,
        rows,
        meta,
        page,
        loading,
        load_cursor,
    }
}

pub(in crate::panels::modules::opportunities) fn apply_symbol_opportunities_success(
    latest: &OpportunityListResponse,
    state: RwSignal<LoadState<()>>,
    rows: RwSignal<Vec<OpportunityRow>>,
    meta: RwSignal<OpportunityCountMeta>,
    page: RwSignal<Option<OpportunityListPage>>,
) {
    apply_opportunity_rows_envelope(
        latest,
        state,
        rows,
        meta,
        page,
        opportunities_from_list_response,
    );
}

pub(in crate::panels::modules::opportunities) fn apply_symbol_opportunities_error(
    error: ApiError,
    state: RwSignal<LoadState<()>>,
    query: &str,
    cursor: Option<&str>,
) {
    let problem = symbol_search_problem(error.problem, query, cursor);
    state.update(|state| state.apply_result(Err(problem)));
}

pub(in crate::panels::modules::opportunities) fn apply_opportunity_list_result(
    result: Result<OpportunityListResponse, ApiError>,
    state: RwSignal<LoadState<()>>,
    rows: RwSignal<Vec<OpportunityRow>>,
    meta: RwSignal<OpportunityCountMeta>,
    page: RwSignal<Option<OpportunityListPage>>,
) {
    match result {
        Ok(latest) => {
            apply_opportunity_rows_envelope(
                &latest,
                state,
                rows,
                meta,
                page,
                opportunities_from_list_response,
            );
        }
        Err(error) => {
            state.update(|state| state.apply_result(Err(error.problem)));
        }
    }
}

pub(in crate::panels::modules::opportunities) fn opportunities_from_list_response(
    latest: &OpportunityListResponse,
) -> Vec<OpportunityRow> {
    view_models_from_rows(&latest.page.snapshot_id, latest.rows.clone())
}

fn opportunity_list_request_is_current(
    request_strategy: Option<StrategyKind>,
    request_cursor: Option<&str>,
    current_strategy: Option<StrategyKind>,
    current_cursor: Option<&str>,
) -> bool {
    request_strategy == current_strategy && request_cursor == current_cursor
}

#[cfg(test)]
pub(in crate::panels::modules::opportunities) fn merge_opportunity_rows(
    base: &[OpportunityRow],
    extra: &[OpportunityRow],
) -> Vec<OpportunityRow> {
    crate::panels::modules::opportunity_runtime::merge_opportunity_projections(
        base,
        extra,
        |left, right| left.id == right.id,
        opportunity_rank_order,
    )
}

pub(in crate::panels::modules::opportunities) fn merge_symbol_opportunity_rows(
    live: OpportunityQuoteSnapshot<'_, OpportunityRow>,
    search: OpportunityQuoteSnapshot<'_, OpportunityRow>,
) -> OpportunityQuoteProjection<OpportunityRow> {
    merge_symbol_quote_projections(live, search, |row| &row.id, |row| &row.pair, opportunity_rank_order)
}

pub(in crate::panels::modules::opportunities) fn opportunity_rank_order(
    left: &OpportunityRow,
    right: &OpportunityRow,
) -> std::cmp::Ordering {
    right
        .execution_eligible
        .cmp(&left.execution_eligible)
        .then_with(|| verified_positive_profit(right).cmp(&verified_positive_profit(left)))
        .then_with(|| right.one_cycle_net_bps.total_cmp(&left.one_cycle_net_bps))
        .then_with(|| left.id.cmp(&right.id))
}

fn verified_positive_profit(row: &OpportunityRow) -> bool {
    row.cost_verified && row.one_cycle_net_bps.is_finite() && row.one_cycle_net_bps > 0.0
}
