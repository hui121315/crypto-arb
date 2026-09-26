use crate::api::rest::{ApiError, OpportunityListResponse};
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_runtime::{
    apply_live_first_page_from_stream, apply_opportunity_rows_envelope,
    clean_opportunity_cursor, opportunity_page_request_required, OpportunityRowsTarget,
};
#[cfg(test)]
use crate::panels::modules::opportunity_view_model::patch_projected_rows;
use crate::panels::modules::opportunity_view_model::{
    view_models_from_rows, OpportunityListViewRow,
};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::read_scope::bounded_read;
use leptos::prelude::*;
use shared_types::{OpportunityListPage, StrategyKind};

use crate::panels::modules::futures::mapper::to_futures_opps_from_list_views;

use super::*;

mod merge;
pub(in crate::panels::modules::futures) use merge::*;
mod refresh;
pub(in crate::panels::modules::futures) use refresh::*;

pub(in crate::panels::modules::futures) fn use_futures_opportunities(
    runtime: FuturesRuntime,
) -> FuturesOpportunityStore {
    let global = use_global();
    let scope = global.arbitrage_stream.scope;
    let read = scope.request();
    let stream_state = global.arbitrage_stream.state;
    let live_rows = global.arbitrage_stream.live_rows;
    let list = runtime.list;
    let state = list.state;
    let shared_rows = list.rows;
    let rows = Memo::new(move |_| to_futures_opps_from_list_views(shared_rows.get()));
    let meta = list.meta;
    let page = list.page;
    let loading = list.loading;
    let cursor = list.cursor;
    let refresh = RwSignal::new(0_u64);
    let strategy = Memo::new(move |_| runtime.filter.get().strategy.kind());
    Effect::new(move |previous: Option<StrategyKind>| {
        let current = strategy.get();
        if previous.is_some_and(|previous| previous != current) {
            state.set(LoadState::Loading);
            shared_rows.set(Vec::new());
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
            bounded_read(client.futures_opportunity_list_for_strategy_page(
                    strategy_value,
                    cursor_value.as_deref(),
                    FUTURES_PAGE_SIZE,
                ))
                .await
                .map_err(ApiError::from_problem)
        }, move |result| {
            if !futures_list_request_is_current(
                strategy_value,
                request_cursor.as_deref(),
                strategy.get_untracked(),
                cursor.get_untracked().as_deref(),
            ) {
                return;
            }
            loading.set(false);
            apply_futures_list_result(result, state, shared_rows, meta, page);
        });
    });
    Effect::new(move |_| {
        let selected_strategy = strategy.get();
        if cursor.get().is_some() {
            return;
        }
        let handled = stream_state.with(|stream| {
            live_rows.with(|rows| {
                apply_live_first_page_from_stream(
                    stream,
                    rows,
                    Some(selected_strategy),
                    &OpportunityRowsTarget::new(state, shared_rows, meta, page),
                    shared_rows_from_list_response,
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
    FuturesOpportunityStore {
        state,
        rows,
        meta,
        page,
        loading,
        load_cursor,
    }
}

fn apply_futures_list_result(
    result: Result<OpportunityListResponse, ApiError>,
    state: RwSignal<LoadState<()>>,
    rows: RwSignal<Vec<OpportunityListViewRow>>,
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
                shared_rows_from_list_response,
            );
        }
        Err(error) => {
            state.update(|state| state.apply_result(Err(error.problem)));
        }
    }
}

fn shared_rows_from_list_response(latest: &OpportunityListResponse) -> Vec<OpportunityListViewRow> {
    view_models_from_rows(&latest.page.snapshot_id, latest.rows.clone())
}

#[cfg(test)]
pub(in crate::panels::modules::futures) fn patch_futures_rows(
    rows: &mut Vec<FuturesOpportunityRow>,
    removed_ids: &[String],
    changed_rows: Vec<FuturesOpportunityRow>,
) {
    patch_projected_rows(rows, removed_ids, changed_rows, |row| row.id.as_str());
}

pub(in crate::panels::modules::futures) fn clean_cursor(cursor: Option<String>) -> Option<String> {
    clean_opportunity_cursor(cursor)
}
