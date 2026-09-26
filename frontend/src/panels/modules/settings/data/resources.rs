//! Settings 模块只读资源 hooks 与请求版本/落态助手。

use crate::api::rest::{ApiClient, ApiError, TradingAdaptersResponse, TradingStatusResponse};
use crate::state::load_state::LoadState;
use crate::state::read_scope::{bounded_read, ReadScope};
use leptos::prelude::*;
use shared_types::{
    AccountStateSnapshot, ActionRun, EnvTemplateResponse, ExchangeWsVenuesResponse,
    FeeScheduleRegistryResponse, FundingRatesEnvelope, MarketDataDiagnosticsSnapshot,
    RestEndpointsResponse, VenueCredentialsResponse, VenueOperationHealthSnapshot,
};

mod runtime_health;

pub(in crate::panels::modules::settings) use runtime_health::{
    use_venue_operation_health, use_venue_runtime_health,
};

pub(in crate::panels::modules::settings) type SettingsResource<T> = RwSignal<LoadState<T>>;

pub(in crate::panels::modules::settings) fn use_trading_adapters(
    refresh_nonce: RwSignal<u64>,
    state: SettingsResource<TradingAdaptersResponse>,
) {
    attach_refresh_resource(state, refresh_nonce, move |client| {
        async move { client.trading_adapters().await }
    })
}

pub(in crate::panels::modules::settings) fn use_venue_credentials(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<VenueCredentialsResponse> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.venue_credentials().await }
    })
}

pub(in crate::panels::modules::settings) fn use_account_state_snapshot(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<AccountStateSnapshot> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.trading_account_state().await }
    })
}

pub(in crate::panels::modules::settings) fn use_env_template(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<EnvTemplateResponse> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.trading_credentials_env_template().await }
    })
}

pub(in crate::panels::modules::settings) fn use_exchange_ws_venues(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<ExchangeWsVenuesResponse> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.trading_ws_venues().await }
    })
}

pub(in crate::panels::modules::settings) fn use_rest_endpoint_registry(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<RestEndpointsResponse> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.trading_rest_endpoints().await }
    })
}

pub(in crate::panels::modules::settings) fn use_fee_schedule_registry(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<FeeScheduleRegistryResponse> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.trading_fee_schedules().await }
    })
}

pub(in crate::panels::modules::settings) fn use_scoped_venue_operation_health(
    refresh_nonce: RwSignal<u64>,
    selected_venue: RwSignal<String>,
) -> SettingsResource<VenueOperationHealthSnapshot> {
    let state = RwSignal::new(LoadState::Loading);
    let loaded_venue = RwSignal::new(None::<String>);
    let scope = ReadScope::new(move || {
        loaded_venue.set(None);
        state.set(LoadState::Loading);
    });
    let request = scope.request();
    Effect::new(move |_| {
        scope.track();
        refresh_nonce.get();
        let venue = selected_venue.get();
        if venue.trim().is_empty() {
            request.cancel();
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
        let requested_venue = venue.clone();
        request.run(move |client| async move {
            settings_read(client.venue_operation_health_for_venue(&requested_venue)).await
        }, move |result| {
            if result.is_ok() || state.with_untracked(|state| state.value().is_some()) {
                loaded_venue.set(Some(venue));
            }
            state.update(|state| apply_settings_result(state, result));
        });
    });
    state
}

pub(in crate::panels::modules::settings) fn use_market_data_diagnostics(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<MarketDataDiagnosticsSnapshot> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.market_data_diagnostics().await }
    })
}

pub(in crate::panels::modules::settings) fn use_funding_rates_diagnostics(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<FundingRatesEnvelope> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.funding_rates().await }
    })
}

pub(in crate::panels::modules::settings) fn use_trading_status(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<TradingStatusResponse> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.trading_status().await }
    })
}

pub(in crate::panels::modules::settings) fn use_action_runs(
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<Vec<ActionRun>> {
    local_refresh_resource(refresh_nonce, move |client| {
        async move { client.action_runs().await }
    })
}

pub(in crate::panels::modules::settings) fn use_action_run_detail(
    selected_id: RwSignal<Option<String>>,
    refresh_nonce: RwSignal<u64>,
) -> SettingsResource<Option<ActionRun>> {
    let state = RwSignal::new(LoadState::Ready(None));
    let scope = ReadScope::new(move || state.set(LoadState::Loading));
    let request = scope.request();
    Effect::new(move |_| {
        scope.track();
        refresh_nonce.get();
        let Some(id) = selected_id.get() else {
            request.cancel();
            state.set(LoadState::Ready(None));
            return;
        };
        state.update(|state| mark_action_run_detail_loading(state, &id));
        request.run(move |client| async move {
            settings_read(client.action_run(&id)).await.map(Some)
        }, move |result| {
            state.update(|state| apply_settings_result(state, result));
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
    F: Fn(ApiClient) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<T, ApiError>> + 'static,
{
    let state = RwSignal::new(LoadState::Loading);
    attach_refresh_resource(state, refresh_nonce, fetch);
    state
}

fn attach_refresh_resource<T, F, Fut>(
    state: SettingsResource<T>,
    refresh_nonce: RwSignal<u64>,
    fetch: F,
)
where
    T: Send + Sync + 'static,
    F: Fn(ApiClient) -> Fut + 'static,
    Fut: std::future::Future<Output = Result<T, ApiError>> + 'static,
{
    let scope = ReadScope::new(move || state.set(LoadState::Loading));
    let request = scope.request();
    Effect::new(move |_| {
        scope.track();
        refresh_nonce.get();
        request.run(|client| settings_read(fetch(client)), move |result| {
            state.update(|state| apply_settings_result(state, result));
        });
    });
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

pub(in crate::panels::modules::settings) async fn settings_read<T>(
    future: impl std::future::Future<Output = Result<T, ApiError>>,
) -> Result<T, ApiError> {
    bounded_read(future).await.map_err(|problem| ApiError::from_problem(
        if problem.code == "SHARED_READ_TIMEOUT" {
            shared_types::ApiProblem::new("SETTINGS_READ_TIMEOUT",
                "读取设置超过 15 秒未返回，已停止等待；已读取的内容和未保存的输入会保留，请重试")
                .with_source("frontend.settings")
        } else { problem }
    ))
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
