use super::{review_envelope_result, review_now_ms, review_read, review_request, ReviewRuntime};
use super::retry::{review_poll_allowed, review_retry_deadline_ms};
use crate::api::ws::{start_review_stream_with_state, WsChannelState};
use crate::state::polling::use_ws_channel_fallback_polling;
use gloo_timers::callback::Interval;
use leptos::prelude::*;
use shared_types::{ApiProblem, ReviewRuntimeSnapshot};
use std::time::Duration;

pub(in crate::panels::modules::review) fn use_runtime_projection(
    runtime: ReviewRuntime,
) -> RwSignal<bool> {
    let request = review_request(runtime.connection);
    let revision = RwSignal::new(0_u64);
    let reading = RwSignal::new(false);
    let initialized = RwSignal::new(false);
    let retry_until_ms = RwSignal::new(None::<u64>);
    let tick = RwSignal::new(0_u64);
    let last_refresh = StoredValue::new(None::<u64>);
    let channel_state = RwSignal::new(WsChannelState::new("review"));
    let handle = start_review_stream_with_state(
        channel_state,
        move |snapshot| {
            if runtime.connection.current() && revision.try_get_untracked().is_some()
                && apply_snapshot(runtime, snapshot) {
                revision.update(|value| *value = value.wrapping_add(1));
                request.cancel();
                reading.set(false);
                initialized.set(true);
                retry_until_ms.set(None);
            }
        },
        move |problem| {
            if !runtime.connection.current() || revision.try_get_untracked().is_none() { return; }
            revision.update(|value| *value = value.wrapping_add(1));
            apply_problem(runtime, problem);
        },
    );
    on_cleanup(move || handle.cancel());
    let fallback = use_ws_channel_fallback_polling(
        channel_state, Duration::from_secs(8), Duration::from_secs(40),
    );
    Effect::new(move |previous: Option<Interval>| {
        previous.unwrap_or_else(|| Interval::new(10_000, move || {
            tick.try_update(|value| *value = value.wrapping_add(1));
        }))
    });
    Effect::new(move |_| {
        let refresh = runtime.refresh_nonce.get();
        tick.get();
        if !runtime.connection.current() || reading.get_untracked() {
            return;
        }
        let explicit = last_refresh.get_value() != Some(refresh);
        if !explicit && !(initialized.get_untracked() && fallback.get_untracked()
            && review_poll_allowed(retry_until_ms.get_untracked(), review_now_ms())) {
            return;
        }
        last_refresh.set_value(Some(refresh));
        reading.set(true);
        let anchor = revision.get_untracked();
        request.run(|client| async move { review_read(client.review_runtime()).await }, move |result| {
            if !runtime.connection.current() { return; }
            reading.set(false);
            initialized.set(true);
            if response_is_current(revision.get_untracked(), anchor) {
                retry_until_ms.set(result.as_ref().err()
                    .and_then(|problem| review_retry_deadline_ms(problem.retry_after_ms, review_now_ms())));
                apply_result(runtime, result);
            }
        });
    });
    reading
}

fn apply_snapshot(runtime: ReviewRuntime, snapshot: ReviewRuntimeSnapshot) -> bool {
    if has_newer_generation(review_generation(runtime), Some(snapshot.generated_at_ms)) {
        return false;
    }
    // Failed reads also advance generation, so an older success cannot clear the error.
    runtime.projection_generation.set(Some(snapshot.generated_at_ms));
    let ReviewRuntimeSnapshot {
        executed,
        strategy_performance,
        ..
    } = snapshot;
    let executed = review_envelope_result(executed);
    runtime.executed_first_page.update(|state| state.apply_result(executed.clone()));
    if runtime.executed.cursor.get_untracked().is_none() && runtime.scope.get_untracked().is_none()
    {
        runtime.executed.state.update(|state| state.apply_result(executed));
    }
    runtime.perf.update(|state| state.apply_result(review_envelope_result(strategy_performance)));
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
    executed.into_iter().chain(performance).chain(runtime.projection_generation.get_untracked()).max()
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
    if runtime.executed.cursor.get_untracked().is_none() && runtime.scope.get_untracked().is_none()
    {
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
