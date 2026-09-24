use super::{
    api_problem, next_request_gate, RequestGate, ReviewPagedState, ReviewRuntime, ReviewState,
};
use crate::api::rest::ApiClient;
use crate::state::context::use_global;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::review::ReviewScope;
use shared_types::ExecutedTrade;

pub(super) fn use_executed_pages(runtime: ReviewRuntime) -> ReviewPagedState<ExecutedTrade> {
    let state = runtime.executed.state;
    let cursor = runtime.executed.cursor;
    let loading = RwSignal::new(false);
    let request_version = RwSignal::new(0_u64);
    let client = use_global().client;

    let initial_client = client.clone();
    Effect::new(move |_| {
        runtime.refresh_nonce.get();
        let scope = runtime.scope.get();
        if scope.is_some() || cursor.get_untracked().is_some() {
            let gate = next_request_gate(request_version);
            spawn_page_fetch(
                runtime,
                loading,
                initial_client.clone(),
                gate,
                cursor.get_untracked(),
                scope,
            );
        } else {
            next_request_gate(request_version);
            loading.set(false);
            state.set(runtime.executed_first_page.get_untracked());
        }
    });

    let load_cursor = Callback::new(move |next_cursor: Option<String>| {
        cursor.set(next_cursor.clone());
        let gate = next_request_gate(request_version);
        let scope = runtime.scope.get_untracked();
        if next_cursor.is_some() || scope.is_some() {
            spawn_page_fetch(runtime, loading, client.clone(), gate, next_cursor, scope);
        } else {
            restore_first_page(runtime, state, loading);
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
    state.set(runtime.executed_first_page.get_untracked());
}

fn spawn_page_fetch(
    runtime: ReviewRuntime,
    loading: RwSignal<bool>,
    client: ApiClient,
    gate: RequestGate,
    cursor: Option<String>,
    scope: Option<ReviewScope>,
) {
    loading.set(true);
    spawn_local(async move {
        let base = client.base_url();
        let result = match scope.as_ref() {
            Some(scope) => client.review_scoped_page(scope, cursor.as_deref()).await,
            None => client.review_executed_page(30, cursor.as_deref()).await,
        }
        .map_err(api_problem);
        if gate.is_latest() && runtime.scope.get_untracked() == scope && client.base_url() == base {
            runtime
                .executed
                .state
                .update(|state| state.apply_result(result));
            loading.set(false);
        }
    });
}
