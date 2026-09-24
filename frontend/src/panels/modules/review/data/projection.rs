use super::{api_problem, ReviewRuntime};
use crate::api::ws::{start_review_stream_with_state, WsChannelState};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::{use_ws_channel_context_snapshot_fallback, SnapshotFallbackTiming};
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{ApiProblem, ReviewRuntimeSnapshot};
use std::time::Duration;

pub(in crate::panels::modules::review) fn use_runtime_projection(
    runtime: ReviewRuntime,
) -> RwSignal<bool> {
    let client = use_global().client;
    let revision = RwSignal::new(0_u64);
    let reading = RwSignal::new(false);
    let initialized = RwSignal::new(false);
    let channel_state = RwSignal::new(WsChannelState::new("review"));
    let handle = start_review_stream_with_state(
        channel_state,
        move |snapshot| {
            if apply_snapshot(runtime, snapshot) {
                revision.update(|value| *value = value.wrapping_add(1));
            }
        },
        move |problem| {
            revision.update(|value| *value = value.wrapping_add(1));
            apply_problem(runtime, problem);
        },
    );
    on_cleanup(move || handle.cancel());
    let seed_client = client.clone();
    Effect::new(move |_| {
        runtime.refresh_nonce.get();
        if reading.get_untracked() {
            return;
        }
        reading.set(true);
        let client = seed_client.clone();
        let anchor = revision.get_untracked();
        let base = client.base_url();
        spawn_local(async move {
            let result = client.review_runtime().await.map_err(api_problem);
            if reading.try_get_untracked().is_none() {
                return;
            }
            reading.set(false);
            initialized.set(true);
            if base == client.base_url() && response_is_current(revision.get_untracked(), anchor) {
                apply_result(runtime, result);
            }
        });
    });
    let result_client = client.clone();
    use_ws_channel_context_snapshot_fallback(
        channel_state,
        SnapshotFallbackTiming {
            period: Duration::from_secs(10),
            grace: Duration::from_secs(8),
            stale_after: Duration::from_secs(40),
        },
        move || initialized.get_untracked() && !reading.get_untracked(),
        move || {
            let client = client.clone();
            let context = (client.base_url(), revision.get_untracked());
            reading.set(true);
            async move { (context, client.review_runtime().await.map_err(api_problem)) }
        },
        move |(base, anchor), result| {
            reading.set(false);
            if base == result_client.base_url()
                && response_is_current(revision.get_untracked(), anchor)
            {
                apply_result(runtime, result);
            }
        },
    );
    reading
}

fn apply_snapshot(runtime: ReviewRuntime, snapshot: ReviewRuntimeSnapshot) -> bool {
    if has_newer_generation(review_generation(runtime), Some(snapshot.generated_at_ms)) {
        return false;
    }
    let ReviewRuntimeSnapshot {
        executed,
        strategy_performance,
        ..
    } = snapshot;
    runtime
        .executed_first_page
        .set(LoadState::Ready(executed.clone()));
    if runtime.executed.cursor.get_untracked().is_none() {
        runtime.executed.state.set(LoadState::Ready(executed));
    }
    runtime.perf.set(LoadState::Ready(strategy_performance));
    true
}

fn apply_result(runtime: ReviewRuntime, result: Result<ReviewRuntimeSnapshot, ApiProblem>) {
    match result {
        Ok(snapshot) => {
            apply_snapshot(runtime, snapshot);
        }
        Err(problem) => apply_problem(runtime, problem),
    }
}

fn review_generation(runtime: ReviewRuntime) -> Option<i64> {
    let executed = runtime
        .executed_first_page
        .get_untracked()
        .value()
        .map(|envelope| envelope.generated_at_ms);
    let performance = runtime
        .perf
        .get_untracked()
        .value()
        .map(|envelope| envelope.generated_at_ms);
    executed.into_iter().chain(performance).max()
}

fn has_newer_generation(current: Option<i64>, reference: Option<i64>) -> bool {
    current > reference
}

fn response_is_current(revision: u64, request_revision: u64) -> bool {
    revision == request_revision
}

fn apply_problem(runtime: ReviewRuntime, problem: ApiProblem) {
    runtime
        .executed_first_page
        .update(|state| state.apply_result(Err(problem.clone())));
    if runtime.executed.cursor.get_untracked().is_none() {
        runtime
            .executed
            .state
            .update(|state| state.apply_result(Err(problem.clone())));
    }
    runtime
        .perf
        .update(|state| state.apply_result(Err(problem)));
}

#[cfg(test)]
mod tests {
    use super::has_newer_generation;

    #[test]
    fn same_timestamp_ws_revision_rejects_previous_http_success_or_error() {
        assert!(super::response_is_current(2, 2));
        assert!(!super::response_is_current(3, 2));
    }

    #[test]
    fn newer_ws_generation_rejects_older_rest_snapshot() {
        assert!(has_newer_generation(Some(20), Some(15)));
        assert!(!has_newer_generation(Some(20), Some(20)));
    }

    #[test]
    fn newer_ws_generation_rejects_obsolete_rest_error() {
        assert!(has_newer_generation(Some(20), Some(10)));
        assert!(has_newer_generation(Some(20), None));
        assert!(!has_newer_generation(None, None));
    }
}
