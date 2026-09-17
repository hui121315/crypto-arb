use super::{
    api_problem, next_request_gate, RequestGate, ReviewPagedState, ReviewRuntime, ReviewState,
};
use crate::api::rest::ApiClient;
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::ExecutedTrade;

pub(super) fn use_executed_pages(runtime: ReviewRuntime) -> ReviewPagedState<ExecutedTrade> {
    let state = runtime.executed.state;
    let cursor = runtime.executed.cursor;
    let loading = RwSignal::new(false);
    let request_version = RwSignal::new(0_u64);
    let client = use_global().client;

    if let Some(current_cursor) = cursor.get_untracked() {
        let gate = next_request_gate(request_version);
        spawn_page_fetch(state, loading, client.clone(), gate, current_cursor);
    } else if let Some(first_page) = runtime.executed_first_page.get_untracked() {
        state.set(LoadState::Ready(first_page));
    }

    let load_cursor = Callback::new(move |next_cursor: Option<String>| {
        cursor.set(next_cursor.clone());
        let gate = next_request_gate(request_version);
        match next_cursor {
            Some(next_cursor) => {
                spawn_page_fetch(state, loading, client.clone(), gate, next_cursor);
            }
            None => restore_first_page(runtime, state, loading),
        }
    });

    ReviewPagedState {
        state,
        loading,
        load_cursor,
    }
}

fn restore_first_page(
    runtime: ReviewRuntime,
    state: ReviewState<ExecutedTrade>,
    loading: RwSignal<bool>,
) {
    loading.set(false);
    match runtime.executed_first_page.get_untracked() {
        Some(first_page) => state.set(LoadState::Ready(first_page)),
        None => state.set(LoadState::Loading),
    }
}

fn spawn_page_fetch(
    state: ReviewState<ExecutedTrade>,
    loading: RwSignal<bool>,
    client: ApiClient,
    gate: RequestGate,
    cursor: String,
) {
    loading.set(true);
    spawn_local(async move {
        let result = client
            .review_executed_page(30, Some(&cursor))
            .await
            .map_err(api_problem);
        if gate.is_latest() {
            state.update(|state| state.apply_result(result));
            loading.set(false);
        }
    });
}
