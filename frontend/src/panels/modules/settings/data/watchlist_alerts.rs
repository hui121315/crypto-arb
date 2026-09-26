use std::time::Duration;

use crate::state::load_state::LoadState;
use crate::state::polling::{now_ms, polling_allowed, retry_deadline_ms, use_ws_channel_fallback_polling};
use crate::state::read_scope::{bounded_read, ReadScope};
use crate::state::watchlist_alerts::{use_watchlist_alert_runtime, WatchlistAlertRuntime};
use leptos::prelude::*;
use shared_types::{AlertRulesEnvelope, WatchlistEnvelope};

const WATCHLIST_ALERT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const WATCHLIST_ALERT_POLL_GRACE: Duration = Duration::from_secs(8);
const WATCHLIST_ALERT_STALE_AFTER: Duration = Duration::from_secs(15);

#[derive(Clone, Copy)]
pub(in crate::panels::modules::settings) struct SettingsWatchlistAlertState {
    pub watchlist: RwSignal<LoadState<WatchlistEnvelope>>,
    pub alert_rules: RwSignal<LoadState<AlertRulesEnvelope>>,
    pub runtime: WatchlistAlertRuntime,
}

pub(in crate::panels::modules::settings) fn use_watchlist_alert_state(
) -> SettingsWatchlistAlertState {
    let runtime = use_watchlist_alert_runtime();
    let watchlist = RwSignal::new(LoadState::Loading);
    let alert_rules = RwSignal::new(LoadState::Loading);
    let route_available = RwSignal::new(false);
    let watchlist_retry = RwSignal::new(None);
    let alerts_retry = RwSignal::new(None);
    let scope = ReadScope::new(move || {
        watchlist.set(LoadState::Loading);
        alert_rules.set(LoadState::Loading);
        watchlist_retry.set(None);
        alerts_retry.set(None);
    });
    let watchlist_fallback = use_ws_channel_fallback_polling(
        runtime.watchlist_channel,
        WATCHLIST_ALERT_POLL_GRACE,
        WATCHLIST_ALERT_STALE_AFTER,
    );
    let alerts_fallback = use_ws_channel_fallback_polling(
        runtime.alerts_channel,
        WATCHLIST_ALERT_POLL_GRACE,
        WATCHLIST_ALERT_STALE_AFTER,
    );
    scope.poll(
        WATCHLIST_ALERT_POLL_INTERVAL,
        move || route_available.get() && polling_allowed(watchlist_fallback.get(), watchlist_retry.get_untracked(), now_ms()),
        move |client| {
            let version = runtime.connection.get_untracked();
            async move {
                let started_at_ms = crate::api::ws::now_ms();
                (started_at_ms, version, bounded_read(client.watchlist_quiet()).await)
            }
        },
        move |(started_at_ms, version, result)| {
            if !runtime.current_connection(version)
                || newer_stream_received(runtime.watchlist_received_at_ms, started_at_ms)
            {
                return;
            }
            watchlist_retry.set(result.as_ref().err().and_then(|p| retry_deadline_ms(p.retry_after_ms, now_ms())));
            if disable_route_after_not_found(route_available, runtime, &result) {
                return;
            }
            apply_rest_fallback(
                watchlist,
                runtime.watchlist_received_at_ms,
                started_at_ms,
                result,
            );
        },
    );
    scope.poll(
        WATCHLIST_ALERT_POLL_INTERVAL,
        move || route_available.get() && polling_allowed(alerts_fallback.get(), alerts_retry.get_untracked(), now_ms()),
        move |client| {
            let version = runtime.connection.get_untracked();
            async move {
                let started_at_ms = crate::api::ws::now_ms();
                (started_at_ms, version, bounded_read(client.alert_rules_quiet()).await)
            }
        },
        move |(started_at_ms, version, result)| {
            if !runtime.current_connection(version)
                || newer_stream_received(runtime.alert_rules_received_at_ms, started_at_ms)
            {
                return;
            }
            alerts_retry.set(result.as_ref().err().and_then(|p| retry_deadline_ms(p.retry_after_ms, now_ms())));
            if disable_route_after_not_found(route_available, runtime, &result) {
                return;
            }
            apply_rest_fallback(
                alert_rules,
                runtime.alert_rules_received_at_ms,
                started_at_ms,
                result,
            );
        },
    );
    Effect::new(move |_| {
        route_available.set(optional_route_enabled(runtime.surface_available.get()));
    });
    Effect::new(move |_| {
        watchlist.set(runtime.watchlist.get().map(LoadState::Ready).unwrap_or(LoadState::Loading));
        watchlist_retry.set(None);
    });
    Effect::new(move |_| {
        alert_rules.set(runtime.alert_rules.get().map(LoadState::Ready).unwrap_or(LoadState::Loading));
        alerts_retry.set(None);
    });
    SettingsWatchlistAlertState {
        watchlist,
        alert_rules,
        runtime,
    }
}

fn disable_route_after_not_found<T>(
    route_available: RwSignal<bool>,
    runtime: WatchlistAlertRuntime,
    result: &Result<T, shared_types::ApiProblem>,
) -> bool {
    if matches!(result, Err(problem) if problem.status == Some(404)) {
        route_available.set(false);
        runtime.surface_available.set(Some(false));
        return true;
    }
    false
}

fn optional_route_enabled(surface_available: Option<bool>) -> bool {
    surface_available == Some(true)
}

fn apply_rest_fallback<T: Clone + Send + Sync + 'static>(
    state: RwSignal<LoadState<T>>,
    latest_ws_received_at_ms: RwSignal<Option<u64>>,
    request_started_at_ms: u64,
    result: Result<T, shared_types::ApiProblem>,
) {
    if newer_stream_received(latest_ws_received_at_ms, request_started_at_ms) {
        return;
    }
    state.update(|state| state.apply_result(result));
}

fn newer_stream_received(
    latest_ws_received_at_ms: RwSignal<Option<u64>>,
    request_started_at_ms: u64,
) -> bool {
    latest_ws_received_at_ms
        .get_untracked()
        .is_some_and(|received_at_ms| received_at_ms >= request_started_at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delayed_rest_response_cannot_replace_newer_ws_snapshot() {
        let owner = Owner::new();
        owner.with(|| {
            let state = RwSignal::new(LoadState::Ready("ws"));
            let ws_received_at_ms = RwSignal::new(Some(20));

            apply_rest_fallback(state, ws_received_at_ms, 10, Ok("rest"));

            assert_eq!(state.get_untracked(), LoadState::Ready("ws"));
        });
    }

    #[test]
    fn missing_feature_route_stops_rest_fallback_polling() {
        let owner = Owner::new();
        owner.with(|| {
            let route_available = RwSignal::new(true);
            let runtime = WatchlistAlertRuntime {
                connection: RwSignal::new(0),
                surface_available: RwSignal::new(Some(true)),
                probe_problem: RwSignal::new(None),
                probe_reading: RwSignal::new(false),
                retry_probe: Callback::new(|()| {}),
                watchlist: RwSignal::new(None),
                alert_rules: RwSignal::new(None),
                watchlist_received_at_ms: RwSignal::new(None),
                alert_rules_received_at_ms: RwSignal::new(None),
                last_notification: RwSignal::new(None),
                watchlist_channel: RwSignal::new(crate::api::ws::WsChannelState::new("watchlist")),
                alerts_channel: RwSignal::new(crate::api::ws::WsChannelState::new("alerts")),
            };
            let result = Err::<(), _>(
                shared_types::ApiProblem::new("HTTP_ERROR", "route missing").with_status(404),
            );

            assert!(disable_route_after_not_found(
                route_available,
                runtime,
                &result
            ));

            assert!(!route_available.get_untracked());
            assert_eq!(runtime.surface_available.get_untracked(), Some(false));
        });
    }

    #[test]
    fn optional_polling_waits_for_a_positive_surface_probe() {
        assert!(!optional_route_enabled(None));
        assert!(!optional_route_enabled(Some(false)));
        assert!(optional_route_enabled(Some(true)));
    }
}
