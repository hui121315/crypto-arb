use super::apply::apply_events;
use super::transport::{send_private_ws_subscriptions, ws_config, AbortOnDrop};
use super::*;

pub(super) fn spawn_kucoin_private_ws(
    state: AppState,
    credentials: Option<(String, String, String)>,
) -> Option<JoinHandle<()>> {
    let (api_key, api_secret, passphrase) = credentials?;
    Some(tokio::spawn(async move {
        let health_state = state.clone();
        if let Err(error) = run_kucoin_private_ws(state, api_key, api_secret, passphrase).await {
            health_state
                .private_ws_health()
                .record_disconnected("kucoin", &error.to_string());
            warn!(%error, "kucoin private ws stopped");
        }
    }))
}

async fn run_kucoin_private_ws(
    state: AppState,
    api_key: String,
    api_secret: String,
    passphrase: String,
) -> ExchangeResult<()> {
    state.private_ws_health().record_task_started("kucoin");
    let session = super::kucoin_session::fetch_private_session(
        KUCOIN_FUTURES_BASE,
        kucoin_ws_user::KUCOIN_FUTURES_PRIVATE_BULLET_PATH,
        "kucoin-private-ws",
        &api_key,
        &api_secret,
        &passphrase,
    )
    .await?;
    let manager = Arc::new(WsManager::new(ws_config(
        "kucoin",
        session.ws_url,
        Duration::from_millis(session.ping_interval_ms),
        WsHeartbeat::Text(r#"{"id":"ping","type":"ping"}"#.to_owned()),
        WsInboundCodec::Plain,
        WsServerPing::None,
    )));
    let run = Arc::clone(&manager);
    let _run_guard = AbortOnDrop::new(tokio::spawn(async move {
        if let Err(error) = run.run().await {
            warn!(%error, "kucoin user ws manager stopped");
        }
    }));
    let mut rx = manager.subscribe();
    while let Ok(event) = rx.recv().await {
        match event {
            WsEvent::Connected => {
                state.private_ws_health().record_connected("kucoin");
                let messages = kucoin_private_subscribe_messages()?;
                if !send_private_ws_subscriptions("kucoin", &state, &manager, &messages).await {
                    return Err(ExchangeError::WsClosed(
                        "kucoin private ws subscribe failed".into(),
                    ));
                }
            }
            WsEvent::Text(text) => handle_kucoin_text(&state, &text).await,
            WsEvent::CircuitOpened => {
                state.private_ws_health().record_circuit_opened("kucoin");
                return Err(ExchangeError::CircuitBreaker {
                    exchange: "kucoin".to_owned(),
                });
            }
            WsEvent::Disconnected(reason) => {
                state
                    .private_ws_health()
                    .record_disconnected("kucoin", &reason);
                debug!(%reason, "kucoin user ws disconnected");
            }
            WsEvent::Binary(_) => {}
        }
    }
    Err(ExchangeError::WsClosed("kucoin receiver closed".into()))
}

pub(super) fn kucoin_private_subscribe_messages() -> ExchangeResult<[String; 3]> {
    Ok([
        kucoin_ws_user::subscribe_orders_payload("orders")?,
        kucoin_ws_user::subscribe_balance_payload("balance")?,
        kucoin_ws_user::subscribe_positions_payload("positions")?,
    ])
}

async fn handle_kucoin_text(state: &AppState, text: &str) {
    state.private_ws_health().record_text_received("kucoin");
    match kucoin_ws_user::parse_user_event(text) {
        Ok(Some(event)) => {
            let events = crate::trading_service::private_ws_mapper::map_kucoin_event(event);
            state.private_ws_health().record_events("kucoin", &events);
            apply_events(state, "kucoin", events).await;
        }
        Ok(None) => {}
        Err(error) => {
            state
                .private_ws_health()
                .record_parse_error("kucoin", &error.to_string());
            warn!(%error, "kucoin private ws parse failed");
        }
    }
}
