use super::{
    next_request_gate, review_envelope_result, review_read, review_request, RequestGate, ReviewPagedState, ReviewRuntime, ReviewState,
};
use crate::state::read_scope::ScopedRead;
use leptos::prelude::*;
use shared_types::review::ReviewScope;
use shared_types::ExecutedTrade;

pub(super) fn use_executed_pages(runtime: ReviewRuntime) -> ReviewPagedState<ExecutedTrade> {
    let state = runtime.executed.state;
    let cursor = runtime.executed.cursor;
    let loading = RwSignal::new(false);
    let request_version = RwSignal::new(0_u64);
    let request = review_request(runtime.connection);

    Effect::new(move |_| {
        runtime.refresh_nonce.get();
        let scope = runtime.scope.get();
        if !runtime.connection.current() { return; }
        if scope.is_some() || cursor.get_untracked().is_some() {
            let gate = next_request_gate(request_version);
            spawn_page_fetch(
                runtime,
                loading,
                request,
                gate,
                cursor.get_untracked(),
                scope,
            );
        } else {
            request.cancel();
            next_request_gate(request_version);
            loading.set(false);
            state.set(runtime.executed_first_page.get_untracked());
        }
    });

    let load_cursor = Callback::new(move |next_cursor: Option<String>| {
        if !runtime.connection.current() { return; }
        cursor.set(next_cursor.clone());
        let gate = next_request_gate(request_version);
        let scope = runtime.scope.get_untracked();
        if next_cursor.is_some() || scope.is_some() {
            spawn_page_fetch(runtime, loading, request, gate, next_cursor, scope);
        } else {
            request.cancel();
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
    request: ScopedRead,
    gate: RequestGate,
    cursor: Option<String>,
    scope: Option<ReviewScope>,
) {
    loading.set(true);
    let requested_scope = scope.clone();
    request.run(move |client| async move {
        let result = match requested_scope.as_ref() {
            Some(scope) => review_read(client.review_scoped_page(scope, cursor.as_deref())).await,
            None => review_read(client.review_executed_page(30, cursor.as_deref())).await,
        };
        result.and_then(review_envelope_result)
    }, move |result| {
        if runtime.connection.current() && gate.is_latest() && runtime.scope.get_untracked() == scope {
            runtime
                .executed
                .state
                .update(|state| state.apply_result(result));
            loading.set(false);
        }
    });
}
