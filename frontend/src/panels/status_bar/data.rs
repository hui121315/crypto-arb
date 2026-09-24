use crate::api::ws::{start_system_stream_with_state, WsChannelState};
use crate::state::context::use_global;
use crate::state::load_state::LoadState;
use crate::state::polling::{use_conditional_polling_load_state, use_ws_channel_fallback_polling};
use crate::state::resource_polling::use_conditional_resource_load_state;
use leptos::prelude::*;
use shared_types::{
    OpportunityStreamEvent, SystemHealth, TradingStatusResponse, VenueOperationHealthSnapshot,
};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const WS_POLL_GRACE: Duration = Duration::from_secs(8);
const WS_STALE_AFTER: Duration = Duration::from_secs(10);

#[derive(Clone, Copy)]
pub(crate) struct ScanStatusState {
    pub(crate) state: RwSignal<LoadState<OpportunityStreamEvent>>,
}

#[derive(Clone, Copy)]
pub(crate) struct VenueOperationHealthState {
    pub(crate) state: RwSignal<LoadState<VenueOperationHealthSnapshot>>,
}

#[derive(Clone, Copy)]
pub(crate) struct SystemHealthState {
    pub(crate) state: RwSignal<LoadState<SystemHealth>>,
    pub(crate) ws_channel: RwSignal<WsChannelState>,
}

pub(crate) fn use_scan_status_state() -> ScanStatusState {
    let stream = use_global().arbitrage_stream;
    ScanStatusState {
        state: stream.state,
    }
}

pub(crate) fn use_system_health_state() -> SystemHealthState {
    let client = use_global().client;
    let ws_channel_state = RwSignal::new(WsChannelState::new("system"));
    let poll_enabled =
        use_ws_channel_fallback_polling(ws_channel_state, WS_POLL_GRACE, WS_STALE_AFTER);
    let health =
        use_conditional_resource_load_state(POLL_INTERVAL, move || poll_enabled.get_untracked(), {
            let fetch_client = client;
            move || {
                let client = fetch_client.clone();
                async move {
                    client
                        .system_health_envelope()
                        .await
                        .map_err(|error| error.problem)
                }
            }
        });

    let handle = start_system_stream_with_state(
        ws_channel_state,
        move |latest| health.set(LoadState::Ready(latest)),
        move |problem| health.update(|state| state.apply_result(Err(problem))),
    );
    on_cleanup(move || handle.cancel());
    SystemHealthState {
        state: health,
        ws_channel: ws_channel_state,
    }
}

pub fn use_trading_status_state() -> RwSignal<LoadState<TradingStatusResponse>> {
    crate::state::trading_status::provide_trading_status().state
}

pub(crate) fn use_venue_operation_health_state() -> VenueOperationHealthState {
    let client = use_global().client;
    let state = use_conditional_polling_load_state(
        POLL_INTERVAL,
        || true,
        move || {
            let client = client.clone();
            async move {
                client
                    .venue_operation_health()
                    .await
                    .map_err(|error| error.problem)
            }
        },
    );
    VenueOperationHealthState { state }
}
