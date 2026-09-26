use super::apply::apply_events_in_session;
use super::transport::{ws_config, AbortOnDrop};
use super::*;

pub(super) fn spawn_binance_user_stream(
    state: AppState,
    credentials: Option<(String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret) = credentials?;
    let session = PrivateWsSession::capture(&state, "binance");
    Some(tokio::spawn(async move {
        let health_state = state.clone();
        if let Err(error) =
            run_binance_user_stream(state, session.clone(), api_key, api_secret).await
        {
            let Some(_account) = session.lock(&health_state).await else {
                return;
            };
            health_state
                .private_ws_health()
                .record_disconnected("binance", &error.to_string());
            warn!(%error, "binance private ws stopped");
        }
    }))
}

async fn run_binance_user_stream(
    state: AppState,
    session: PrivateWsSession,
    api_key: String,
    api_secret: String,
) -> ExchangeResult<()> {
    {
        let Some(_account) = session.lock(&state).await else {
            return Ok(());
        };
        state.private_ws_health().record_task_started("binance");
    }
    let adapter = Binance::new(BinanceConfig {
        credentials: Some(BinanceCredentials {
            api_key,
            api_secret,
        }),
        ..BinanceConfig::default()
    })?;
    loop {
        if session.lock(&state).await.is_none() {
            return Ok(());
        }
        let listen_key = adapter.start_user_data_stream().await?;
        {
            let Some(_account) = session.lock(&state).await else {
                return Ok(());
            };
            state
                .private_ws_health()
                .record_subscribe_sent("binance", 1, 1);
        }
        let manager = Arc::new(WsManager::new(ws_config(
            "binance",
            binance_ws_user::user_stream_ws_url(&listen_key)?,
            Duration::from_secs(30),
            WsHeartbeat::PingFrame,
            WsInboundCodec::Plain,
            WsServerPing::None,
        )));
        let run = Arc::clone(&manager);
        let run_guard = AbortOnDrop::new(tokio::spawn(async move {
            if let Err(error) = run.run().await {
                warn!(%error, "binance user ws manager stopped");
            }
        }));
        let mut rx = manager.subscribe();
        let mut keepalive = tokio::time::interval(Duration::from_secs(
            binance_ws_user::USER_STREAM_KEEPALIVE_INTERVAL_SECS,
        ));
        keepalive.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let started_at = common::time::now_ms();
        loop {
            tokio::select! {
                _ = keepalive.tick() => {
                    if session.lock(&state).await.is_none() { return Ok(()); }
                    adapter.keepalive_user_data_stream().await?;
                    if should_rotate_binance_stream(started_at) {
                        break;
                    }
                }
                Ok(event) = rx.recv() => {
                    let Some(account) = session.lock(&state).await else { return Ok(()); };
                    match event {
                        WsEvent::Connected => {
                            state.private_ws_health().record_connected("binance");
                        }
                        WsEvent::Text(text) => handle_binance_text(&state, &text, &session, account).await,
                        WsEvent::Disconnected(reason) => {
                            state.private_ws_health().record_disconnected("binance", &reason);
                        }
                        WsEvent::CircuitOpened => {
                            state.private_ws_health().record_circuit_opened("binance");
                        }
                        WsEvent::Binary(_) => {}
                    }
                }
            }
        }
        drop(run_guard);
        let _ = adapter.close_user_data_stream().await;
    }
}

pub(super) fn should_rotate_binance_stream(started_at_ms: i64) -> bool {
    let max_ms = (binance_ws_user::USER_STREAM_CONNECTION_MAX_SECS as i64).saturating_mul(1_000);
    common::time::now_ms().saturating_sub(started_at_ms) >= max_ms
}

async fn handle_binance_text(
    state: &AppState,
    text: &str,
    session: &PrivateWsSession,
    account: tokio::sync::MutexGuard<'_, ()>,
) {
    state.private_ws_health().record_text_received("binance");
    match binance_ws_user::parse_user_event(text) {
        Ok(Some(event)) => {
            let events = crate::trading_service::private_ws_mapper::map_binance_event(event);
            if !binance_order_events_require_durable_ack(&events) {
                state.private_ws_health().record_events("binance", &events);
            }
            apply_events_in_session(state, "binance", events, session, account).await;
        }
        Ok(None) => {}
        Err(error) => {
            state
                .private_ws_health()
                .record_parse_error("binance", &error.to_string());
            warn!(%error, "binance private ws parse failed");
        }
    }
}

pub(super) fn binance_order_events_require_durable_ack(
    events: &[crate::trading_service::private_ws_events::PrivateWsEvent],
) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            crate::trading_service::private_ws_events::PrivateWsEvent::BinanceOrderTrade(_)
        )
    })
}
