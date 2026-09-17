use crate::api::rest::{ApiError, OpportunityListResponse};
use crate::panels::modules::opportunity_counts::OpportunityCountMeta;
use crate::panels::modules::opportunity_runtime::{
    apply_live_first_page_from_stream_when, apply_opportunity_rows_envelope,
    clean_opportunity_cursor, opportunity_page_request_required, OpportunityRowsTarget,
};
#[cfg(test)]
use crate::panels::modules::opportunity_view_model::patch_projected_rows;
use crate::panels::modules::opportunity_view_model::{
    view_models_from_rows, OpportunityListViewRow,
};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use shared_types::{OpportunityEnvelopeStatus, OpportunityListPage, StrategyKind};

use crate::panels::modules::futures::mapper::to_futures_opps_from_list_views;

use super::*;

mod merge;
pub(in crate::panels::modules::futures) use merge::*;
mod refresh;
pub(in crate::panels::modules::futures) use refresh::*;

const FUTURES_EMPTY_WINDOW_CONFIRM_MS: i64 = 2_500;

pub(in crate::panels::modules::futures) fn use_futures_opportunities(
    runtime: FuturesRuntime,
) -> FuturesOpportunityStore {
    let global = use_global();
    let client = global.client;
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
    let empty_window_since_ms = RwSignal::new(None::<i64>);
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
            empty_window_since_ms.set(None);
        }
        current
    });
    let resource = LocalResource::new(move || {
        let client = client.clone();
        let cursor_value = cursor.get();
        let strategy_value = strategy.get();
        let should_fetch = opportunity_page_request_required(cursor_value.as_deref());
        async move {
            if !should_fetch {
                return None;
            }
            let result = client
                .futures_opportunity_list_for_strategy_page(
                    strategy_value,
                    cursor_value.as_deref(),
                    FUTURES_PAGE_SIZE,
                )
                .await;
            Some(match result {
                Ok(latest) => Ok((strategy_value, cursor_value, latest)),
                Err(error) => Err((strategy_value, cursor_value, error)),
            })
        }
    });
    Effect::new(move |_| {
        let Some(value) = resource.get() else {
            return;
        };
        let Some(scoped_result) = (*value).clone() else {
            return;
        };
        let (strategy_value, request_cursor, result) = match scoped_result {
            Ok((strategy, cursor, latest)) => (strategy, cursor, Ok(latest)),
            Err((strategy, cursor, error)) => (strategy, cursor, Err(error)),
        };
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
    Effect::new(move |_| {
        let selected_strategy = strategy.get();
        if cursor.get().is_some() {
            return;
        }
        let handled = stream_state.with(|stream| {
            live_rows.with(|rows| {
                apply_live_first_page_from_stream_when(
                    stream,
                    rows,
                    Some(selected_strategy),
                    &OpportunityRowsTarget::new(state, shared_rows, meta, page),
                    |envelope| {
                        if envelope.status != OpportunityEnvelopeStatus::Fresh {
                            empty_window_since_ms.set(None);
                            return true;
                        }
                        let mut empty_since_ms = empty_window_since_ms.get_untracked();
                        let publish = futures_stream_window_should_publish(
                            envelope.rows.len(),
                            shared_rows.with_untracked(Vec::len),
                            envelope.observed_at_ms,
                            &mut empty_since_ms,
                        );
                        empty_window_since_ms.set(empty_since_ms);
                        publish
                    },
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
        cursor.set(clean_cursor(next));
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

pub(in crate::panels::modules::futures) fn futures_stream_window_should_publish(
    incoming_rows: usize,
    current_rows: usize,
    observed_at_ms: i64,
    empty_since_ms: &mut Option<i64>,
) -> bool {
    if incoming_rows > 0 || current_rows == 0 {
        *empty_since_ms = None;
        return true;
    }
    let first_empty_at_ms = empty_since_ms.get_or_insert(observed_at_ms);
    if observed_at_ms.saturating_sub(*first_empty_at_ms) < FUTURES_EMPTY_WINDOW_CONFIRM_MS {
        return false;
    }
    *empty_since_ms = None;
    true
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
