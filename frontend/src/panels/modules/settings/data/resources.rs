//! Settings 模块只读资源 hooks 与请求版本/落态助手。

use crate::api::rest::{ApiError, TradingAdaptersResponse, TradingStatusResponse};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use leptos::prelude::*;
use leptos::task::spawn_local;
use shared_types::{
    AccountStateSnapshot, ActionRun, EnvTemplateResponse, ExchangeWsVenuesResponse,
    FeeScheduleRegistryResponse, FundingRatesEnvelope, MarketDataDiagnosticsSnapshot,
    RestEndpointsResponse, VenueCredentialsResponse, VenueOperationHealthSnapshot,
};

mod lifetime;
mod runtime_health;

use lifetime::settings_request_lifetime;
pub(in crate::panels::modules::settings) use runtime_health::{
    use_venue_operation_health, use_venue_runtime_health,
};

pub(in crate::panels::modules::settings) type SettingsResource<T> = RwSignal<LoadState<T>>;

pub(in crate::panels::modules::settings) fn use_trading_adapters(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<TradingAdaptersResponse> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.trading_adapters().await }
    })
}

pub(in crate::panels::modules::settings) fn use_venue_credentials(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<VenueCredentialsResponse> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.venue_credentials().await }
    })
}

pub(in crate::panels::modules::settings) fn use_account_state_snapshot(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<AccountStateSnapshot> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.trading_account_state().await }
    })
}

pub(in crate::panels::modules::settings) fn use_env_template(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<EnvTemplateResponse> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.trading_credentials_env_template().await }
    })
}

pub(in crate::panels::modules::settings) fn use_exchange_ws_venues(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<ExchangeWsVenuesResponse> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.trading_ws_venues().await }
    })
}

pub(in crate::panels::modules::settings) fn use_rest_endpoint_registry(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<RestEndpointsResponse> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.trading_rest_endpoints().await }
    })
}

pub(in crate::panels::modules::settings) fn use_fee_schedule_registry(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<FeeScheduleRegistryResponse> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.trading_fee_schedules().await }
    })
}

pub(in crate::panels::modules::settings) fn use_scoped_venue_operation_health(
    refresh_nonce: RwSignal<u64>,
    selected_venue: RwSignal<String>,
) -> SettingsResource<VenueOperationHealthSnapshot> {
    let client = use_global().client;
    let state = RwSignal::new(LoadState::Loading);
    let request_version = RwSignal::new(0_u64);
    let loaded_venue = RwSignal::new(None::<String>);
    let lifetime = settings_request_lifetime();
    Effect::new(move |_| {
        refresh_nonce.get();
        let venue = selected_venue.get();
        let version = next_request_version(request_version);
        if venue.trim().is_empty() {
            loaded_venue.set(None);
            state.set(LoadState::Ready(VenueOperationHealthSnapshot::new(
                Vec::new(),
                0,
            )));
            return;
        }
        let current_venue = loaded_venue.get_untracked();
        let keeps_value = state.with_untracked(|state| {
            scoped_venue_health_keeps_value(state, current_venue.as_deref(), &venue)
        });
        if !keeps_value {
            loaded_venue.set(None);
            state.set(LoadState::Loading);
        }
        let client = client.clone();
        let lifetime = lifetime.clone();
        spawn_local(async move {
            let result = client.venue_operation_health_for_venue(&venue).await;
            if !lifetime.is_active() {
                return;
            }
            if request_version.get_untracked() == version {
                if result.is_ok() || state.with_untracked(|state| state.value().is_some()) {
                    loaded_venue.set(Some(venue));
                }
                state.update(|state| apply_settings_result(state, result));
            }
        });
    });
    state
}

pub(in crate::panels::modules::settings) fn use_market_data_diagnostics(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<MarketDataDiagnosticsSnapshot> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.market_data_diagnostics().await }
    })
}

pub(in crate::panels::modules::settings) fn use_funding_rates_diagnostics(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<FundingRatesEnvelope> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.funding_rates().await }
    })
}

pub(in crate::panels::modules::settings) fn use_trading_status(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<TradingStatusResponse> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.trading_status().await }
    })
}

pub(in crate::panels::modules::settings) fn use_action_runs(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<Vec<ActionRun>> {
    let client = use_global().client;
    local_refresh_resource(refresh_nonce, move || {
        let client = client.clone();
        async move { client.action_runs().await }
    })
}

pub(in crate::panels::modules::settings) fn use_action_run_detail(
    selected_id: RwSignal<Option<String>>,
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<Option<ActionRun>> {
    let client = use_global().client;
    let state = RwSignal::new(LoadState::Ready(None));
    let request_version = RwSignal::new(0_u64);
    let lifetime = settings_request_lifetime();
    Effect::new(move |_| {
        refresh_nonce.get();
        let version = next_request_version(request_version);
        let Some(id) = selected_id.get() else {
            state.set(LoadState::Ready(None));
            return;
        };
        state.update(|state| mark_action_run_detail_loading(state, &id));
        let client = client.clone();
        let lifetime = lifetime.clone();
        spawn_local(async move {
            let result = client.action_run(&id).await.map(Some);
            if !lifetime.is_active() {
                return;
            }
            if request_version.get_untracked() == version {
                state.update(|state| apply_settings_result(state, result));
            }
        });
    });
    state
}

pub(in crate::panels::modules::settings) fn local_refresh_resource<T, F, Fut>(
    refresh_nonce: RwSignal<u64>,
    fetch: F,
) -> SettingsResource<T>
where
    T: Send + Sync + 'static,
    F: Fn() -> Fut + 'static,
    Fut: std::future::Future<Output = Result<T, ApiError>> + 'static,
{
    let state = RwSignal::new(LoadState::Loading);
    let request_version = RwSignal::new(0_u64);
    let lifetime = settings_request_lifetime();
    Effect::new(move |_| {
        refresh_nonce.get();
        let fut = fetch();
        let version = next_request_version(request_version);
        let lifetime = lifetime.clone();
        spawn_local(async move {
            let result = fut.await;
            if !lifetime.is_active() {
                return;
            }
            if request_version.get_untracked() == version {
                state.update(|state| apply_settings_result(state, result));
            }
        });
    });
    state
}

pub(in crate::panels::modules::settings) fn settings_state<T: Clone + Send + Sync + 'static>(
    resource: SettingsResource<T>,
) -> LoadState<T> {
    resource.get()
}

pub(in crate::panels::modules::settings) fn settings_value<T: Clone + Send + Sync + 'static>(
    resource: SettingsResource<T>,
) -> Option<T> {
    settings_state(resource).value().cloned()
}

pub(in crate::panels::modules::settings) fn bump_refresh(refresh_nonce: RwSignal<u64>) {
    refresh_nonce.update(|value| *value = value.wrapping_add(1));
}

fn next_request_version(request_version: RwSignal<u64>) -> u64 {
    let version = request_version.get_untracked().wrapping_add(1);
    request_version.set(version);
    version
}

pub(in crate::panels::modules::settings) fn apply_settings_result<T>(
    state: &mut LoadState<T>,
    result: Result<T, ApiError>,
) {
    state.apply_result(result.map_err(|error| error.problem));
}

pub(in crate::panels::modules::settings) fn scoped_venue_health_keeps_value(
    state: &LoadState<VenueOperationHealthSnapshot>,
    loaded_venue: Option<&str>,
    next_venue: &str,
) -> bool {
    state.value().is_some() && loaded_venue == Some(next_venue)
}

pub(in crate::panels::modules::settings) fn mark_action_run_detail_loading(
    state: &mut LoadState<Option<ActionRun>>,
    id: &str,
) {
    if selected_detail_matches(state, id) {
        return;
    }
    *state = LoadState::Loading;
}

fn selected_detail_matches(state: &LoadState<Option<ActionRun>>, id: &str) -> bool {
    state
        .value()
        .and_then(Option::as_ref)
        .is_some_and(|run| run.id == id)
}
