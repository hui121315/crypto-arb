use crate::api::ws::{
    start_alert_stream_with_state, start_watchlist_stream_with_state, WsChannelState,
};
use crate::state::context::use_global;
use crate::state::{push_toast_to, use_toasts, ToastLevel, Toasts};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    AlertNotification, AlertRulesEnvelope, AlertStreamEvent, ApiProblem, WatchlistEnvelope,
    WatchlistStreamEvent, OP_STORAGE_WATCHLIST_ALERTS, OP_WATCHLIST_PREWARM,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const FEATURE_PROBE_RETRY_MS: u32 = 5_000;
type OptionalStreamHandles = (
    crate::api::ws::WsStreamHandle,
    crate::api::ws::WsStreamHandle,
);

#[derive(Clone, Copy)]
pub struct WatchlistAlertRuntime {
    pub surface_available: RwSignal<Option<bool>>,
    pub watchlist: RwSignal<Option<WatchlistEnvelope>>,
    pub alert_rules: RwSignal<Option<AlertRulesEnvelope>>,
    pub watchlist_received_at_ms: RwSignal<Option<u64>>,
    pub alert_rules_received_at_ms: RwSignal<Option<u64>>,
    pub last_notification: RwSignal<Option<AlertNotification>>,
    pub watchlist_channel: RwSignal<WsChannelState>,
    pub alerts_channel: RwSignal<WsChannelState>,
}

pub fn provide_watchlist_alert_runtime() -> WatchlistAlertRuntime {
    let client = use_global().client;
    let toasts = use_toasts();
    let runtime = WatchlistAlertRuntime {
        surface_available: RwSignal::new(None),
        watchlist: RwSignal::new(None),
        alert_rules: RwSignal::new(None),
        watchlist_received_at_ms: RwSignal::new(None),
        alert_rules_received_at_ms: RwSignal::new(None),
        last_notification: RwSignal::new(None),
        watchlist_channel: RwSignal::new(WsChannelState::new("watchlist")),
        alerts_channel: RwSignal::new(WsChannelState::new("alerts")),
    };
    let active = Arc::new(AtomicBool::new(true));
    let handles = Arc::new(Mutex::new(None));
    spawn_local({
        let active = Arc::clone(&active);
        let handles = Arc::clone(&handles);
        async move {
            probe_and_start_streams(client, runtime, toasts, active, handles).await;
        }
    });
    on_cleanup(move || {
        active.store(false, Ordering::Release);
        if let Some((watchlist, alerts)) = take_stream_handles(&handles) {
            watchlist.cancel();
            alerts.cancel();
        }
    });
    provide_context(runtime);
    runtime
}

async fn probe_and_start_streams(
    client: crate::api::rest::ApiClient,
    runtime: WatchlistAlertRuntime,
    toasts: Toasts,
    active: Arc<AtomicBool>,
    handles: Arc<Mutex<Option<OptionalStreamHandles>>>,
) {
    loop {
        match probe_optional_surfaces(&client).await {
            FeatureProbe::Available((watchlist, alert_rules)) => {
                if !active.load(Ordering::Acquire) {
                    return;
                }
                let received_at_ms = crate::api::ws::now_ms();
                runtime.surface_available.set(Some(true));
                runtime.watchlist_received_at_ms.set(Some(received_at_ms));
                runtime.alert_rules_received_at_ms.set(Some(received_at_ms));
                runtime.watchlist.set(Some(watchlist));
                runtime.alert_rules.set(Some(alert_rules));
                store_stream_handles(&active, &handles, start_streams(runtime, toasts));
                return;
            }
            FeatureProbe::Disabled => {
                runtime.surface_available.set(Some(false));
                return;
            }
            FeatureProbe::Retry => {
                TimeoutFuture::new(FEATURE_PROBE_RETRY_MS).await;
                if !active.load(Ordering::Acquire) {
                    return;
                }
            }
        }
    }
}

async fn probe_optional_surfaces(
    client: &crate::api::rest::ApiClient,
) -> FeatureProbe<(WatchlistEnvelope, AlertRulesEnvelope)> {
    let Ok(health) = client.venue_operation_health().await else {
        return FeatureProbe::Retry;
    };
    let configured = watchlist_surface_configured(
        health
            .rows
            .iter()
            .map(|row| (row.operation.as_str(), row.configured)),
    );
    if !should_fetch_optional_surfaces(configured) {
        return FeatureProbe::Disabled;
    }
    classify_feature_probe(fetch_optional_surfaces(client).await)
}

async fn fetch_optional_surfaces(
    client: &crate::api::rest::ApiClient,
) -> Result<(WatchlistEnvelope, AlertRulesEnvelope), ApiProblem> {
    let watchlist = client
        .watchlist_quiet()
        .await
        .map_err(|error| error.problem)?;
    let alert_rules = client
        .alert_rules_quiet()
        .await
        .map_err(|error| error.problem)?;
    Ok((watchlist, alert_rules))
}

fn watchlist_surface_configured<'a>(
    rows: impl IntoIterator<Item = (&'a str, Option<bool>)>,
) -> Option<bool> {
    let mut values = rows
        .into_iter()
        .filter(|(operation, _)| {
            matches!(
                *operation,
                OP_WATCHLIST_PREWARM | OP_STORAGE_WATCHLIST_ALERTS
            )
        })
        .filter_map(|(_, configured)| configured);
    let configured = values.next()?;
    values
        .all(|value| value == configured)
        .then_some(configured)
}

fn should_fetch_optional_surfaces(configured: Option<bool>) -> bool {
    configured != Some(false)
}

fn start_streams(
    runtime: WatchlistAlertRuntime,
    toasts: Toasts,
) -> (
    crate::api::ws::WsStreamHandle,
    crate::api::ws::WsStreamHandle,
) {
    let watchlist_handle = start_watchlist_stream_with_state(
        runtime.watchlist_channel,
        move |event| {
            let WatchlistStreamEvent::WatchlistChanged { envelope, .. } = event;
            runtime
                .watchlist_received_at_ms
                .set(Some(crate::api::ws::now_ms()));
            runtime.watchlist.set(Some(envelope));
        },
        |_| {},
    );
    let alert_handle = start_alert_stream_with_state(
        runtime.alerts_channel,
        move |event| apply_alert_event(runtime, toasts, event),
        |_| {},
    );
    (watchlist_handle, alert_handle)
}

fn store_stream_handles(
    active: &AtomicBool,
    handles: &Mutex<Option<OptionalStreamHandles>>,
    stream_handles: OptionalStreamHandles,
) {
    let mut guard = match handles.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if active.load(Ordering::Acquire) {
        *guard = Some(stream_handles);
    } else {
        stream_handles.0.cancel();
        stream_handles.1.cancel();
    }
}

fn take_stream_handles(
    handles: &Mutex<Option<OptionalStreamHandles>>,
) -> Option<OptionalStreamHandles> {
    match handles.lock() {
        Ok(mut guard) => guard.take(),
        Err(poisoned) => poisoned.into_inner().take(),
    }
}

enum FeatureProbe<T> {
    Available(T),
    Disabled,
    Retry,
}

fn classify_feature_probe<T>(result: Result<T, ApiProblem>) -> FeatureProbe<T> {
    match result {
        Ok(value) => FeatureProbe::Available(value),
        Err(problem) if problem.status == Some(404) => FeatureProbe::Disabled,
        Err(_) => FeatureProbe::Retry,
    }
}

pub fn use_watchlist_alert_runtime() -> WatchlistAlertRuntime {
    expect_context::<WatchlistAlertRuntime>()
}

fn apply_alert_event(runtime: WatchlistAlertRuntime, toasts: Toasts, event: AlertStreamEvent) {
    match event {
        AlertStreamEvent::AlertRulesChanged { envelope, .. } => {
            runtime
                .alert_rules_received_at_ms
                .set(Some(crate::api::ws::now_ms()));
            runtime.alert_rules.set(Some(*envelope));
        }
        AlertStreamEvent::AlertTriggered { notification } => {
            let message = alert_notification_message(&notification);
            runtime.last_notification.set(Some(notification));
            push_toast_to(toasts, ToastLevel::Info, message);
        }
    }
}

fn alert_notification_message(notification: &AlertNotification) -> String {
    format!(
        "{} {} / {} · 单次费后 {:+.3}% · 净差 {:.4}%",
        notification.symbol,
        notification.long_exchange,
        notification.short_exchange,
        notification.one_cycle_net_bps / 100.0,
        notification.net_single_yield,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_runtime() -> WatchlistAlertRuntime {
        WatchlistAlertRuntime {
            surface_available: RwSignal::new(None),
            watchlist: RwSignal::new(None),
            alert_rules: RwSignal::new(None),
            watchlist_received_at_ms: RwSignal::new(None),
            alert_rules_received_at_ms: RwSignal::new(None),
            last_notification: RwSignal::new(None),
            watchlist_channel: RwSignal::new(WsChannelState::new("watchlist")),
            alerts_channel: RwSignal::new(WsChannelState::new("alerts")),
        }
    }

    fn notification() -> AlertNotification {
        AlertNotification {
            id: "alert-1".into(),
            rule_id: 1,
            watchlist_id: 2,
            opportunity_id: "opp-1".into(),
            symbol: "BTC-USDT".into(),
            strategy: shared_types::StrategyKind::PerpCross,
            long_exchange: "binance".into(),
            short_exchange: "okx".into(),
            one_cycle_net_bps: 9.125,
            net_single_yield: 0.1234,
            queued_at_ms: 42,
        }
    }

    #[test]
    fn toast_message_uses_verified_net_profit_and_execution_legs() {
        let notification = notification();

        let message = alert_notification_message(&notification);

        assert!(message.contains("BTC-USDT"));
        assert!(message.contains("binance / okx"));
        assert!(message.contains("单次费后 +0.091%"));
        assert!(message.contains("净差 0.1234%"));
    }

    #[test]
    fn alert_event_uses_captured_toast_signal_without_context_lookup() {
        let owner = Owner::new();
        let (runtime, toasts) = owner.with(|| (test_runtime(), RwSignal::new(Vec::new())));

        apply_alert_event(
            runtime,
            toasts,
            AlertStreamEvent::AlertTriggered {
                notification: notification(),
            },
        );

        assert_eq!(toasts.get_untracked().len(), 1);
        assert!(toasts.get_untracked()[0]
            .message
            .contains("单次费后 +0.091%"));
        assert_eq!(
            runtime
                .last_notification
                .get_untracked()
                .map(|item| item.opportunity_id),
            Some("opp-1".into())
        );
    }

    #[test]
    fn optional_stream_probe_stops_after_missing_route() {
        let result = Err(ApiProblem::new("HTTP_ERROR", "route missing").with_status(404));

        assert!(matches!(
            classify_feature_probe::<()>(result),
            FeatureProbe::Disabled
        ));
    }

    #[test]
    fn optional_stream_probe_retries_transient_failures() {
        let result = Err(ApiProblem::new("FETCH_FAILED", "backend unavailable"));

        assert!(matches!(
            classify_feature_probe::<()>(result),
            FeatureProbe::Retry
        ));
    }

    #[test]
    fn optional_stream_probe_uses_consistent_health_configuration() {
        assert_eq!(
            watchlist_surface_configured([
                (OP_WATCHLIST_PREWARM, Some(false)),
                (OP_STORAGE_WATCHLIST_ALERTS, Some(false)),
            ]),
            Some(false)
        );
        assert_eq!(
            watchlist_surface_configured([
                (OP_WATCHLIST_PREWARM, Some(true)),
                (OP_STORAGE_WATCHLIST_ALERTS, Some(true)),
            ]),
            Some(true)
        );
        assert_eq!(
            watchlist_surface_configured([
                (OP_WATCHLIST_PREWARM, Some(true)),
                (OP_STORAGE_WATCHLIST_ALERTS, Some(false)),
            ]),
            None
        );
    }

    #[test]
    fn optional_stream_probe_uses_endpoint_truth_when_health_rows_lag() {
        assert!(!should_fetch_optional_surfaces(Some(false)));
        assert!(should_fetch_optional_surfaces(Some(true)));
        assert!(should_fetch_optional_surfaces(None));
    }
}
