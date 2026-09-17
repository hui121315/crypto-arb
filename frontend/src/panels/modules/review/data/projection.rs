use super::{api_problem, ReviewRuntime};
use crate::api::ws::{start_review_stream_with_state, WsChannelState};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::{use_ws_channel_context_snapshot_fallback, SnapshotFallbackTiming};
use leptos::prelude::*;
use shared_types::{ApiProblem, ReviewRuntimeSnapshot};
use std::time::Duration;

pub(in crate::panels::modules::review) fn use_runtime_projection(runtime: ReviewRuntime) {
    let client = use_global().client;
    let channel_state = RwSignal::new(WsChannelState::new("review"));
    let handle = start_review_stream_with_state(
        channel_state,
        move |snapshot| apply_snapshot(runtime, snapshot),
        move |problem| apply_problem(runtime, problem),
    );
    on_cleanup(move || handle.cancel());
    use_ws_channel_context_snapshot_fallback(
        channel_state,
        SnapshotFallbackTiming {
            period: Duration::from_secs(10),
            grace: Duration::from_secs(8),
            stale_after: Duration::from_secs(40),
        },
        || true,
        move || {
            let client = client.clone();
            let anchor = review_generation(runtime);
            async move { (anchor, client.review_runtime().await.map_err(api_problem)) }
        },
        move |anchor, result| apply_fallback_result(runtime, anchor, result),
    );
}

fn apply_snapshot(runtime: ReviewRuntime, snapshot: ReviewRuntimeSnapshot) {
    if has_newer_generation(review_generation(runtime), Some(snapshot.generated_at_ms)) {
        return;
    }
    let ReviewRuntimeSnapshot {
        executed,
        strategy_performance,
        ..
    } = snapshot;
    runtime.executed_first_page.set(Some(executed.clone()));
    if runtime.executed.cursor.get_untracked().is_none() {
        runtime.executed.state.set(LoadState::Ready(executed));
    }
    runtime.perf.set(LoadState::Ready(strategy_performance));
}

fn apply_fallback_result(
    runtime: ReviewRuntime,
    request_anchor: Option<i64>,
    result: Result<ReviewRuntimeSnapshot, ApiProblem>,
) {
    match result {
        Ok(snapshot) => apply_snapshot(runtime, snapshot),
        Err(_) if has_newer_generation(review_generation(runtime), request_anchor) => {}
        Err(problem) => apply_problem(runtime, problem),
    }
}

fn review_generation(runtime: ReviewRuntime) -> Option<i64> {
    let executed = runtime
        .executed_first_page
        .get_untracked()
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

fn apply_problem(runtime: ReviewRuntime, problem: ApiProblem) {
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
