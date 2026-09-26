use super::*;
use exchange::{
    Kraken, KrakenConfig, KrakenCredentials, KrakenFuturesCredentials, KrakenSpotCredentials,
};

const STATUS_INTERVAL: Duration = Duration::from_secs(5);

pub(super) fn spawn_kraken_private_ws(
    state: AppState,
    credentials: Option<crate::trading_service::KrakenAdapterCredentials>,
) -> Option<JoinHandle<()>> {
    let credentials = credentials?;
    if !credentials.is_configured() {
        return None;
    }
    let source = PrivateWsSession::capture(&state, "kraken");
    Some(tokio::spawn(run(state, source, config(credentials))))
}

fn config(credentials: crate::trading_service::KrakenAdapterCredentials) -> KrakenConfig {
    KrakenConfig {
        credentials: Some(KrakenCredentials {
            spot: credentials
                .spot
                .map(|(api_key, api_secret)| KrakenSpotCredentials {
                    api_key,
                    api_secret,
                }),
            futures: credentials
                .futures
                .map(|(api_key, api_secret)| KrakenFuturesCredentials {
                    api_key,
                    api_secret,
                }),
        }),
        allow_live_writes: false,
        timeout_secs: 10,
        qps: shared_types::VenueId::Kraken.defaults().qps,
        ..Default::default()
    }
}

async fn run(state: AppState, source: PrivateWsSession, config: KrakenConfig) {
    let venue = "kraken";
    {
        let Some(_account) = source.lock(&state).await else {
            return;
        };
        state.private_ws_health().record_task_started(venue);
    }
    let has_spot = config
        .credentials
        .as_ref()
        .is_some_and(|row| row.spot.is_some());
    let adapter = match Kraken::new(config) {
        Ok(adapter) => adapter,
        Err(error) => {
            let Some(_account) = source.lock(&state).await else {
                return;
            };
            state
                .private_ws_health()
                .record_auth_failed(venue, &error.to_string());
            return;
        }
    };
    let updates = if has_spot {
        match adapter.subscribe_spot_executions() {
            Ok(updates) => Some(updates),
            Err(error) => {
                let Some(_account) = source.lock(&state).await else {
                    return;
                };
                state
                    .private_ws_health()
                    .record_auth_failed(venue, &error.to_string());
                return;
            }
        }
    } else {
        None
    };
    // Health checks may await authentication; they must not hold up real-time fills.
    tokio::select! {
        _ = monitor(&state, &source, &adapter) => {},
        _ = forward_executions(&state, &source, &adapter, updates) => {},
    }
}

async fn forward_executions(
    state: &AppState,
    source: &PrivateWsSession,
    adapter: &Kraken,
    updates: Option<
        tokio::sync::broadcast::Receiver<exchange::adapters::kraken::KrakenSpotExecution>,
    >,
) {
    let Some(mut updates) = updates else {
        return std::future::pending().await;
    };
    replay_executions(state, source, adapter).await;
    loop {
        match updates.recv().await {
            Ok(event) => {
                let Some(account) = source.lock(state).await else {
                    return;
                };
                super::apply::apply_events_in_session(
                    state,
                    "kraken",
                    crate::trading_service::private_ws_mapper::map_kraken_spot_execution(event),
                    source,
                    account,
                )
                .await
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                warn!(skipped, "Kraken execution consumer lagged; replaying bounded local receipts, not resubmitting orders");
                replay_executions(state, source, adapter).await;
            }
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

async fn replay_executions(state: &AppState, source: &PrivateWsSession, adapter: &Kraken) {
    let Some(account) = source.lock(state).await else {
        return;
    };
    if let Ok(snapshot) = adapter.spot_execution_snapshot() {
        let events = snapshot
            .into_iter()
            .flat_map(crate::trading_service::private_ws_mapper::map_kraken_spot_execution)
            .collect();
        super::apply::apply_events_in_session(state, "kraken", events, source, account).await;
    }
}

async fn monitor(state: &AppState, source: &PrivateWsSession, adapter: &Kraken) {
    let venue = "kraken";
    let health = state.private_ws_health();
    let mut announced = false;
    let mut tick = tokio::time::interval(STATUS_INTERVAL);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);
    loop {
        tick.tick().await;
        let result = adapter.warm_private_ws().await;
        let Some(_account) = source.lock(state).await else {
            return;
        };
        match result {
            Ok(status) => {
                health.record_connected(venue);
                if !announced {
                    health.record_subscribe_attempt(venue, status.subscriptions);
                    health.record_subscribe_sent_pending_ack(
                        venue,
                        status.subscriptions,
                        status.subscriptions,
                    );
                    announced = true;
                }
                super::gate_crossex::record_samples(health, venue, status);
            }
            Err(error) => health.record_disconnected(venue, &error.to_string()),
        }
    }
}
