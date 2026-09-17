use crate::api::ws::{WsChannelState, WsStatus};
use crate::state::load_state::LoadState;
use gloo_timers::callback::{Interval, Timeout};
use leptos::prelude::*;
use shared_types::ApiProblem;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq)]
pub enum PollingEvent<T, Err> {
    Disabled,
    Fetched(Result<T, Err>),
}

impl<T, Err> PollingEvent<T, Err> {
    pub fn into_fetched(self) -> Option<Result<T, Err>> {
        match self {
            Self::Disabled => None,
            Self::Fetched(result) => Some(result),
        }
    }
}

pub trait ResourceEnvelope {
    fn resource_problem(&self) -> Option<ApiProblem>;
    fn resource_retry_after_ms(&self) -> Option<u64>;
    fn should_publish_value(&self) -> bool {
        self.resource_problem().is_none()
    }
}

pub fn use_resource_envelope<T, E>(state: &mut LoadState<T>, envelope: &E, value: T)
where
    E: ResourceEnvelope + ?Sized,
{
    let publish_value = envelope.should_publish_value();
    apply_resource_envelope_state(state, envelope, value, publish_value);
}

pub fn apply_resource_envelope_state<T, E>(
    state: &mut LoadState<T>,
    envelope: &E,
    value: T,
    publish_value: bool,
) where
    E: ResourceEnvelope + ?Sized,
{
    match envelope.resource_problem() {
        Some(problem) if publish_value => {
            *state = LoadState::Stale { value, problem };
        }
        Some(problem) => state.apply_result(Err(problem)),
        None => *state = LoadState::Ready(value),
    }
}

pub fn use_conditional_polling_result<T, F, Fut, E, Err>(
    period: Duration,
    enabled: E,
    fetch: F,
) -> LocalResource<PollingEvent<T, Err>>
where
    T: 'static,
    Err: 'static,
    F: Fn() -> Fut + Clone + 'static,
    Fut: std::future::Future<Output = Result<T, Err>> + 'static,
    E: Fn() -> bool + Clone + 'static,
{
    let tick = RwSignal::new(0_u64);
    let interval_ms = duration_ms(period);
    let interval_enabled = enabled.clone();
    Effect::new(move |prev: Option<Interval>| {
        if let Some(interval) = prev {
            return interval;
        }
        let enabled = interval_enabled.clone();
        Interval::new(interval_ms, move || {
            // Route-owned signals are disposed before the browser releases every
            // queued timer callback. Check the hook's own signal first so an old
            // interval never evaluates a caller guard from the previous route.
            if tick.try_get_untracked().is_some() && enabled() {
                let _ = tick.try_update(|value| *value = value.wrapping_add(1));
            }
        })
    });

    LocalResource::new(move || {
        let enabled_now = tick.try_get().is_some() && enabled();
        let fetch = fetch.clone();
        async move {
            if enabled_now {
                PollingEvent::Fetched(fetch().await)
            } else {
                PollingEvent::Disabled
            }
        }
    })
}

pub fn use_conditional_polling_load_state<T, F, Fut, E>(
    period: Duration,
    enabled: E,
    fetch: F,
) -> RwSignal<LoadState<T>>
where
    T: Clone + Send + Sync + 'static,
    F: Fn() -> Fut + Clone + 'static,
    Fut: std::future::Future<Output = Result<T, ApiProblem>> + 'static,
    E: Fn() -> bool + Clone + 'static,
{
    let state = RwSignal::new(LoadState::Loading);
    let retry_until_ms = RwSignal::new(None::<u64>);
    let poll_enabled = move || {
        let Some(retry_until_ms) = retry_until_ms.try_get_untracked() else {
            return false;
        };
        polling_allowed(enabled(), retry_until_ms, now_ms())
    };
    let poll = use_conditional_polling_result(period, poll_enabled, fetch);
    Effect::new(move |_| {
        if let Some(result) = poll.get().and_then(|event| event.take().into_fetched()) {
            retry_until_ms.set(
                result
                    .as_ref()
                    .err()
                    .and_then(|problem| retry_deadline_ms(problem.retry_after_ms, now_ms())),
            );
            state.update(|state| state.apply_result(result));
        }
    });
    state
}

pub fn use_debounced_string(
    source: impl Fn() -> String + 'static,
    delay: Duration,
) -> RwSignal<String> {
    // Hook construction can run inside a parent render effect. Keep the initial
    // read from subscribing that parent; the inner effect below owns updates.
    let value = RwSignal::new(untrack(&source));
    let pending = StoredValue::new_local(None::<Timeout>);
    let delay_ms = duration_ms(delay);
    Effect::new(move |_| {
        pending.update_value(|slot| {
            if let Some(timeout) = slot.take() {
                timeout.cancel();
            }
        });
        let next = source();
        if next == value.get_untracked() {
            return;
        }
        if next.is_empty() {
            value.set(String::new());
            return;
        }
        pending.set_value(Some(Timeout::new(delay_ms, move || value.set(next))));
    });
    on_cleanup(move || {
        pending.update_value(|slot| {
            if let Some(timeout) = slot.take() {
                timeout.cancel();
            }
        });
    });
    value
}

pub fn use_debounced_value<T>(
    source: impl Fn() -> T + 'static,
    delay: Duration,
) -> RwSignal<Option<T>>
where
    T: Clone + PartialEq + Send + Sync + 'static,
{
    let value = RwSignal::new(None::<T>);
    let pending = StoredValue::new_local(None::<Timeout>);
    let delay_ms = duration_ms(delay);
    Effect::new(move |_| {
        pending.update_value(|slot| {
            if let Some(timeout) = slot.take() {
                timeout.cancel();
            }
        });
        let next = source();
        if value.get_untracked().as_ref() == Some(&next) {
            return;
        }
        pending.set_value(Some(Timeout::new(delay_ms, move || value.set(Some(next)))));
    });
    on_cleanup(move || {
        pending.update_value(|slot| {
            if let Some(timeout) = slot.take() {
                timeout.cancel();
            }
        });
    });
    value
}

pub fn use_ws_channel_fallback_polling(
    channel_state: RwSignal<WsChannelState>,
    grace: Duration,
    stale_after: Duration,
) -> RwSignal<bool> {
    let enabled = RwSignal::new(true);
    let pending = StoredValue::new_local(None::<Timeout>);
    let grace_ms = duration_ms(grace);
    let stale_ms = duration_ms(stale_after);
    Effect::new(move |_| {
        pending.update_value(|slot| {
            if let Some(timeout) = slot.take() {
                timeout.cancel();
            }
        });
        let state = channel_state.get();
        let delay = channel_fallback_delay_ms(&state, now_ms(), grace_ms, stale_ms);
        match delay {
            None => enabled.set(true),
            Some(wait_ms) => {
                if state.status != WsStatus::Disconnected {
                    enabled.set(false);
                }
                pending.set_value(Some(Timeout::new(wait_ms, move || enabled.set(true))));
            }
        }
    });
    on_cleanup(move || {
        pending.update_value(|slot| {
            if let Some(timeout) = slot.take() {
                timeout.cancel();
            }
        });
    });
    enabled
}

/// Timing budget for a WS-backed REST snapshot fallback: how often to poll once
/// armed (`period`), how long a non-connected channel waits before arming
/// (`grace`), and how long a connected-but-silent channel waits (`stale_after`).
#[derive(Clone, Copy)]
pub struct SnapshotFallbackTiming {
    pub period: Duration,
    pub grace: Duration,
    pub stale_after: Duration,
}

/// Drive a REST snapshot fallback for a WS-backed data module.
///
/// The fallback fetch is gated by three independent signals so a healthy stream
/// suppresses polling but a half-open / silent / disconnected channel recovers a
/// snapshot by freshness:
/// - `extra_enabled`: caller-side guard (e.g. only poll when a filter is active);
/// - per-channel WS freshness via [`use_ws_channel_fallback_polling`];
/// - server `retryAfterMs` backoff via the typed [`ApiProblem`] on the last error.
///
/// `apply` receives every completed fetch result so the caller can merge it into a
/// store that never blanks an existing snapshot on error.
pub fn use_ws_channel_snapshot_fallback<T, F, Fut, G>(
    channel_state: RwSignal<WsChannelState>,
    timing: SnapshotFallbackTiming,
    extra_enabled: G,
    fetch: F,
    apply: impl Fn(Result<T, ApiProblem>) + 'static,
) where
    T: Clone + 'static,
    F: Fn() -> Fut + Clone + 'static,
    Fut: std::future::Future<Output = Result<T, ApiProblem>> + 'static,
    G: Fn() -> bool + Clone + 'static,
{
    let SnapshotFallbackTiming {
        period,
        grace,
        stale_after,
    } = timing;
    let fresh_enabled = use_ws_channel_fallback_polling(channel_state, grace, stale_after);
    let retry_until_ms = RwSignal::new(None::<u64>);
    let poll_enabled = move || {
        let Some(fresh_enabled) = fresh_enabled.try_get_untracked() else {
            return false;
        };
        let Some(retry_until_ms) = retry_until_ms.try_get_untracked() else {
            return false;
        };
        snapshot_fallback_poll_enabled(extra_enabled(), fresh_enabled, retry_until_ms, now_ms())
    };
    let poll = use_conditional_polling_result(period, poll_enabled, fetch);
    Effect::new(move |_| {
        if let Some(result) = poll.get().and_then(|event| event.take().into_fetched()) {
            retry_until_ms.set(
                result
                    .as_ref()
                    .err()
                    .and_then(|problem| retry_deadline_ms(problem.retry_after_ms, now_ms())),
            );
            apply(result);
        }
    });
}

/// Context-preserving variant for consumers whose active query can change while
/// a fallback request is in flight. The request context is delivered with both
/// success and error results while retry backoff still uses the typed problem.
pub fn use_ws_channel_context_snapshot_fallback<C, T, F, Fut, G>(
    channel_state: RwSignal<WsChannelState>,
    timing: SnapshotFallbackTiming,
    extra_enabled: G,
    fetch: F,
    apply: impl Fn(C, Result<T, ApiProblem>) + 'static,
) where
    C: Clone + 'static,
    T: Clone + 'static,
    F: Fn() -> Fut + Clone + 'static,
    Fut: std::future::Future<Output = (C, Result<T, ApiProblem>)> + 'static,
    G: Fn() -> bool + Clone + 'static,
{
    let SnapshotFallbackTiming {
        period,
        grace,
        stale_after,
    } = timing;
    let fresh_enabled = use_ws_channel_fallback_polling(channel_state, grace, stale_after);
    let retry_until_ms = RwSignal::new(None::<u64>);
    let poll_enabled = move || {
        let Some(fresh_enabled) = fresh_enabled.try_get_untracked() else {
            return false;
        };
        let Some(retry_until_ms) = retry_until_ms.try_get_untracked() else {
            return false;
        };
        snapshot_fallback_poll_enabled(extra_enabled(), fresh_enabled, retry_until_ms, now_ms())
    };
    let poll = use_conditional_polling_result(period, poll_enabled, move || {
        let fetch = fetch.clone();
        async move {
            let (context, result) = fetch().await;
            match result {
                Ok(value) => Ok((context, value)),
                Err(problem) => Err((context, problem)),
            }
        }
    });
    Effect::new(move |_| {
        if let Some(result) = poll.get().and_then(|event| event.take().into_fetched()) {
            retry_until_ms.set(
                result
                    .as_ref()
                    .err()
                    .and_then(|(_, problem)| retry_deadline_ms(problem.retry_after_ms, now_ms())),
            );
            match result {
                Ok((context, value)) => apply(context, Ok(value)),
                Err((context, problem)) => apply(context, Err(problem)),
            }
        }
    });
}

fn channel_fallback_delay_ms(
    state: &WsChannelState,
    now_ms: u64,
    grace_ms: u32,
    stale_ms: u32,
) -> Option<u32> {
    if state.status != WsStatus::Connected {
        return Some(grace_ms);
    }
    let Some(last_message_at_ms) = state.last_message_at_ms else {
        return Some(grace_ms);
    };
    let elapsed = now_ms.saturating_sub(last_message_at_ms);
    if elapsed >= u64::from(stale_ms) {
        return None;
    }
    Some((u64::from(stale_ms) - elapsed).min(u64::from(u32::MAX)) as u32)
}

fn duration_ms(period: Duration) -> u32 {
    period.as_millis().clamp(250, u32::MAX as u128) as u32
}

pub(crate) fn now_ms() -> u64 {
    js_sys::Date::now().max(0.0).round().min(u64::MAX as f64) as u64
}

pub(crate) fn polling_allowed(enabled: bool, retry_until_ms: Option<u64>, now_ms: u64) -> bool {
    enabled && retry_until_ms.is_none_or(|until_ms| now_ms >= until_ms)
}

/// Combine the caller guard, per-channel WS freshness, and server retry backoff
/// into a single snapshot-fallback poll decision. A healthy stream
/// (`fresh_enabled == false`) suppresses polling even while the caller guard is
/// active, so a refresh never blanks an existing snapshot.
fn snapshot_fallback_poll_enabled(
    extra_enabled: bool,
    fresh_enabled: bool,
    retry_until_ms: Option<u64>,
    now_ms: u64,
) -> bool {
    extra_enabled && polling_allowed(fresh_enabled, retry_until_ms, now_ms)
}

pub(crate) fn retry_deadline_ms(retry_after_ms: Option<u64>, now_ms: u64) -> Option<u64> {
    retry_after_ms
        .filter(|retry_after_ms| *retry_after_ms > 0)
        .map(|retry_after_ms| now_ms.saturating_add(retry_after_ms))
}

pub(crate) fn max_retry_after_ms<'a>(
    direct_retry_after_ms: Option<u64>,
    problems: impl IntoIterator<Item = &'a ApiProblem>,
) -> Option<u64> {
    direct_retry_after_ms
        .into_iter()
        .chain(
            problems
                .into_iter()
                .filter_map(|problem| problem.retry_after_ms),
        )
        .max()
}

#[cfg(test)]
mod tests {
    use super::{
        channel_fallback_delay_ms, max_retry_after_ms, polling_allowed, retry_deadline_ms,
        snapshot_fallback_poll_enabled, use_resource_envelope, PollingEvent, ResourceEnvelope,
        WsChannelState, WsStatus,
    };
    use crate::state::load_state::LoadState;
    use shared_types::ApiProblem;

    #[derive(Clone)]
    struct TestEnvelope {
        publish: bool,
        problem: Option<ApiProblem>,
        retry_after_ms: Option<u64>,
    }

    impl ResourceEnvelope for TestEnvelope {
        fn resource_problem(&self) -> Option<ApiProblem> {
            self.problem.clone()
        }

        fn resource_retry_after_ms(&self) -> Option<u64> {
            self.retry_after_ms.or_else(|| {
                self.problem
                    .as_ref()
                    .and_then(|problem| problem.retry_after_ms)
            })
        }

        fn should_publish_value(&self) -> bool {
            self.publish
        }
    }

    #[test]
    fn polling_event_keeps_disabled_distinct_from_failed_fetch() {
        let disabled = PollingEvent::<u32, &str>::Disabled;
        let failed = PollingEvent::<u32, &str>::Fetched(Err("rate limited"));

        assert_eq!(disabled.into_fetched(), None);
        assert_eq!(failed.into_fetched(), Some(Err("rate limited")));
    }

    #[test]
    fn resource_envelope_preserves_stale_value_and_typed_problem() {
        let mut state = LoadState::Ready(7);
        let envelope = TestEnvelope {
            publish: false,
            problem: Some(
                ApiProblem::new("RATE_LIMITED", "rate limited")
                    .with_request_id(Some("req-envelope-1".into()))
                    .with_retry_after_ms(Some(2_000)),
            ),
            retry_after_ms: None,
        };

        use_resource_envelope(&mut state, &envelope, 9);

        assert!(matches!(state, LoadState::Stale { .. }));
        if let LoadState::Stale { value, problem } = state {
            assert_eq!(value, 7);
            assert_eq!(problem.code, "RATE_LIMITED");
            assert_eq!(problem.request_id.as_deref(), Some("req-envelope-1"));
            assert_eq!(problem.retry_after_ms, Some(2_000));
        }
        assert_eq!(envelope.resource_retry_after_ms(), Some(2_000));
    }

    #[test]
    fn resource_envelope_publishes_degraded_payload_as_stale() {
        let mut state = LoadState::Loading;
        let envelope = TestEnvelope {
            publish: true,
            problem: Some(
                ApiProblem::new("PARTIAL", "partial")
                    .with_request_id(Some("req-envelope-2".into()))
                    .with_retry_after_ms(Some(5_000)),
            ),
            retry_after_ms: Some(6_000),
        };

        use_resource_envelope(&mut state, &envelope, 11);

        assert!(matches!(state, LoadState::Stale { .. }));
        if let LoadState::Stale { value, problem } = state {
            assert_eq!(value, 11);
            assert_eq!(problem.request_id.as_deref(), Some("req-envelope-2"));
            assert_eq!(problem.retry_after_ms, Some(5_000));
        }
        assert_eq!(envelope.resource_retry_after_ms(), Some(6_000));
    }

    #[test]
    fn resource_envelope_without_prior_value_is_error() {
        let mut state = LoadState::<u32>::Loading;
        let envelope = TestEnvelope {
            publish: false,
            problem: Some(ApiProblem::new("UPSTREAM", "upstream failed")),
            retry_after_ms: None,
        };

        use_resource_envelope(&mut state, &envelope, 3);

        assert!(matches!(state, LoadState::Error(_)));
        assert_eq!(
            state.problem().map(|problem| problem.code.as_str()),
            Some("UPSTREAM")
        );
    }

    #[test]
    fn channel_fallback_waits_until_connected_channel_is_stale() {
        let mut state = WsChannelState::new("system");
        state.status = WsStatus::Connected;
        state.subscribed = true;
        state.last_message_at_ms = Some(1_000);

        assert_eq!(
            channel_fallback_delay_ms(&state, 1_500, 8_000, 5_000),
            Some(4_500)
        );
        assert_eq!(channel_fallback_delay_ms(&state, 6_000, 8_000, 5_000), None);
    }

    #[test]
    fn channel_fallback_graces_connected_channel_without_messages() {
        let mut state = WsChannelState::new("portfolio");
        state.status = WsStatus::Connected;
        state.subscribed = true;

        assert_eq!(
            channel_fallback_delay_ms(&state, 1_000, 8_000, 5_000),
            Some(8_000)
        );
    }

    #[test]
    fn polling_retry_deadline_uses_typed_retry_after() {
        assert_eq!(retry_deadline_ms(Some(2_000), 10_000), Some(12_000));
        assert_eq!(retry_deadline_ms(Some(0), 10_000), None);
        assert_eq!(retry_deadline_ms(None, 10_000), None);
    }

    #[test]
    fn polling_retry_after_prefers_longest_problem_deadline() {
        let short = ApiProblem::new("SHORT", "short").with_retry_after_ms(Some(1_000));
        let long = ApiProblem::new("LONG", "long").with_retry_after_ms(Some(3_000));

        assert_eq!(
            max_retry_after_ms(Some(2_000), [&short, &long]),
            Some(3_000)
        );
        assert_eq!(max_retry_after_ms(Some(0), [&short]), Some(1_000));
    }

    #[test]
    fn polling_allowed_waits_until_retry_deadline() {
        assert!(!polling_allowed(true, Some(12_000), 11_999));
        assert!(polling_allowed(true, Some(12_000), 12_000));
        assert!(!polling_allowed(false, Some(12_000), 12_000));
    }

    #[test]
    fn snapshot_fallback_polls_only_when_caller_guard_and_freshness_agree() {
        // Healthy stream (fresh_enabled == false) suppresses the REST snapshot
        // fallback even while the caller guard is active.
        assert!(!snapshot_fallback_poll_enabled(true, false, None, 10_000));
        // Caller guard closed (e.g. no active filter) suppresses polling even
        // when the channel is stale/disconnected.
        assert!(!snapshot_fallback_poll_enabled(false, true, None, 10_000));
        // Stale/disconnected channel with an active guard recovers a snapshot.
        assert!(snapshot_fallback_poll_enabled(true, true, None, 10_000));
        // Server retry backoff still defers the fallback until the deadline.
        assert!(!snapshot_fallback_poll_enabled(
            true,
            true,
            Some(12_000),
            11_999
        ));
        assert!(snapshot_fallback_poll_enabled(
            true,
            true,
            Some(12_000),
            12_000
        ));
    }
}
