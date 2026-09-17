//! `/ws`：WebSocket 推送端点（订阅 `WsHub` 频道）。
//!
//! 协议：
//! ```json
//! // 客户端 -> 服务端
//! {"type": "auth", "ticket": "..."}
//! {"type": "subscribe", "channels": ["arbitrage", "watchlist"], "requestId": "web-..."}
//! {"type": "subscribe", "channels": ["arbitrage"], "replay": true, "requestId": "web-..."}
//! {"type": "unsubscribe", "channels": ["arbitrage"]}
//! {"type": "ping"}
//!
//! // 服务端 -> 客户端
//! {"type": "ack", "subscribed": ["arbitrage"], "requestId": "..."}
//! {"type": "message", "channel": "arbitrage", "payload": ...}
//! {"type": "batch", "messages": [{"channel": "arbitrage", "payload": ...}]}
//! {"type": "pong"}
//! {"type": "error", "code": "WS_CHANNEL_REJECTED", "message": "...", "requestId": "..."}
//! ```

use super::ws_outbound::{OutboundBatch, BATCH_LIMIT, FLUSH_MS};
use crate::services::{ws_auth::WsTicketConsume, ws_replay};
use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures::{SinkExt, StreamExt};
use realtime::{WsHub, WsMessage};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio::time::{Duration, MissedTickBehavior};
use tracing::{debug, warn};

type Subscribers = HashMap<String, JoinHandle<()>>;
type ForwardTx = mpsc::Sender<ForwardEvent>;

const MAX_SUBSCRIPTIONS_PER_CONNECTION: usize = 32;
const WS_BAD_JSON: &str = "WS_BAD_JSON";
const WS_BROADCAST_LAGGED: &str = "WS_BROADCAST_LAGGED";
const WS_CHANNEL_REJECTED: &str = "WS_CHANNEL_REJECTED";
const WS_UNAUTHORIZED: &str = "WS_UNAUTHORIZED";
const HEADER_REQUEST_ID: &str = "x-request-id";
const ZLIB_JSON_ENCODING: &str = "zlib-json";
const ZLIB_MIN_FRAME_BYTES: usize = 8 * 1024;
const ZLIB_LEVEL: u8 = 3;

pub(crate) fn router() -> Router<AppState> {
    Router::new().route("/ws", get(ws_handler))
}

async fn ws_handler(
    State(state): State<AppState>,
    Query(query): Query<WsQuery>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    let hub = state.ws_hub().clone();
    let context = WsRequestContext::from_headers(&headers)
        .with_frame_encoding(WsFrameEncoding::from_query(query.encoding.as_deref()));
    ws.on_upgrade(move |socket| handle_socket(socket, state, hub, context))
}

#[derive(Debug, Default, Deserialize)]
struct WsQuery {
    #[serde(default)]
    encoding: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum WsFrameEncoding {
    #[default]
    TextJson,
    ZlibJson,
}

impl WsFrameEncoding {
    fn from_query(value: Option<&str>) -> Self {
        if value == Some(ZLIB_JSON_ENCODING) {
            Self::ZlibJson
        } else {
            Self::TextJson
        }
    }
}

#[derive(Debug, Clone)]
struct WsRequestContext {
    request_id: String,
    frame_encoding: WsFrameEncoding,
}

struct WsConnectionContext<'a> {
    state: &'a AppState,
    hub: &'a WsHub,
    forward_tx: &'a ForwardTx,
    request: &'a WsRequestContext,
}

impl WsRequestContext {
    fn from_headers(headers: &HeaderMap) -> Self {
        let raw_request_id = request_id_from_headers(headers).or_else(common::request_id::current);
        let request_id = common::request_id::normalize(raw_request_id.as_deref());
        Self {
            request_id,
            frame_encoding: WsFrameEncoding::TextJson,
        }
    }

    fn with_frame_encoding(mut self, frame_encoding: WsFrameEncoding) -> Self {
        self.frame_encoding = frame_encoding;
        self
    }

    fn request_id(&self) -> &str {
        &self.request_id
    }

    fn control_request_id(&self, request_id: Option<&str>) -> String {
        request_id
            .map(|request_id| common::request_id::normalize(Some(request_id)))
            .unwrap_or_else(|| self.request_id.clone())
    }
}

fn request_id_from_headers(headers: &HeaderMap) -> Option<String> {
    headers
        .get(HEADER_REQUEST_ID)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientMessage {
    Auth {
        ticket: String,
    },
    Subscribe {
        channels: Vec<String>,
        #[serde(default)]
        replay: bool,
        #[serde(default, rename = "requestId")]
        request_id: Option<String>,
    },
    Unsubscribe {
        channels: Vec<String>,
    },
    Ping,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerMessage<'a> {
    Ack {
        subscribed: Vec<String>,
        #[serde(rename = "requestId")]
        request_id: &'a str,
    },
    Pong,
    Error {
        message: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        channel: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none", rename = "retryAfterMs")]
        retry_after_ms: Option<u64>,
        #[serde(rename = "requestId")]
        request_id: &'a str,
    },
}

enum ForwardEvent {
    ChannelMessage { channel: String, message: WsMessage },
    ControlError(ForwardError),
}

struct ForwardError {
    channel: String,
    code: &'static str,
    message: String,
    retry_after_ms: Option<u64>,
}

impl ForwardEvent {
    fn channel_message(channel: &str, message: WsMessage) -> Self {
        Self::ChannelMessage {
            channel: channel.to_owned(),
            message,
        }
    }

    fn lagged(channel: &str, skipped: u64) -> Self {
        Self::ControlError(ForwardError {
            channel: channel.to_owned(),
            code: WS_BROADCAST_LAGGED,
            message: format!(
                "channel {channel} lagged; skipped {skipped} broadcast message(s); subscribe with replay=true recommended"
            ),
            retry_after_ms: Some(FLUSH_MS),
        })
    }
}

async fn handle_socket(socket: WebSocket, state: AppState, hub: WsHub, context: WsRequestContext) {
    let (mut sender, mut receiver) = socket.split();
    let mut subscribers: Subscribers = HashMap::new();
    let mut authenticated = !state.config().security.auth_required();
    let (forward_tx, mut forward_rx) = mpsc::channel::<ForwardEvent>(BATCH_LIMIT);
    let mut outbound = OutboundBatch::new(BATCH_LIMIT);
    let mut flush = tokio::time::interval(Duration::from_millis(FLUSH_MS));
    flush.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let connection = WsConnectionContext {
        state: &state,
        hub: &hub,
        forward_tx: &forward_tx,
        request: &context,
    };

    'socket_loop: loop {
        tokio::select! {
            msg = receiver.next() => {
                if !handle_client_next(
                    msg,
                    &connection,
                    &mut subscribers,
                    &mut authenticated,
                    &mut sender,
                ).await {
                    break 'socket_loop;
                }
            }
            Some(event) = forward_rx.recv() => {
                if !handle_forward_event(event, &context, &mut sender, &mut outbound).await {
                    break 'socket_loop;
                }
            }
            _ = flush.tick(), if !outbound.is_empty() => {
                if flush_outbound(&mut sender, &mut outbound, context.frame_encoding).await.is_err() {
                    break 'socket_loop;
                }
            }
        }
    }

    abort_subscribers(subscribers);
}

async fn handle_forward_event<S>(
    event: ForwardEvent,
    context: &WsRequestContext,
    sender: &mut S,
    outbound: &mut OutboundBatch,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    match event {
        ForwardEvent::ChannelMessage { channel, message } => {
            let flush_now = should_flush_outbound(&channel, false);
            let batch_full = outbound.push((channel, message));
            if !batch_full && !flush_now {
                return true;
            }
            flush_outbound(sender, outbound, context.frame_encoding)
                .await
                .is_ok()
        }
        ForwardEvent::ControlError(error) => {
            if flush_outbound(sender, outbound, context.frame_encoding)
                .await
                .is_err()
            {
                return false;
            }
            send_json(sender, &server_forward_error(&error, context))
                .await
                .is_ok()
        }
    }
}

async fn handle_client_next<S>(
    msg: Option<Result<Message, axum::Error>>,
    connection: &WsConnectionContext<'_>,
    subscribers: &mut Subscribers,
    authenticated: &mut bool,
    sender: &mut S,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    let Some(Ok(msg)) = msg else {
        debug!("ws client disconnected");
        return false;
    };
    handle_client_message(msg, connection, subscribers, authenticated, sender).await
}

async fn handle_client_message<S>(
    msg: Message,
    connection: &WsConnectionContext<'_>,
    subscribers: &mut Subscribers,
    authenticated: &mut bool,
    sender: &mut S,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    match msg {
        Message::Text(text) => {
            handle_client_text(&text, connection, subscribers, authenticated, sender).await
        }
        Message::Ping(payload) => sender.send(Message::Pong(payload)).await.is_ok(),
        Message::Close(_) => {
            debug!("ws close received");
            false
        }
        _ => true,
    }
}

async fn handle_client_text<S>(
    text: &str,
    connection: &WsConnectionContext<'_>,
    subscribers: &mut Subscribers,
    authenticated: &mut bool,
    sender: &mut S,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    match serde_json::from_str::<ClientMessage>(text) {
        Ok(ClientMessage::Auth { ticket }) => {
            handle_auth(ticket, connection, authenticated, sender).await
        }
        Ok(ClientMessage::Subscribe {
            channels,
            replay,
            request_id,
        }) => {
            handle_subscribe(
                SubscribeRequest {
                    channels,
                    replay,
                    request_id: request_id.as_deref(),
                },
                connection,
                subscribers,
                *authenticated,
                sender,
            )
            .await
        }
        Ok(ClientMessage::Unsubscribe { channels }) => {
            unsubscribe_channels(channels, subscribers);
            true
        }
        Ok(ClientMessage::Ping) => send_json(sender, &ServerMessage::Pong).await.is_ok(),
        Err(error) => send_bad_json(sender, error, connection.request).await,
    }
}

async fn handle_auth<S>(
    ticket: String,
    connection: &WsConnectionContext<'_>,
    authenticated: &mut bool,
    sender: &mut S,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    if *authenticated {
        return true;
    }
    match connection
        .state
        .ws_tickets()
        .consume(Some(&ticket), common::time::now_ms())
    {
        WsTicketConsume::Accepted => {
            *authenticated = true;
            true
        }
        outcome => {
            let message = ws_auth_error_message(outcome);
            let _ = send_json(
                sender,
                &server_error(WS_UNAUTHORIZED, message, None, connection.request),
            )
            .await;
            false
        }
    }
}

struct SubscribeRequest<'a> {
    channels: Vec<String>,
    replay: bool,
    request_id: Option<&'a str>,
}

async fn handle_subscribe<S>(
    request: SubscribeRequest<'_>,
    connection: &WsConnectionContext<'_>,
    subscribers: &mut Subscribers,
    authenticated: bool,
    sender: &mut S,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    let request_id = connection.request.control_request_id(request.request_id);
    if !authenticated {
        let _ = send_json(
            sender,
            &server_error_for_request(
                WS_UNAUTHORIZED,
                "websocket auth ticket required before subscribe",
                None,
                &request_id,
            ),
        )
        .await;
        return false;
    }
    let outcome = subscribe_channels(
        request.channels,
        connection.hub,
        connection.forward_tx,
        subscribers,
        connection.state.config().api_surface.watchlist_alerts,
    );
    let ack = ServerMessage::Ack {
        subscribed: outcome.subscribed.clone(),
        request_id: &request_id,
    };
    // Start the stock feed only after the authenticated hub subscription exists.
    if outcome.subscribed.iter().any(|channel| channel == realtime::channels::STOCKS) {
        connection.state.backpack_stocks().ensure_started(connection.hub.clone());
    }
    if send_json(sender, &ack).await.is_err() {
        return false;
    }
    let replay_channels = if request.replay {
        &outcome.subscribed
    } else {
        &outcome.newly_subscribed
    };
    if !replay_subscribed(
        replay_channels,
        connection.state,
        sender,
        connection.request.frame_encoding,
    )
    .await
    {
        return false;
    }
    if outcome.rejected.is_empty() {
        return true;
    }
    for rejection in &outcome.rejected {
        let message = rejection.message();
        if send_json(
            sender,
            &server_channel_error(
                WS_CHANNEL_REJECTED,
                &message,
                &rejection.channel,
                &request_id,
            ),
        )
        .await
        .is_err()
        {
            return false;
        }
    }
    true
}

fn ws_auth_error_message(outcome: WsTicketConsume) -> &'static str {
    match outcome {
        WsTicketConsume::Accepted => "websocket auth accepted",
        WsTicketConsume::Missing => "websocket auth ticket missing",
        WsTicketConsume::NotFound => "websocket auth ticket invalid or already used",
        WsTicketConsume::Expired => "websocket auth ticket expired",
    }
}

async fn replay_subscribed<S>(
    channels: &[String],
    state: &AppState,
    sender: &mut S,
    frame_encoding: WsFrameEncoding,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    for channel in channels {
        let Ok(payloads) = ws_replay::payloads_for_channel(channel, state).await else {
            return false;
        };
        if send_replay_payloads(sender, payloads, frame_encoding)
            .await
            .is_err()
        {
            return false;
        }
    }
    true
}

/// 单条 replay 保持普通 message 帧；多条（orders 最多 50 条、execution runs）
/// 合并为一个 batch 帧——前端 `ServerEnvelope::Batch` 原生支持，subscribe 期间
/// 连接消息循环的停顿从 N 次串行 send 缩短为 1 次。
async fn send_replay_payloads<S>(
    sender: &mut S,
    mut payloads: Vec<ws_replay::ReplayPayload>,
    frame_encoding: WsFrameEncoding,
) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    match payloads.len() {
        0 => Ok(()),
        1 => {
            let payload = payloads.remove(0);
            send_channel_payload(sender, payload.channel, payload.payload, frame_encoding).await
        }
        _ => {
            let messages = payloads
                .into_iter()
                .map(|payload| {
                    serde_json::json!({
                        "channel": payload.channel,
                        "payload": payload.payload,
                    })
                })
                .collect::<Vec<_>>();
            let frame = serde_json::json!({ "type": "batch", "messages": messages });
            send_data_frame(sender, frame.to_string(), frame_encoding).await
        }
    }
}

async fn send_channel_payload<S>(
    sender: &mut S,
    channel: &str,
    payload: serde_json::Value,
    frame_encoding: WsFrameEncoding,
) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let frame = serde_json::json!({
        "type": "message",
        "channel": channel,
        "payload": payload,
    });
    send_data_frame(sender, frame.to_string(), frame_encoding).await
}

struct SubscribeOutcome {
    subscribed: Vec<String>,
    newly_subscribed: Vec<String>,
    rejected: Vec<SubscribeRejection>,
}

struct SubscribeRejection {
    channel: String,
    reason: &'static str,
}

impl SubscribeRejection {
    fn message(&self) -> String {
        format!("rejected channel: {} ({})", self.channel, self.reason)
    }
}

enum ChannelDecision {
    AlreadySubscribed,
    Accept,
    Reject(&'static str),
}

fn classify_channel(
    channel: &str,
    already_subscribed: bool,
    active_count: usize,
) -> ChannelDecision {
    if already_subscribed {
        return ChannelDecision::AlreadySubscribed;
    }
    if realtime::channels::ws_channel_spec(channel).is_none() {
        return ChannelDecision::Reject("unknown channel");
    }
    if active_count >= MAX_SUBSCRIPTIONS_PER_CONNECTION {
        return ChannelDecision::Reject("subscription limit reached");
    }
    ChannelDecision::Accept
}

fn classify_channel_with_policy(
    channel: &str,
    already_subscribed: bool,
    active_count: usize,
    watchlist_alerts_enabled: bool,
) -> ChannelDecision {
    let feature = realtime::channels::ws_channel_spec(channel).map(|spec| spec.feature);
    if !watchlist_alerts_enabled
        && feature == Some(realtime::channels::WsChannelFeature::WatchlistAlerts)
    {
        return ChannelDecision::Reject("api_surface.watchlist_alerts disabled");
    }
    classify_channel(channel, already_subscribed, active_count)
}

fn should_flush_outbound(channel: &str, batch_full: bool) -> bool {
    batch_full || is_immediate_channel(channel)
}

fn is_immediate_channel(channel: &str) -> bool {
    realtime::channels::ws_channel_spec(channel)
        .is_some_and(|spec| spec.delivery == realtime::channels::WsChannelDelivery::Immediate)
}

fn subscribe_channels(
    requested: Vec<String>,
    hub: &WsHub,
    forward_tx: &ForwardTx,
    subscribers: &mut Subscribers,
    watchlist_alerts_enabled: bool,
) -> SubscribeOutcome {
    let mut subscribed = Vec::with_capacity(requested.len());
    let mut newly_subscribed = Vec::with_capacity(requested.len());
    let mut rejected = Vec::new();
    let mut seen = HashSet::with_capacity(requested.len());
    for channel in requested {
        if !seen.insert(channel.clone()) {
            continue;
        }
        match classify_channel_with_policy(
            &channel,
            subscribers.contains_key(&channel),
            subscribers.len(),
            watchlist_alerts_enabled,
        ) {
            ChannelDecision::AlreadySubscribed => subscribed.push(channel),
            ChannelDecision::Accept => {
                let handle = spawn_channel_forwarder(hub, forward_tx, &channel);
                subscribers.insert(channel.clone(), handle);
                newly_subscribed.push(channel.clone());
                subscribed.push(channel);
            }
            ChannelDecision::Reject(reason) => {
                warn!(channel = %channel, reason, "ws subscribe rejected");
                rejected.push(SubscribeRejection { channel, reason });
            }
        }
    }
    SubscribeOutcome {
        subscribed,
        newly_subscribed,
        rejected,
    }
}

fn spawn_channel_forwarder(hub: &WsHub, forward_tx: &ForwardTx, channel: &str) -> JoinHandle<()> {
    let mut rx = hub.subscribe(channel);
    let hub = hub.clone();
    let tx = forward_tx.clone();
    let channel = channel.to_owned();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(message) => {
                    if tx
                        .send(ForwardEvent::channel_message(&channel, message))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    hub.record_lag(&channel, skipped, common::time::now_ms());
                    if tx
                        .send(ForwardEvent::lagged(&channel, skipped))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

fn unsubscribe_channels(channels: Vec<String>, subscribers: &mut Subscribers) {
    for channel in channels {
        if let Some(handle) = subscribers.remove(&channel) {
            handle.abort();
        }
    }
}

async fn send_bad_json<S>(
    sender: &mut S,
    error: serde_json::Error,
    context: &WsRequestContext,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    let message = format!("bad json: {error}");
    send_json(sender, &server_error(WS_BAD_JSON, &message, None, context))
        .await
        .is_ok()
}

async fn flush_outbound<S>(
    sender: &mut S,
    outbound: &mut OutboundBatch,
    frame_encoding: WsFrameEncoding,
) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let Some(payload) = outbound.serialize_and_clear().map_err(|_| ())? else {
        return Ok(());
    };
    if send_data_frame(sender, payload, frame_encoding)
        .await
        .is_err()
    {
        warn!("ws send failed; closing");
        return Err(());
    }
    Ok(())
}

async fn send_data_frame<S>(
    sender: &mut S,
    payload: String,
    frame_encoding: WsFrameEncoding,
) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    sender
        .send(encode_data_frame(payload, frame_encoding))
        .await
        .map_err(|_| ())
}

fn encode_data_frame(payload: String, frame_encoding: WsFrameEncoding) -> Message {
    if frame_encoding == WsFrameEncoding::ZlibJson && payload.len() >= ZLIB_MIN_FRAME_BYTES {
        return Message::Binary(miniz_oxide::deflate::compress_to_vec_zlib(
            payload.as_bytes(),
            ZLIB_LEVEL,
        ));
    }
    Message::Text(payload)
}

fn abort_subscribers(subscribers: Subscribers) {
    for handle in subscribers.into_values() {
        handle.abort();
    }
}

async fn send_json<S>(sender: &mut S, msg: &ServerMessage<'_>) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let Ok(text) = serde_json::to_string(msg) else {
        return Err(());
    };
    sender.send(Message::Text(text)).await.map_err(|_| ())
}

fn server_error<'a>(
    code: &'a str,
    message: &'a str,
    retry_after_ms: Option<u64>,
    context: &'a WsRequestContext,
) -> ServerMessage<'a> {
    server_error_for_request(code, message, retry_after_ms, context.request_id())
}

fn server_error_for_request<'a>(
    code: &'a str,
    message: &'a str,
    retry_after_ms: Option<u64>,
    request_id: &'a str,
) -> ServerMessage<'a> {
    ServerMessage::Error {
        message,
        channel: None,
        code: Some(code),
        retry_after_ms,
        request_id,
    }
}

fn server_forward_error<'a>(
    error: &'a ForwardError,
    context: &'a WsRequestContext,
) -> ServerMessage<'a> {
    ServerMessage::Error {
        message: &error.message,
        channel: Some(&error.channel),
        code: Some(error.code),
        retry_after_ms: error.retry_after_ms,
        request_id: context.request_id(),
    }
}

fn server_channel_error<'a>(
    code: &'a str,
    message: &'a str,
    channel: &'a str,
    request_id: &'a str,
) -> ServerMessage<'a> {
    ServerMessage::Error {
        message,
        channel: Some(channel),
        code: Some(code),
        retry_after_ms: None,
        request_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use futures::channel::mpsc as futures_mpsc;

    #[test]
    fn known_channel_under_cap_is_accepted() {
        assert!(matches!(
            classify_channel("arbitrage", false, 0),
            ChannelDecision::Accept
        ));
        assert!(matches!(
            classify_channel(realtime::channels::EXECUTION, false, 0),
            ChannelDecision::Accept
        ));
        assert!(matches!(
            classify_channel(realtime::channels::FUNDING_RATES, false, 5),
            ChannelDecision::Accept
        ));
    }

    #[test]
    fn already_subscribed_channel_is_noop() {
        assert!(matches!(
            classify_channel("orders", true, 1),
            ChannelDecision::AlreadySubscribed
        ));
    }

    #[test]
    fn unknown_channel_is_rejected() {
        assert!(matches!(
            classify_channel("definitely-not-a-channel", false, 0),
            ChannelDecision::Reject(_)
        ));
        assert!(matches!(
            classify_channel("", false, 0),
            ChannelDecision::Reject(_)
        ));
        for channel in ["market:BTC", "ticker:BTC:binance", "funding:BTC"] {
            assert!(matches!(
                classify_channel(channel, false, 0),
                ChannelDecision::Reject("unknown channel")
            ));
        }
    }

    #[test]
    fn cap_blocks_new_subscriptions() {
        assert!(matches!(
            classify_channel("system", false, MAX_SUBSCRIPTIONS_PER_CONNECTION),
            ChannelDecision::Reject(_)
        ));
    }

    #[test]
    fn orders_execution_and_risk_alerts_are_immediate_channels() {
        assert!(is_immediate_channel(realtime::channels::ORDERS));
        assert!(is_immediate_channel(realtime::channels::EXECUTION));
        assert!(is_immediate_channel(realtime::channels::ALERTS));
        assert!(is_immediate_channel(realtime::channels::RISK_ALERTS));
        assert!(!is_immediate_channel("risk_alerts"));
        assert!(!is_immediate_channel(realtime::channels::ARBITRAGE));
    }

    #[test]
    fn watchlist_alert_channels_follow_feature_gate() {
        for channel in [realtime::channels::WATCHLIST, realtime::channels::ALERTS] {
            assert!(matches!(
                classify_channel_with_policy(channel, false, 0, false),
                ChannelDecision::Reject("api_surface.watchlist_alerts disabled")
            ));
            assert!(matches!(
                classify_channel_with_policy(channel, false, 0, true),
                ChannelDecision::Accept
            ));
        }
        assert!(matches!(
            classify_channel_with_policy(realtime::channels::ORDERS, false, 0, false),
            ChannelDecision::Accept
        ));
    }

    #[test]
    fn batch_channels_wait_for_limit_or_tick() {
        assert!(!should_flush_outbound(realtime::channels::ARBITRAGE, false));
        assert!(should_flush_outbound(realtime::channels::ARBITRAGE, true));
        assert!(should_flush_outbound(realtime::channels::ORDERS, false));
        assert!(should_flush_outbound(realtime::channels::EXECUTION, false));
        assert!(should_flush_outbound(
            realtime::channels::RISK_ALERTS,
            false
        ));
    }

    #[test]
    fn zlib_encoding_is_exact_and_large_frames_round_trip() -> Result<(), String> {
        assert_eq!(
            WsFrameEncoding::from_query(Some(ZLIB_JSON_ENCODING)),
            WsFrameEncoding::ZlibJson
        );
        assert_eq!(
            WsFrameEncoding::from_query(Some("gzip")),
            WsFrameEncoding::TextJson
        );

        let source = "x".repeat(ZLIB_MIN_FRAME_BYTES);
        let Message::Binary(encoded) = encode_data_frame(source.clone(), WsFrameEncoding::ZlibJson)
        else {
            return Err("large negotiated frame was not compressed".into());
        };
        let decoded = miniz_oxide::inflate::decompress_to_vec_zlib(&encoded)
            .map_err(|error| format!("zlib decode failed: {error:?}"))?;
        assert_eq!(decoded, source.as_bytes());

        assert!(matches!(
            encode_data_frame("small".into(), WsFrameEncoding::ZlibJson),
            Message::Text(text) if text == "small"
        ));
        Ok(())
    }

    #[test]
    fn server_message_envelope_is_stable() -> Result<(), String> {
        let context = test_context("rid-envelope");
        let ack = server_json(ServerMessage::Ack {
            subscribed: vec![realtime::channels::ORDERS.to_owned()],
            request_id: context.request_id(),
        })?;
        assert_eq!(
            ack,
            serde_json::json!({
                "type": "ack",
                "subscribed": [realtime::channels::ORDERS],
                "requestId": "rid-envelope",
            })
        );
        let error = server_json(server_error(
            WS_CHANNEL_REJECTED,
            "rejected channel(s): unknown (unknown channel)",
            None,
            &context,
        ))?;
        assert_eq!(
            error,
            serde_json::json!({
                "type": "error",
                "code": "WS_CHANNEL_REJECTED",
                "message": "rejected channel(s): unknown (unknown channel)",
                "requestId": "rid-envelope",
            })
        );
        let pong = server_json(ServerMessage::Pong)?;
        assert_eq!(
            pong,
            serde_json::json!({
                "type": "pong",
            })
        );
        Ok(())
    }

    #[test]
    fn forward_error_envelope_scopes_lagged_channel() -> Result<(), String> {
        let context = test_context("rid-lag");
        let event = ForwardEvent::lagged(realtime::channels::ORDERS, 3);
        let ForwardEvent::ControlError(error) = event else {
            return Err("lagged event did not produce control error".into());
        };

        let value = server_json(server_forward_error(&error, &context))?;

        assert_eq!(value["type"], "error");
        assert_eq!(value["code"], WS_BROADCAST_LAGGED);
        assert_eq!(value["channel"], realtime::channels::ORDERS);
        assert_eq!(value["retryAfterMs"], FLUSH_MS);
        assert_eq!(value["requestId"], "rid-lag");
        assert!(value["message"]
            .as_str()
            .is_some_and(|msg| msg.contains("skipped 3") && msg.contains("replay")));
        Ok(())
    }

    #[test]
    fn request_context_prefers_request_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER_REQUEST_ID, HeaderValue::from_static("rid-header"));

        let context = WsRequestContext::from_headers(&headers);

        assert_eq!(context.request_id(), "rid-header");
    }

    #[test]
    fn request_context_replaces_unsafe_request_id_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            HEADER_REQUEST_ID,
            HeaderValue::from_static("unsafe/id value"),
        );

        let context = WsRequestContext::from_headers(&headers);

        assert_eq!(context.request_id().len(), 32);
        assert_ne!(context.request_id(), "unsafe/id value");
    }

    #[test]
    fn subscribe_command_parses_and_normalizes_request_id() -> Result<(), String> {
        let message = serde_json::from_str::<ClientMessage>(
            r#"{"type":"subscribe","channels":["orders"],"replay":true,"requestId":"web-command-7"}"#,
        )
        .map_err(|error| error.to_string())?;
        let ClientMessage::Subscribe {
            channels,
            replay,
            request_id,
        } = message
        else {
            return Err("subscribe fixture parsed as the wrong command".into());
        };
        let context = test_context("rid-connection");

        assert_eq!(channels, vec!["orders"]);
        assert!(replay);
        assert_eq!(request_id.as_deref(), Some("web-command-7"));
        assert_eq!(
            context.control_request_id(request_id.as_deref()),
            "web-command-7"
        );

        let normalized = context.control_request_id(Some("unsafe/id value"));
        assert_eq!(normalized.len(), 32);
        assert_ne!(normalized, "unsafe/id value");
        assert_eq!(context.control_request_id(None), "rid-connection");

        let default_message = serde_json::from_str::<ClientMessage>(
            r#"{"type":"subscribe","channels":["orders"],"requestId":"web-command-8"}"#,
        )
        .map_err(|error| error.to_string())?;
        let ClientMessage::Subscribe { replay, .. } = default_message else {
            return Err("default subscribe fixture parsed as the wrong command".into());
        };
        assert!(!replay);
        Ok(())
    }

    #[tokio::test]
    async fn request_context_uses_trace_scope_when_header_is_absent() {
        let headers = HeaderMap::new();

        let request_id = common::request_id::scope("rid-scope".to_owned(), async {
            WsRequestContext::from_headers(&headers)
                .request_id()
                .to_owned()
        })
        .await;

        assert_eq!(request_id, "rid-scope");
    }

    #[tokio::test]
    async fn mixed_subscribe_sends_ack_before_rejection_error() -> Result<(), String> {
        let state = test_state().await?;
        let hub = state.ws_hub().clone();
        let (forward_tx, _forward_rx) = mpsc::channel::<ForwardEvent>(BATCH_LIMIT);
        let mut subscribers = Subscribers::new();
        let (mut sink, mut rx) = futures_mpsc::unbounded::<Message>();
        let request = test_context("rid-subscribe");
        let connection = WsConnectionContext {
            state: &state,
            hub: &hub,
            forward_tx: &forward_tx,
            request: &request,
        };

        let ok = handle_subscribe(
            SubscribeRequest {
                channels: vec![
                    realtime::channels::ORDERS.to_owned(),
                    "unknown-channel".to_owned(),
                ],
                replay: false,
                request_id: Some("web-subscribe-command"),
            },
            &connection,
            &mut subscribers,
            true,
            &mut sink,
        )
        .await;

        let frames = collect_text_frames(&mut rx, 2).await;
        abort_subscribers(subscribers);

        assert!(ok);
        assert_eq!(frames.len(), 2);
        let ack = parse_json(&frames[0])?;
        assert_eq!(
            ack,
            serde_json::json!({
                "type": "ack",
                "subscribed": [realtime::channels::ORDERS],
                "requestId": "web-subscribe-command",
            })
        );
        let error = parse_json(&frames[1])?;
        assert_eq!(
            error,
            serde_json::json!({
                "type": "error",
                "code": "WS_CHANNEL_REJECTED",
                "channel": "unknown-channel",
                "message": "rejected channel: unknown-channel (unknown channel)",
                "requestId": "web-subscribe-command",
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn execution_subscribe_replays_recent_run_after_ack() -> Result<(), String> {
        let state = test_state().await?;
        crate::services::execution_runs::record(&state, replay_run("run-replay", 9_000_000));
        let hub = state.ws_hub().clone();
        let (forward_tx, _forward_rx) = mpsc::channel::<ForwardEvent>(BATCH_LIMIT);
        let mut subscribers = Subscribers::new();
        let (mut sink, mut rx) = futures_mpsc::unbounded::<Message>();
        let request = test_context("rid-replay");
        let connection = WsConnectionContext {
            state: &state,
            hub: &hub,
            forward_tx: &forward_tx,
            request: &request,
        };

        let ok = handle_subscribe(
            SubscribeRequest {
                channels: vec![realtime::channels::EXECUTION.to_owned()],
                replay: false,
                request_id: None,
            },
            &connection,
            &mut subscribers,
            true,
            &mut sink,
        )
        .await;

        let frames = collect_text_frames(&mut rx, 2).await;
        abort_subscribers(subscribers);

        assert!(ok);
        assert_eq!(frames.len(), 2);
        let ack = parse_json(&frames[0])?;
        assert_eq!(ack["subscribed"][0], realtime::channels::EXECUTION);
        assert_eq!(ack["requestId"], "rid-replay");
        let replay = parse_json(&frames[1])?;
        assert_eq!(replay["type"], "message");
        assert_eq!(replay["channel"], realtime::channels::EXECUTION);
        assert_eq!(replay["payload"]["event"], "execution_run_updated");
        assert_eq!(replay["payload"]["executionRun"]["runId"], "run-replay");
        Ok(())
    }

    #[tokio::test]
    async fn repeated_subscribe_requires_explicit_replay() -> Result<(), String> {
        let state = test_state().await?;
        crate::services::execution_runs::record(&state, replay_run("run-idempotent", 9_000_001));
        let hub = state.ws_hub().clone();
        let (forward_tx, _forward_rx) = mpsc::channel::<ForwardEvent>(BATCH_LIMIT);
        let mut subscribers = Subscribers::new();
        let (mut sink, mut rx) = futures_mpsc::unbounded::<Message>();
        let request = test_context("rid-idempotent");
        let connection = WsConnectionContext {
            state: &state,
            hub: &hub,
            forward_tx: &forward_tx,
            request: &request,
        };

        assert!(
            handle_subscribe(
                SubscribeRequest {
                    channels: vec![
                        realtime::channels::EXECUTION.to_owned(),
                        realtime::channels::EXECUTION.to_owned(),
                    ],
                    replay: false,
                    request_id: Some("initial"),
                },
                &connection,
                &mut subscribers,
                true,
                &mut sink,
            )
            .await
        );
        let initial_frames = collect_text_frames(&mut rx, 3).await;
        assert_eq!(initial_frames.len(), 2);
        assert_eq!(
            parse_json(&initial_frames[0])?["subscribed"],
            serde_json::json!([realtime::channels::EXECUTION])
        );

        assert!(
            handle_subscribe(
                SubscribeRequest {
                    channels: vec![realtime::channels::EXECUTION.to_owned()],
                    replay: false,
                    request_id: Some("idempotent"),
                },
                &connection,
                &mut subscribers,
                true,
                &mut sink,
            )
            .await
        );
        let idempotent_frames = collect_text_frames(&mut rx, 2).await;
        assert_eq!(idempotent_frames.len(), 1);
        assert_eq!(parse_json(&idempotent_frames[0])?["type"], "ack");

        assert!(
            handle_subscribe(
                SubscribeRequest {
                    channels: vec![realtime::channels::EXECUTION.to_owned()],
                    replay: true,
                    request_id: Some("explicit-replay"),
                },
                &connection,
                &mut subscribers,
                true,
                &mut sink,
            )
            .await
        );
        let replay_frames = collect_text_frames(&mut rx, 3).await;
        abort_subscribers(subscribers);

        assert_eq!(replay_frames.len(), 2);
        assert_eq!(parse_json(&replay_frames[0])?["type"], "ack");
        let replay = parse_json(&replay_frames[1])?;
        assert_eq!(replay["channel"], realtime::channels::EXECUTION);
        assert_eq!(replay["payload"]["executionRun"]["runId"], "run-idempotent");
        Ok(())
    }

    #[tokio::test]
    async fn subscribe_requires_auth_when_security_enabled() -> Result<(), String> {
        let state = authed_test_state().await?;
        let hub = state.ws_hub().clone();
        let (forward_tx, _forward_rx) = mpsc::channel::<ForwardEvent>(BATCH_LIMIT);
        let mut subscribers = Subscribers::new();
        let (mut sink, mut rx) = futures_mpsc::unbounded::<Message>();
        let request = test_context("rid-ws-auth-required");
        let connection = WsConnectionContext {
            state: &state,
            hub: &hub,
            forward_tx: &forward_tx,
            request: &request,
        };

        let ok = handle_subscribe(
            SubscribeRequest {
                channels: vec![realtime::channels::ORDERS.to_owned()],
                replay: false,
                request_id: Some("web-auth-required"),
            },
            &connection,
            &mut subscribers,
            false,
            &mut sink,
        )
        .await;

        let frames = collect_text_frames(&mut rx, 1).await;

        assert!(!ok);
        assert!(subscribers.is_empty());
        let error = parse_json(&frames[0])?;
        assert_eq!(error["code"], WS_UNAUTHORIZED);
        assert_eq!(error["requestId"], "web-auth-required");
        Ok(())
    }

    #[tokio::test]
    async fn auth_ticket_allows_subscribe_once() -> Result<(), String> {
        let state = authed_test_state().await?;
        let hub = state.ws_hub().clone();
        let (forward_tx, _forward_rx) = mpsc::channel::<ForwardEvent>(BATCH_LIMIT);
        let mut subscribers = Subscribers::new();
        let (mut sink, mut rx) = futures_mpsc::unbounded::<Message>();
        let request = test_context("rid-ws-auth-ok");
        let connection = WsConnectionContext {
            state: &state,
            hub: &hub,
            forward_tx: &forward_tx,
            request: &request,
        };
        let ticket = state.ws_tickets().issue(common::time::now_ms()).ticket;
        let mut authenticated = false;

        assert!(handle_auth(ticket.clone(), &connection, &mut authenticated, &mut sink).await);
        assert!(authenticated);
        let ok = handle_subscribe(
            SubscribeRequest {
                channels: vec![realtime::channels::ORDERS.to_owned()],
                replay: false,
                request_id: None,
            },
            &connection,
            &mut subscribers,
            authenticated,
            &mut sink,
        )
        .await;

        let frames = collect_text_frames(&mut rx, 1).await;
        abort_subscribers(subscribers);

        assert!(ok);
        assert_eq!(
            state
                .ws_tickets()
                .consume(Some(&ticket), common::time::now_ms()),
            WsTicketConsume::NotFound
        );
        let ack = parse_json(&frames[0])?;
        assert_eq!(ack["subscribed"][0], realtime::channels::ORDERS);
        assert_eq!(ack["requestId"], "rid-ws-auth-ok");
        Ok(())
    }

    #[tokio::test]
    async fn bad_json_error_is_typed() -> Result<(), String> {
        let (mut sink, mut rx) = futures_mpsc::unbounded::<Message>();
        let Err(json_error) = serde_json::from_str::<ClientMessage>("{not-json") else {
            return Err("invalid json fixture parsed successfully".into());
        };

        let ok = send_bad_json(&mut sink, json_error, &test_context("rid-bad-json")).await;

        let frames = collect_text_frames(&mut rx, 1).await;

        assert!(ok);
        assert_eq!(frames.len(), 1);
        let error = parse_json(&frames[0])?;
        assert_eq!(error["type"], "error");
        assert_eq!(error["code"], WS_BAD_JSON);
        assert_eq!(error["requestId"], "rid-bad-json");
        assert!(error["message"]
            .as_str()
            .is_some_and(|msg| msg.contains("bad json")));
        Ok(())
    }

    #[tokio::test]
    async fn lagged_broadcast_sends_typed_error_and_keeps_forwarder_alive() -> Result<(), String> {
        let hub = WsHub::new(1);
        let (tx, mut rx) = mpsc::channel::<ForwardEvent>(BATCH_LIMIT);
        let handle = spawn_channel_forwarder(&hub, &tx, realtime::channels::ORDERS);

        hub.publish(realtime::channels::ORDERS, WsMessage::Text("old".into()));
        hub.publish(realtime::channels::ORDERS, WsMessage::Text("new".into()));
        hub.publish(realtime::channels::ORDERS, WsMessage::Text("latest".into()));

        let first = recv_forward_event(&mut rx).await?;
        let ForwardEvent::ControlError(error) = first else {
            handle.abort();
            return Err("expected lag control error first".into());
        };
        assert_eq!(error.code, WS_BROADCAST_LAGGED);
        assert_eq!(error.channel, realtime::channels::ORDERS);
        let runtime = hub.runtime_snapshots();
        let lag = runtime
            .iter()
            .find(|row| row.channel == realtime::channels::ORDERS)
            .ok_or_else(|| "missing orders lag runtime snapshot".to_owned())?;
        assert_eq!(lag.lag_events, 1);
        assert_eq!(lag.skipped_messages, 2);
        assert!(lag.last_lag_at_ms.is_some());

        let second = recv_forward_event(&mut rx).await?;
        handle.abort();
        let ForwardEvent::ChannelMessage { channel, message } = second else {
            return Err("expected channel message after lag".into());
        };
        assert_eq!(channel, realtime::channels::ORDERS);
        assert!(matches!(message, WsMessage::Text(text) if text == "latest"));
        Ok(())
    }

    async fn collect_text_frames(
        rx: &mut futures_mpsc::UnboundedReceiver<Message>,
        limit: usize,
    ) -> Vec<String> {
        let mut frames = Vec::with_capacity(limit);
        for _ in 0..limit {
            let next = tokio::time::timeout(Duration::from_millis(50), rx.next()).await;
            let Ok(Some(Message::Text(text))) = next else {
                break;
            };
            frames.push(text.to_string());
        }
        frames
    }

    async fn recv_forward_event(
        rx: &mut mpsc::Receiver<ForwardEvent>,
    ) -> Result<ForwardEvent, String> {
        tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .map_err(|_| "timed out waiting for forward event".to_owned())?
            .ok_or_else(|| "forward channel closed".to_owned())
    }

    fn server_json(message: ServerMessage<'_>) -> Result<serde_json::Value, String> {
        serde_json::to_value(message)
            .map_err(|error| format!("server message serialization failed: {error}"))
    }

    fn parse_json(text: &str) -> Result<serde_json::Value, String> {
        serde_json::from_str(text)
            .map_err(|error| format!("server frame json parse failed: {error}; frame={text}"))
    }

    fn test_context(request_id: &str) -> WsRequestContext {
        WsRequestContext {
            request_id: request_id.to_owned(),
            frame_encoding: WsFrameEncoding::TextJson,
        }
    }

    async fn test_state() -> Result<AppState, String> {
        AppState::new(common::config::AppConfig::default())
            .await
            .map_err(|error| format!("test state failed: {error}"))
    }

    async fn authed_test_state() -> Result<AppState, String> {
        let mut config = common::config::AppConfig::default();
        config.history.enabled = false;
        config.security.auth_token = Some("secret".to_owned());
        AppState::new(config)
            .await
            .map_err(|error| format!("test state failed: {error}"))
    }

    fn replay_run(id: &str, updated_at_ms: i64) -> shared_types::ExecutionRun {
        shared_types::ExecutionRun {
            run_id: id.to_owned(),
            ticket_id: format!("ticket-{id}"),
            opportunity_id: format!("opp-{id}"),
            state: shared_types::ExecutionRunState::SecondLegSubmitted,
            long_leg: replay_leg(shared_types::HedgeLegRole::Long),
            short_leg: replay_leg(shared_types::HedgeLegRole::Short),
            net_exposure_usd: 0.0,
            cost_reconciliation: None,
            valuation_problem: None,
            unwind_problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            evidence: Default::default(),
            recovery_action: None,
            status_reason: "replay".to_owned(),
            created_at_ms: updated_at_ms,
            updated_at_ms,
        }
    }

    fn replay_leg(role: shared_types::HedgeLegRole) -> shared_types::ExecutionRunLeg {
        shared_types::ExecutionRunLeg {
            role,
            exchange: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            order_ids: Vec::new(),
            identity: None,
            finality_source: None,
            confirmed_filled_at_ms: None,
            state: shared_types::LiveOrderState::Accepted,
            target_quantity: 1.0,
            filled_quantity: None,
            target_notional_usd: 1.0,
            filled_notional_usd: None,
            filled_fee: None,
        }
    }
}
