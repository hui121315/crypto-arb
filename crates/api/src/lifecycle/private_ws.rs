use super::tasks::{tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::services::trading_credentials;
use crate::state::AppState;
use exchange::adapters::{
    binance_spot_ws_user, binance_ws_user, bitget_uta_ws_user as bitget_ws_user, bybit_ws_user,
    gate_spot_ws_user, gate_ws_user, hyperliquid_ws_user, kucoin_spot_ws_user, kucoin_ws_user,
    okx_ws_user,
};
use exchange::{
    Binance, BinanceConfig, BinanceCredentials, ExchangeError, ExchangeResult, HttpClient,
    WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing,
};
use reqwest::Method;
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

const GATE_API_BASE: &str = "https://api.gateio.ws";
const GATE_ACCOUNT_DETAIL_PATH: &str = "/api/v4/account/detail";
const KUCOIN_FUTURES_BASE: &str = "https://api-futures.kucoin.com";
const KUCOIN_SPOT_BASE: &str = "https://api.kucoin.com";
const DEFAULT_KUCOIN_PING_MS: u64 = 18_000;
const HYPERLIQUID_PRIVATE_WS_SUBSCRIPTIONS: usize = 13;
const PRIVATE_WS_REFRESH_INTERVAL: Duration = Duration::from_secs(15);

mod account_recovery;
mod apply;
mod binance;
mod binance_spot;
mod gate;
mod gate_crossex;
mod gate_spot;
mod kraken;
mod kucoin;
mod kucoin_session;
mod kucoin_spot;
mod plain_venues;
mod supervisor;
#[cfg(test)]
mod tests;
mod transport;

use binance::spawn_binance_user_stream;
use binance_spot::spawn_binance_spot_user_stream;
use gate::spawn_gate_private_ws;
use gate_crossex::spawn_gate_crossex_private_ws;
use gate_spot::spawn_gate_spot_private_ws;
use kraken::spawn_kraken_private_ws;
use kucoin::spawn_kucoin_private_ws;
use kucoin_spot::spawn_kucoin_spot_private_ws;
use plain_venues::{
    spawn_bitget_private_ws, spawn_bybit_private_ws, spawn_hyperliquid_private_ws,
    spawn_okx_private_ws,
};
use supervisor::VenueTask;

pub(super) fn spawn_supervisor(state: &AppState, tasks: &mut BackgroundTasks) {
    account_recovery::spawn_worker(state, tasks);
    let shutdown = tasks.shutdown_token();
    let state = state.clone();
    tasks.supervise(
        "private_ws_supervisor",
        PRIVATE_WS_REFRESH_INTERVAL.as_millis() as i64,
        move || {
            let state = state.clone();
            let shutdown = shutdown.clone();
            async move { run_supervisor(state, shutdown).await }
        },
    );
    info!(
        period_secs = PRIVATE_WS_REFRESH_INTERVAL.as_secs(),
        "private ws supervisor started"
    );
}

async fn run_supervisor(state: AppState, shutdown: ShutdownToken) {
    let registry = state.task_registry().clone();
    let mut active = PrivateWsActiveSet::default();
    let mut tick = tokio::time::interval(PRIVATE_WS_REFRESH_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    while tick_or_shutdown(&mut tick, &shutdown).await {
        refresh_private_ws(&state, &mut active).await;
        registry.record_tick("private_ws_supervisor");
    }
    // Dropping `active` aborts each venue connection through its JoinHandle.
}

async fn refresh_private_ws(state: &AppState, active: &mut PrivateWsActiveSet) {
    let credentials = trading_credentials::current_adapter_credentials();
    active.reconcile_binance(state, credentials.binance_live);
    active.reconcile_okx(state, credentials.okx_live);
    active.reconcile_bybit(state, credentials.bybit_live);
    active.reconcile_bitget(state, credentials.bitget_live);
    active.reconcile_gate(state, credentials.gate_live);
    active.reconcile_gate_crossex(state, credentials.gate_crossex_live);
    active.reconcile_kucoin(state, credentials.kucoin_live);
    active.reconcile_kraken(state, credentials.kraken_live);
    active.reconcile_hyperliquid(state, credentials.hyperliquid_live);
}

#[derive(Default)]
struct PrivateWsActiveSet {
    binance: VenueTask<Option<(String, String)>>,
    binance_spot: VenueTask<Option<(String, String)>>,
    okx: VenueTask<Option<(String, String, String)>>,
    bybit: VenueTask<Option<(String, String)>>,
    bitget: VenueTask<Option<(String, String, String)>>,
    gate: VenueTask<Option<(String, String)>>,
    gate_spot: VenueTask<Option<(String, String)>>,
    gate_crossex: VenueTask<Option<(String, String)>>,
    kucoin: VenueTask<Option<(String, String, String)>>,
    kucoin_spot: VenueTask<Option<(String, String, String)>>,
    kraken: VenueTask<Option<crate::trading_service::KrakenAdapterCredentials>>,
    hyperliquid: VenueTask<Option<crate::trading_service::HyperliquidAdapterCredentials>>,
}

impl PrivateWsActiveSet {
    fn reconcile_binance(&mut self, state: &AppState, credentials: Option<(String, String)>) {
        self.binance.reconcile(
            "binance",
            state.private_ws_health(),
            credentials.clone(),
            |credentials| spawn_binance_user_stream(state.clone(), credentials),
        );
        self.binance_spot.reconcile(
            "binance",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_binance_spot_user_stream(state.clone(), credentials),
        );
    }

    fn reconcile_okx(&mut self, state: &AppState, credentials: Option<(String, String, String)>) {
        self.okx.reconcile(
            "okx",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_okx_private_ws(state.clone(), credentials),
        );
    }

    fn reconcile_bybit(&mut self, state: &AppState, credentials: Option<(String, String)>) {
        self.bybit.reconcile(
            "bybit",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_bybit_private_ws(state.clone(), credentials),
        );
    }

    fn reconcile_bitget(
        &mut self,
        state: &AppState,
        credentials: Option<(String, String, String)>,
    ) {
        self.bitget.reconcile(
            "bitget",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_bitget_private_ws(state.clone(), credentials),
        );
    }

    fn reconcile_gate(&mut self, state: &AppState, credentials: Option<(String, String)>) {
        self.gate.reconcile(
            "gate",
            state.private_ws_health(),
            credentials.clone(),
            |credentials| spawn_gate_private_ws(state.clone(), credentials),
        );
        self.gate_spot.reconcile(
            "gate",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_gate_spot_private_ws(state.clone(), credentials),
        );
    }

    fn reconcile_gate_crossex(&mut self, state: &AppState, credentials: Option<(String, String)>) {
        self.gate_crossex.reconcile(
            "gate_crossex",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_gate_crossex_private_ws(state.clone(), credentials),
        );
    }

    fn reconcile_kucoin(
        &mut self,
        state: &AppState,
        credentials: Option<(String, String, String)>,
    ) {
        self.kucoin.reconcile(
            "kucoin",
            state.private_ws_health(),
            credentials.clone(),
            |credentials| spawn_kucoin_private_ws(state.clone(), credentials),
        );
        self.kucoin_spot.reconcile(
            "kucoin",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_kucoin_spot_private_ws(state.clone(), credentials),
        );
    }

    fn reconcile_kraken(
        &mut self,
        state: &AppState,
        credentials: Option<crate::trading_service::KrakenAdapterCredentials>,
    ) {
        self.kraken.reconcile(
            "kraken",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_kraken_private_ws(state.clone(), credentials),
        );
    }

    fn reconcile_hyperliquid(
        &mut self,
        state: &AppState,
        credentials: Option<crate::trading_service::HyperliquidAdapterCredentials>,
    ) {
        self.hyperliquid.reconcile(
            "hyperliquid",
            state.private_ws_health(),
            credentials,
            |credentials| spawn_hyperliquid_private_ws(state.clone(), credentials),
        );
    }
}
