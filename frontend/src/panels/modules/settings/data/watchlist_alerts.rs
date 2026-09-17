use std::time::Duration;

use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::{use_ws_channel_context_snapshot_fallback, SnapshotFallbackTiming};
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
    let client = use_global().client;
    let runtime = use_watchlist_alert_runtime();
    let watchlist = RwSignal::new(LoadState::Loading);
    let alert_rules = RwSignal::new(LoadState::Loading);
    let route_available = RwSignal::new(false);
    let timing = SnapshotFallbackTiming {
        period: WATCHLIST_ALERT_POLL_INTERVAL,
        grace: WATCHLIST_ALERT_POLL_GRACE,
        stale_after: WATCHLIST_ALERT_STALE_AFTER,
    };
    use_ws_channel_context_snapshot_fallback(
        runtime.watchlist_channel,
        timing,
        move || route_available.get_untracked(),
        {
            let client = client.clone();
            move || {
                let client = client.clone();
                async move {
                    let started_at_ms = crate::api::ws::now_ms();
                    (
                        started_at_ms,
                        client
                            .watchlist_quiet()
                            .await
                            .map_err(|error| error.problem),
                    )
                }
            }
        },
        move |started_at_ms, result| {
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
    use_ws_channel_context_snapshot_fallback(
        runtime.alerts_channel,
        timing,
        move || route_available.get_untracked(),
        move || {
            let client = client.clone();
            async move {
                let started_at_ms = crate::api::ws::now_ms();
                (
                    started_at_ms,
                    client
                        .alert_rules_quiet()
                        .await
                        .map_err(|error| error.problem),
                )
            }
        },
        move |started_at_ms, result| {
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
        if let Some(envelope) = runtime.watchlist.get() {
            watchlist.set(LoadState::Ready(envelope));
        }
        if let Some(envelope) = runtime.alert_rules.get() {
            alert_rules.set(LoadState::Ready(envelope));
        }
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
    if latest_ws_received_at_ms
        .get_untracked()
        .is_some_and(|received_at_ms| received_at_ms >= request_started_at_ms)
    {
        return;
    }
    state.update(|state| state.apply_result(result));
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
                surface_available: RwSignal::new(Some(true)),
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
