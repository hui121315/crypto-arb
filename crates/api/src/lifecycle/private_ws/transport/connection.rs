use super::super::apply::apply_events;
use super::super::*;
use super::protocol::{AbortOnDrop, PrivateWsControl, PrivateWsParse};
use super::subscriptions::{
    send_private_ws_subscriptions_with_confirmation, SubscriptionConfirmation,
};

mod cache_ownership;
use cache_ownership::observe_owned_private_ws_caches;

type PrivateWsParser = dyn Fn(&str) -> PrivateWsParse + Send + Sync;
type PrivateWsMessageFactory = dyn Fn() -> ExchangeResult<Vec<String>> + Send + Sync;
const PRIVATE_CACHE_TOUCH_INTERVAL: Duration = Duration::from_secs(5);

pub(in super::super) fn spawn_plain_private_ws(
    state: AppState,
    venue: &'static str,
    config: WsConfig,
    messages_on_connect: impl Fn() -> ExchangeResult<Vec<String>> + Send + Sync + 'static,
    parse: impl Fn(&str) -> PrivateWsParse + Send + Sync + 'static,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        run_plain_private_ws(state, venue, config, messages_on_connect, parse).await;
    })
}

pub(in super::super) async fn run_plain_private_ws(
    state: AppState,
    venue: &'static str,
    config: WsConfig,
    messages_on_connect: impl Fn() -> ExchangeResult<Vec<String>> + Send + Sync + 'static,
    parse: impl Fn(&str) -> PrivateWsParse + Send + Sync + 'static,
) {
    run_private_ws(
        state,
        venue,
        config,
        messages_on_connect,
        SubscriptionConfirmation::Send,
        parse,
    )
    .await;
}

pub(in super::super) async fn run_confirmed_private_ws(
    state: AppState,
    venue: &'static str,
    config: WsConfig,
    messages_on_connect: impl Fn() -> ExchangeResult<Vec<String>> + Send + Sync + 'static,
    parse: impl Fn(&str) -> PrivateWsParse + Send + Sync + 'static,
) {
    run_private_ws(
        state,
        venue,
        config,
        messages_on_connect,
        SubscriptionConfirmation::ServerAck,
        parse,
    )
    .await;
}

async fn run_private_ws(
    state: AppState,
    venue: &'static str,
    config: WsConfig,
    messages_on_connect: impl Fn() -> ExchangeResult<Vec<String>> + Send + Sync + 'static,
    confirmation: SubscriptionConfirmation,
    parse: impl Fn(&str) -> PrivateWsParse + Send + Sync + 'static,
) {
    state.private_ws_health().record_task_started(venue);
    let (manager, _run_guard) = start_ws_manager(venue, config);
    let parse: Arc<PrivateWsParser> = Arc::new(parse);
    let messages_on_connect: Arc<PrivateWsMessageFactory> = Arc::new(messages_on_connect);
    let mut rx = manager.subscribe();
    let mut private_cache_touch = tokio::time::interval(PRIVATE_CACHE_TOUCH_INTERVAL);
    private_cache_touch.set_missed_tick_behavior(MissedTickBehavior::Skip);
    private_cache_touch.tick().await;
    loop {
        tokio::select! {
            received = rx.recv() => {
                let Ok(event) = received else {
                    break;
                };
                if !process_plain_ws_event(
                    &state,
                    venue,
                    PrivateWsConnection {
                        manager: &manager,
                        messages_on_connect: &messages_on_connect,
                        confirmation,
                    },
                    Arc::clone(&parse),
                    event,
                )
                .await
                {
                    state
                        .trading_service()
                        .invalidate_private_ws_session_cache(venue);
                    return;
                }
            }
            _ = private_cache_touch.tick() => {
                if manager.is_connected().await {
                    observe_owned_private_ws_caches(&state, venue);
                }
            }
        }
    }
    state
        .trading_service()
        .invalidate_private_ws_session_cache(venue);
    state
        .private_ws_health()
        .record_disconnected(venue, "receiver closed");
}

struct PrivateWsConnection<'a> {
    manager: &'a Arc<WsManager>,
    messages_on_connect: &'a Arc<PrivateWsMessageFactory>,
    confirmation: SubscriptionConfirmation,
}

async fn process_plain_ws_event(
    state: &AppState,
    venue: &'static str,
    connection: PrivateWsConnection<'_>,
    parse: Arc<PrivateWsParser>,
    event: WsEvent,
) -> bool {
    if observe_disconnection(state, venue, &event)
        || observe_circuit_or_binary(state, venue, &event)
    {
        return true;
    }
    handle_private_ws_event(state, venue, connection, parse, event).await
}

fn observe_disconnection(state: &AppState, venue: &'static str, event: &WsEvent) -> bool {
    if let WsEvent::Disconnected(reason) = event {
        state
            .trading_service()
            .invalidate_private_ws_session_cache(venue);
        state.private_ws_health().record_disconnected(venue, reason);
        debug!(%venue, %reason, "private ws disconnected");
        true
    } else {
        false
    }
}

fn observe_circuit_or_binary(state: &AppState, venue: &'static str, event: &WsEvent) -> bool {
    match event {
        WsEvent::CircuitOpened => {
            state
                .trading_service()
                .invalidate_private_ws_session_cache(venue);
            state.private_ws_health().record_circuit_opened(venue);
            warn!(%venue, "private ws circuit opened");
            true
        }
        WsEvent::Binary(_) => true,
        _ => false,
    }
}

fn start_ws_manager(venue: &'static str, config: WsConfig) -> (Arc<WsManager>, AbortOnDrop) {
    let manager = Arc::new(WsManager::new(config));
    let run = Arc::clone(&manager);
    let run_guard = AbortOnDrop::new(tokio::spawn(async move {
        if let Err(error) = run.run().await {
            warn!(%error, %venue, "private ws manager stopped");
        }
    }));
    (manager, run_guard)
}

async fn handle_private_ws_event(
    state: &AppState,
    venue: &'static str,
    connection: PrivateWsConnection<'_>,
    parse: Arc<PrivateWsParser>,
    event: WsEvent,
) -> bool {
    match event {
        WsEvent::Connected => {
            state.private_ws_health().record_connected(venue);
            let messages = match build_connect_messages(connection.messages_on_connect) {
                Ok(messages) => messages,
                Err(error) => {
                    state.private_ws_health().record_subscribe_build_failed(
                        venue,
                        0,
                        0,
                        &error.to_string(),
                    );
                    warn!(%error, %venue, "private ws on-connect payload build failed");
                    return false;
                }
            };
            send_private_ws_subscriptions_with_confirmation(
                venue,
                state,
                connection.manager,
                &messages,
                connection.confirmation,
            )
            .await
        }
        WsEvent::Text(text) => apply_private_ws_text(state, venue, &parse, &text).await,
        _ => true,
    }
}

fn build_connect_messages(factory: &Arc<PrivateWsMessageFactory>) -> ExchangeResult<Vec<String>> {
    factory()
}

async fn apply_private_ws_text(
    state: &AppState,
    venue: &'static str,
    parse: &Arc<PrivateWsParser>,
    text: &str,
) -> bool {
    state.private_ws_health().record_text_received(venue);
    let parsed = parse(text);
    if let Some(control) = parsed.control {
        let keep_running = apply_private_ws_control(state, venue, control);
        if keep_running {
            observe_owned_private_ws_caches(state, venue);
        }
        return keep_running;
    }
    if let Some(error) = parsed.error {
        state.private_ws_health().record_parse_error(venue, &error);
        return true;
    }
    if let Some(events) = parsed.events {
        if !parsed.health_after_apply {
            state.private_ws_health().record_events(venue, &events);
        }
        apply_events(state, venue, events).await;
        observe_owned_private_ws_caches(state, venue);
        return true;
    }
    observe_owned_private_ws_caches(state, venue);
    true
}

fn apply_private_ws_control(
    state: &AppState,
    venue: &'static str,
    control: PrivateWsControl,
) -> bool {
    match control {
        PrivateWsControl::SubscribeAck {
            channel,
            expected,
            request_id,
        } => {
            state.private_ws_health().record_subscribe_ack(
                venue,
                expected,
                &channel,
                request_id.as_deref(),
            );
            true
        }
        PrivateWsControl::SubscribeRejected {
            channel,
            expected,
            authentication_failed,
            error,
            request_id,
        } => {
            state.private_ws_health().record_subscribe_rejected(
                venue,
                expected,
                &channel,
                &error,
                request_id.as_deref(),
            );
            if authentication_failed {
                state.private_ws_health().record_auth_failed(venue, &error);
            }
            false
        }
    }
}

#[cfg(test)]
#[path = "connection_tests.rs"]
mod tests;
