//! Shared browser WebSocket runtime.
//!
//! The public `ws.rs` start functions remain compatibility wrappers; this module owns the
//! single physical connection and channel dispatch.

mod transport;

use crate::api::base::{normalize_api_auth_token, normalize_api_base, ws_url_from_api_base};
use crate::api::request_id::next_request_id;
use crate::api::rest::{ApiClient, ApiError};
use crate::api::ws::{
    ack_includes_channel, mark_channel_message, mark_channel_problem, now_ms, ws_problem,
    ws_reconnect_problem, ws_server_problem, ChannelPayload, ServerEnvelope, WsChannelState,
    WsStatus, WsStreamHandle, PING_INTERVAL_MS, RECONNECT_DELAY_MS,
};
use futures::future::{AbortHandle, Abortable};
use gloo_timers::future::TimeoutFuture;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::Value;
use shared_types::ApiProblem;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::rc::Rc;
use transport::{frame_sender, BrowserSocket, SocketCallbacks};
use web_sys::WebSocket;

thread_local! {
    static WS_RUNTIME: RefCell<Option<WsRuntime>> = const { RefCell::new(None) };
}

#[derive(Clone)]
pub(crate) struct WsRuntime {
    inner: Rc<WsRuntimeInner>,
}

struct WsRuntimeInner {
    api_base: RwSignal<String>,
    api_auth_token: RwSignal<String>,
    subscribers: RefCell<Vec<RuntimeSubscriber>>,
    active_abort: RefCell<Option<AbortHandle>>,
    active_socket: RefCell<Option<BrowserSocket>>,
    frame_sender: RefCell<Option<FrameSender>>,
    channel_states: RefCell<BTreeMap<String, WsChannelState>>,
    pending_subscribe_batches: RefCell<VecDeque<PendingSubscribe>>,
    next_id: Cell<u64>,
    generation: Cell<u64>,
    connection: RefCell<ConnectionIdentity>,
    #[cfg(test)]
    command_observer: RefCell<Option<Rc<dyn Fn(RuntimeCommand)>>>,
}

type FrameSender = Rc<dyn Fn(&str) -> Result<(), String>>;

#[derive(Clone, PartialEq, Eq)]
struct ConnectionIdentity {
    base: String,
    token: String,
}

pub(crate) struct RuntimeSubscription {
    pub(crate) channel: &'static str,
    pub(crate) channel_state: RwSignal<WsChannelState>,
    pub(crate) on_payload: Rc<dyn Fn(&Value) -> Result<(), ApiProblem>>,
    pub(crate) on_problem: Rc<dyn Fn(ApiProblem)>,
}

#[derive(Clone)]
struct RuntimeSubscriber {
    id: u64,
    channel: &'static str,
    channel_state: RwSignal<WsChannelState>,
    on_payload: Rc<dyn Fn(&Value) -> Result<(), ApiProblem>>,
    on_problem: Rc<dyn Fn(ApiProblem)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RuntimeCommand {
    Subscribe(Vec<String>),
    Unsubscribe(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingSubscribe {
    request_id: String,
    channels: Vec<String>,
}

pub(crate) fn provide_ws_runtime(api_base: RwSignal<String>, api_auth_token: RwSignal<String>) {
    WS_RUNTIME.with(|slot| {
        if let Some(existing) = slot.borrow_mut().take() {
            existing.shutdown();
        }
        let runtime = WsRuntime::new(api_base, api_auth_token);
        restart_runtime_on_api_context_change(runtime.clone());
        *slot.borrow_mut() = Some(runtime);
    });
}

pub(crate) fn current_ws_runtime() -> Option<WsRuntime> {
    WS_RUNTIME.with(|slot| slot.borrow().clone())
}

impl WsRuntime {
    fn new(api_base: RwSignal<String>, api_auth_token: RwSignal<String>) -> Self {
        Self {
            inner: Rc::new(WsRuntimeInner {
                api_base,
                api_auth_token,
                subscribers: RefCell::new(Vec::new()),
                active_abort: RefCell::new(None),
                active_socket: RefCell::new(None),
                frame_sender: RefCell::new(None),
                channel_states: RefCell::new(BTreeMap::new()),
                pending_subscribe_batches: RefCell::new(VecDeque::new()),
                next_id: Cell::new(1),
                generation: Cell::new(0),
                connection: RefCell::new(ConnectionIdentity {
                    base: normalize_api_base(&api_base.get_untracked()),
                    token: normalize_api_auth_token(&api_auth_token.get_untracked()),
                }),
                #[cfg(test)]
                command_observer: RefCell::new(None),
            }),
        }
    }

    pub(crate) fn subscribe(&self, subscription: RuntimeSubscription) -> WsStreamHandle {
        let id = self.next_id();
        let channel = subscription.channel;
        let channel_was_active = self.channel_is_active(channel);
        let previous_state = self.channel_runtime_state(channel);
        self.inner.subscribers.borrow_mut().push(RuntimeSubscriber {
            id,
            channel,
            channel_state: subscription.channel_state,
            on_payload: subscription.on_payload,
            on_problem: subscription.on_problem,
        });
        if channel_was_active {
            if let Some(state) = previous_state.or_else(|| self.subscriber_state(channel)) {
                self.set_state_for_subscriber(id, &state);
            }
            return WsStreamHandle::runtime(id);
        }
        self.subscribe_new_channel(channel);
        WsStreamHandle::runtime(id)
    }

    fn next_id(&self) -> u64 {
        let id = self.inner.next_id.get();
        self.inner.next_id.set(id.wrapping_add(1).max(1));
        id
    }

    pub(crate) fn unsubscribe(&self, id: u64) {
        let Some(channel) = self.remove_subscriber(id) else {
            return;
        };
        if self.channel_is_active(&channel) {
            return;
        }
        if self.active_channels().is_empty() {
            self.restart();
            return;
        }
        if !self.send_command(RuntimeCommand::Unsubscribe(vec![channel])) {
            self.restart();
        }
    }

    fn shutdown(&self) {
        self.bump_generation();
        self.abort_active();
        self.inner.subscribers.borrow_mut().clear();
    }

    fn restart(&self) {
        let token = self.bump_generation();
        self.abort_active();
        self.reset_pending_subscribe_batches();
        if let Some(connection) = self.current_connection() {
            if *self.inner.connection.borrow() != connection {
                *self.inner.connection.borrow_mut() = connection;
                self.inner.channel_states.borrow_mut().clear();
                for subscriber in self.active_subscribers() {
                    sync_subscriber_state(&subscriber, &WsChannelState::new(subscriber.channel));
                }
            }
        }
        let channels = self.active_channels();
        if channels.is_empty() {
            return;
        }
        self.set_status_for_channels(&channels, WsStatus::Connecting);
        self.start_connection(token);
    }

    pub(crate) fn request_channel_replay(&self, channel: &str) {
        if !self.channel_is_active(channel) {
            return;
        }
        if !self
            .channel_runtime_state(channel)
            .is_some_and(|state| state.subscribed)
        {
            self.restart();
            return;
        }
        let channels = vec![channel.to_owned()];
        let request_id = next_request_id();
        let frame = serde_json::json!({
            "type": "subscribe", "channels": channels,
            "replay": true, "requestId": request_id,
        })
        .to_string();
        if self.send_frame(&frame).is_ok() {
            self.queue_pending_subscribe(request_id, channels.clone());
            self.set_status_for_channels(&channels, WsStatus::Connecting);
        } else {
            self.restart();
        }
    }

    fn start_connection(&self, token: u64) {
        if !self.generation_matches(token) || self.active_channels().is_empty() {
            return;
        }
        let connection = self.inner.connection.borrow().clone();
        let url = ws_url_from_api_base(&connection.base);
        let client = ApiClient::with_base_and_auth(&connection.base, &connection.token);
        let runtime = self.clone();
        let (abort, registration) = AbortHandle::new_pair();
        *self.inner.active_abort.borrow_mut() = Some(abort);
        spawn_local(async move {
            let _abort_result =
                Abortable::new(runtime.run_loop(url, client, token), registration).await;
        });
    }

    fn bump_generation(&self) -> u64 {
        let token = self.inner.generation.get().wrapping_add(1);
        self.inner.generation.set(token);
        token
    }

    fn generation_matches(&self, token: u64) -> bool {
        self.inner.generation.get() == token
            && self.current_connection().as_ref() == Some(&*self.inner.connection.borrow())
    }

    fn current_connection(&self) -> Option<ConnectionIdentity> {
        Some(ConnectionIdentity {
            base: normalize_api_base(&self.inner.api_base.try_get_untracked()?),
            token: normalize_api_auth_token(&self.inner.api_auth_token.try_get_untracked()?),
        })
    }

    fn abort_active(&self) {
        if let Some(abort) = self.inner.active_abort.borrow_mut().take() {
            abort.abort();
        }
        self.inner.frame_sender.borrow_mut().take();
        if let Some(socket) = self.inner.active_socket.borrow_mut().take() {
            socket.close();
        }
    }

    fn subscribe_new_channel(&self, channel: &'static str) {
        self.set_status_for_channels(&[channel.to_owned()], WsStatus::Connecting);
        if !self.send_command(RuntimeCommand::Subscribe(vec![channel.to_owned()])) {
            self.restart();
        }
    }

    fn send_command(&self, command: RuntimeCommand) -> bool {
        #[cfg(test)]
        if let Some(observer) = self.inner.command_observer.borrow().as_ref() {
            observer(command);
            return true;
        }

        match self.send_connected_command(command) {
            Ok(()) => true,
            Err(error) => {
                if error != "websocket is not connected" {
                    self.broadcast_problem(&ws_command_write_problem(error));
                }
                false
            }
        }
    }

    fn remove_subscriber(&self, id: u64) -> Option<String> {
        let mut subscribers = self.inner.subscribers.borrow_mut();
        let index = subscribers
            .iter()
            .position(|subscriber| subscriber.id == id)?;
        Some(subscribers.remove(index).channel.to_owned())
    }

    fn active_channels(&self) -> Vec<String> {
        active_channels_from(&self.inner.subscribers.borrow())
    }

    fn channel_is_active(&self, channel: &str) -> bool {
        self.inner
            .subscribers
            .borrow()
            .iter()
            .any(|subscriber| subscriber.channel == channel)
    }

    fn subscriber_state(&self, channel: &str) -> Option<WsChannelState> {
        self.inner
            .subscribers
            .borrow()
            .iter()
            .find(|subscriber| subscriber.channel == channel)
            .map(|subscriber| subscriber.channel_state.get_untracked())
    }

    fn subscribers_for(&self, channel: &str) -> Vec<RuntimeSubscriber> {
        self.inner
            .subscribers
            .borrow()
            .iter()
            .filter(|subscriber| subscriber.channel == channel)
            .cloned()
            .collect()
    }

    fn active_subscribers(&self) -> Vec<RuntimeSubscriber> {
        self.inner.subscribers.borrow().clone()
    }

    async fn run_loop(self, url: String, client: ApiClient, token: u64) {
        while self.generation_matches(token) {
            let channels = self.active_channels();
            if channels.is_empty() {
                return;
            }
            self.reset_pending_subscribe_batches();
            self.set_status_for_channels(&channels, WsStatus::Connecting);
            let ticket = client.ws_ticket().await;
            if !self.generation_matches(token) {
                return;
            }
            let ticket = match ticket {
                Ok(response) => response.ticket,
                Err(error) => {
                    self.broadcast_problem(&ws_auth_ticket_problem(error));
                    self.set_status_for_channels(&channels, WsStatus::Disconnected);
                    TimeoutFuture::new(RECONNECT_DELAY_MS).await;
                    continue;
                }
            };
            match self.open_socket(&url, ticket, channels.clone(), token) {
                Ok(()) => return,
                Err(error) => {
                    self.broadcast_problem(&ws_reconnect_problem(
                        "runtime",
                        "WS_OPEN_FAILED",
                        &error,
                    ));
                    self.set_status_for_channels(&channels, WsStatus::Disconnected);
                    TimeoutFuture::new(RECONNECT_DELAY_MS).await;
                    continue;
                }
            }
        }
    }

    fn open_socket(
        &self,
        url: &str,
        ticket: String,
        channels: Vec<String>,
        token: u64,
    ) -> Result<(), String> {
        let open_runtime = self.clone();
        let text_runtime = self.clone();
        let error_runtime = self.clone();
        let close_runtime = self.clone();
        let callbacks = SocketCallbacks {
            on_open: Rc::new(move |socket| {
                open_runtime.socket_opened(&socket, &ticket, &channels, token);
            }),
            on_text: Rc::new(move |text| {
                if text_runtime.generation_matches(token) {
                    text_runtime.handle_text(&text);
                }
            }),
            on_error: Rc::new(move |error| {
                if error_runtime.generation_matches(token) {
                    error_runtime.broadcast_problem(&ws_reconnect_problem(
                        "runtime",
                        "WS_READ_ERROR",
                        error,
                    ));
                }
            }),
            on_close: Rc::new(move || {
                let runtime = close_runtime.clone();
                spawn_local(async move {
                    TimeoutFuture::new(0).await;
                    runtime.socket_closed(token);
                });
            }),
        };
        let socket = BrowserSocket::open(url, PING_INTERVAL_MS, callbacks)?;
        *self.inner.active_socket.borrow_mut() = Some(socket);
        Ok(())
    }

    fn socket_opened(&self, socket: &WebSocket, ticket: &str, channels: &[String], token: u64) {
        if !self.generation_matches(token) {
            let _ = socket.close();
            return;
        }
        *self.inner.frame_sender.borrow_mut() = Some(frame_sender(socket.clone()));
        if let Err(error) = self.send_frame(&auth_frame(ticket)) {
            self.broadcast_problem(&ws_reconnect_problem("runtime", "WS_AUTH_FAILED", error));
            let _ = socket.close();
            return;
        }
        let request_id = next_request_id();
        if let Err(error) = self.send_frame(&subscribe_frame(channels, &request_id)) {
            self.broadcast_problem(&ws_reconnect_problem(
                "runtime",
                "WS_SUBSCRIBE_FAILED",
                error,
            ));
            let _ = socket.close();
            return;
        }
        self.queue_pending_subscribe(request_id, channels.to_vec());
    }

    fn socket_closed(&self, token: u64) {
        if !self.generation_matches(token) {
            return;
        }
        // Each physical socket gets a distinct generation, even on the same API.
        let token = self.bump_generation();
        self.inner.frame_sender.borrow_mut().take();
        self.inner.active_socket.borrow_mut().take();
        self.reset_pending_subscribe_batches();
        let channels = self.active_channels();
        self.set_status_for_channels(&channels, WsStatus::Disconnected);
        if channels.is_empty() {
            return;
        }
        let runtime = self.clone();
        spawn_local(async move {
            TimeoutFuture::new(RECONNECT_DELAY_MS).await;
            if runtime.generation_matches(token) {
                runtime.start_connection(token);
            }
        });
    }

    fn handle_text(&self, text: &str) -> bool {
        match serde_json::from_str::<ServerEnvelope>(text) {
            Ok(ServerEnvelope::Ack {
                subscribed,
                request_id,
            }) => {
                self.apply_ack(&subscribed, request_id.as_deref());
                true
            }
            Ok(ServerEnvelope::Message { channel, payload }) => {
                self.deliver(&channel, &payload);
                true
            }
            Ok(ServerEnvelope::Batch { messages }) => {
                self.deliver_batch(messages);
                true
            }
            Ok(ServerEnvelope::Error {
                message,
                channel,
                code,
                retry_after_ms,
                request_id,
            }) => {
                let target_channel = channel.as_deref().unwrap_or("runtime");
                let problem = ws_server_problem(
                    target_channel,
                    code.as_deref().unwrap_or("WS_SERVER_ERROR"),
                    &message,
                    retry_after_ms,
                    request_id,
                );
                if let Some(channel) = channel {
                    self.problem_for_channel(&channel, &problem);
                } else {
                    self.broadcast_problem(&problem);
                }
                true
            }
            Ok(ServerEnvelope::Pong) => true,
            Err(error) => {
                self.broadcast_payload_problem(&ws_problem("runtime", "WS_DECODE", error));
                true
            }
        }
    }

    fn apply_ack(&self, subscribed: &[String], request_id: Option<&str>) {
        let pending = match self.take_pending_subscribe(request_id) {
            Ok(pending) => pending,
            Err(problem) => {
                self.broadcast_problem(&problem);
                return;
            }
        };
        for channel in pending.channels {
            if ack_includes_channel(subscribed, &channel) {
                self.set_subscribed(&channel);
            } else {
                let problem = ws_problem(
                    &channel,
                    "WS_SUBSCRIBE_ACK_MISSING",
                    "subscribe ack did not include requested channel",
                )
                .with_request_id(request_id.map(str::to_owned));
                self.problem_for_channel(&channel, &problem);
            }
        }
    }

    fn deliver_batch(&self, messages: Vec<ChannelPayload>) {
        for message in messages {
            self.deliver(&message.channel, &message.payload);
        }
    }

    fn deliver(&self, channel: &str, payload: &Value) {
        let subscribers = self.subscribers_for(channel);
        if subscribers.is_empty() {
            return;
        }
        if let Some(problem) = subscribers
            .iter()
            .find_map(|subscriber| (subscriber.on_payload)(payload).err())
        {
            self.problem_for_channel(channel, &problem);
            return;
        }
        let state = self.record_channel_message(channel);
        for subscriber in subscribers {
            sync_subscriber_state(&subscriber, &state);
        }
    }

    fn broadcast_problem(&self, problem: &ApiProblem) {
        for channel in self.active_channels() {
            self.record_channel_problem(&channel, problem, Some(WsStatus::Disconnected));
        }
        for subscriber in self.active_subscribers() {
            if let Some(state) = self.channel_runtime_state(subscriber.channel) {
                sync_subscriber_state(&subscriber, &state);
            }
            (subscriber.on_problem)(problem.clone());
        }
    }

    fn broadcast_payload_problem(&self, problem: &ApiProblem) {
        // Invalid data is not proof that the socket or subscription was lost.
        for channel in self.active_channels() {
            self.problem_for_channel(&channel, problem);
        }
    }

    fn problem_for_channel(&self, channel: &str, problem: &ApiProblem) {
        let state = self.record_channel_problem(channel, problem, None);
        for subscriber in self.subscribers_for(channel) {
            sync_subscriber_state(&subscriber, &state);
            (subscriber.on_problem)(problem.clone());
        }
    }

    fn set_status_for_channels(&self, channels: &[String], status: WsStatus) {
        for channel in channels {
            let state = self.record_channel_status(channel, status);
            for subscriber in self.subscribers_for(channel) {
                sync_subscriber_state(&subscriber, &state);
            }
        }
    }

    fn set_subscribed(&self, channel: &str) {
        let state = self.record_channel_subscribed(channel);
        for subscriber in self.subscribers_for(channel) {
            sync_subscriber_state(&subscriber, &state);
        }
    }

    fn set_state_for_subscriber(&self, id: u64, state: &WsChannelState) {
        for subscriber in self
            .inner
            .subscribers
            .borrow()
            .iter()
            .filter(|subscriber| subscriber.id == id)
        {
            sync_subscriber_state(subscriber, state);
        }
    }

    fn channel_runtime_state(&self, channel: &str) -> Option<WsChannelState> {
        self.inner.channel_states.borrow().get(channel).cloned()
    }

    fn record_channel_status(&self, channel: &str, status: WsStatus) -> WsChannelState {
        self.update_channel_state(channel, |state| {
            state.status = status;
            if status != WsStatus::Connected {
                state.subscribed = false;
                state.last_message_at_ms = None;
            }
        })
    }

    fn record_channel_subscribed(&self, channel: &str) -> WsChannelState {
        self.update_channel_state(channel, |state| {
            state.status = WsStatus::Connected;
            state.subscribed = true;
            if state
                .last_error
                .as_ref()
                .is_some_and(subscription_ack_resolves_problem)
            {
                state.last_error = None;
                state.retry_after_ms = None;
            }
        })
    }

    fn record_channel_message(&self, channel: &str) -> WsChannelState {
        self.update_channel_state(channel, |state| {
            mark_channel_message(state, now_ms());
        })
    }

    fn record_channel_problem(
        &self,
        channel: &str,
        problem: &ApiProblem,
        status: Option<WsStatus>,
    ) -> WsChannelState {
        self.update_channel_state(channel, |state| {
            if let Some(status) = status {
                state.status = status;
                if status != WsStatus::Connected {
                    state.subscribed = false;
                }
            }
            mark_channel_problem(state, problem, now_ms());
        })
    }

    fn update_channel_state(
        &self,
        channel: &str,
        apply: impl FnOnce(&mut WsChannelState),
    ) -> WsChannelState {
        let mut states = self.inner.channel_states.borrow_mut();
        let state = states
            .entry(channel.to_owned())
            .or_insert_with(|| WsChannelState::new(channel));
        apply(state);
        state.clone()
    }

    fn reset_pending_subscribe_batches(&self) {
        self.inner.pending_subscribe_batches.borrow_mut().clear();
    }

    fn queue_pending_subscribe(&self, request_id: String, channels: Vec<String>) {
        if !channels.is_empty() {
            self.inner
                .pending_subscribe_batches
                .borrow_mut()
                .push_back(PendingSubscribe {
                    request_id,
                    channels,
                });
        }
    }

    fn take_pending_subscribe(
        &self,
        request_id: Option<&str>,
    ) -> Result<PendingSubscribe, ApiProblem> {
        let mut pending = self.inner.pending_subscribe_batches.borrow_mut();
        if let Some(request_id) = request_id {
            let Some(index) = pending
                .iter()
                .position(|command| command.request_id == request_id)
            else {
                return Err(ws_problem(
                    "runtime",
                    "WS_ACK_REQUEST_ID_UNKNOWN",
                    "subscribe ack requestId does not match a pending command",
                )
                .with_request_id(Some(request_id.to_owned()))
                .with_retry_after_ms(Some(RECONNECT_DELAY_MS.into())));
            };
            return pending.remove(index).ok_or_else(|| {
                ws_problem(
                    "runtime",
                    "WS_ACK_REQUEST_ID_UNKNOWN",
                    "subscribe ack command disappeared before correlation",
                )
                .with_request_id(Some(request_id.to_owned()))
            });
        }
        pending.pop_front().ok_or_else(|| {
            ws_problem(
                "runtime",
                "WS_ACK_WITHOUT_PENDING_COMMAND",
                "legacy subscribe ack arrived without a pending command",
            )
        })
    }

    fn send_connected_command(&self, command: RuntimeCommand) -> Result<(), String> {
        match command {
            RuntimeCommand::Subscribe(channels) => {
                let request_id = next_request_id();
                self.send_frame(&subscribe_frame(&channels, &request_id))?;
                self.queue_pending_subscribe(request_id, channels);
                Ok(())
            }
            RuntimeCommand::Unsubscribe(channels) => self.send_frame(&unsubscribe_frame(&channels)),
        }
    }

    fn send_frame(&self, frame: &str) -> Result<(), String> {
        if self.current_connection().as_ref() != Some(&*self.inner.connection.borrow()) {
            return Err("websocket is not connected".to_owned());
        }
        let sender = self
            .inner
            .frame_sender
            .borrow()
            .as_ref()
            .cloned()
            .ok_or_else(|| "websocket is not connected".to_owned())?;
        sender(frame)
    }
}

fn sync_subscriber_state(subscriber: &RuntimeSubscriber, state: &WsChannelState) {
    subscriber.channel_state.set(state.clone());
}

fn restart_runtime_on_api_context_change(runtime: WsRuntime) {
    Effect::new(move |_| {
        runtime.inner.api_base.get();
        runtime.inner.api_auth_token.get();
        if runtime.current_connection().as_ref() != Some(&*runtime.inner.connection.borrow()) {
            runtime.restart();
        }
    });
}

fn active_channels_from(subscribers: &[RuntimeSubscriber]) -> Vec<String> {
    subscribers
        .iter()
        .map(|subscriber| subscriber.channel.to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn subscribe_frame(channels: &[String], request_id: &str) -> String {
    serde_json::json!({
        "type": "subscribe",
        "channels": channels,
        "requestId": request_id,
    })
    .to_string()
}

fn auth_frame(ticket: &str) -> String {
    serde_json::json!({
        "type": "auth",
        "ticket": ticket,
    })
    .to_string()
}

fn ws_auth_ticket_problem(error: ApiError) -> ApiProblem {
    let mut problem = error.problem;
    problem.code = "WS_AUTH_TICKET_FAILED".into();
    problem.source = Some("frontend-ws-runtime".into());
    problem.retry_after_ms = problem.retry_after_ms.or(Some(RECONNECT_DELAY_MS.into()));
    problem
}

fn ws_command_write_problem(error: impl std::fmt::Display) -> ApiProblem {
    ws_reconnect_problem("runtime", "WS_WRITE_ERROR", error)
}

fn subscription_ack_resolves_problem(problem: &ApiProblem) -> bool {
    matches!(
        problem.code.as_str(),
        "WS_OPEN_FAILED"
            | "WS_AUTH_FAILED"
            | "WS_SUBSCRIBE_FAILED"
            | "WS_READ_ERROR"
            | "WS_WRITE_ERROR"
    )
}

fn unsubscribe_frame(channels: &[String]) -> String {
    serde_json::json!({
        "type": "unsubscribe",
        "channels": channels,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_subscribe_frame_dedups_and_sorts_channels() {
        Owner::new().with(|| {
            let subscribers = vec![
                subscriber(1, "orders"),
                subscriber(2, "arbitrage"),
                subscriber(3, "orders"),
            ];

            let channels = active_channels_from(&subscribers);
            let frame = subscribe_frame(&channels, "web-subscribe-1");

            assert_eq!(channels, vec!["arbitrage", "orders"]);
            assert!(frame.contains(r#""type":"subscribe""#));
            assert!(frame.contains(r#""requestId":"web-subscribe-1""#));
            assert!(frame.contains(r#""arbitrage""#));
            assert!(frame.contains(r#""orders""#));
        });
    }

    #[test]
    fn runtime_unsubscribe_frame_uses_protocol_shape() {
        let frame = unsubscribe_frame(&["orders".into(), "system".into()]);

        assert!(frame.contains(r#""type":"unsubscribe""#));
        assert!(frame.contains(r#""orders""#));
        assert!(frame.contains(r#""system""#));
    }

    #[test]
    fn runtime_new_channel_uses_hot_subscribe_without_restart() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .push(subscriber(1, "orders"));
            let commands = observe_commands(&runtime);
            let generation = runtime.inner.generation.get();

            let _handle = runtime.subscribe(subscription(2, "execution"));
            let command = last_command(&commands);

            assert_eq!(runtime.inner.generation.get(), generation);
            assert!(matches!(
                command,
                Some(RuntimeCommand::Subscribe(channels)) if channels == vec!["execution"]
            ));
        });
    }

    #[test]
    fn runtime_existing_channel_inherits_status_without_resubscribe() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let existing = channel_state("orders", WsStatus::Connected);
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .push(subscriber_with_channel_state(1, "orders", existing));
            let commands = observe_commands(&runtime);
            let next_state = channel_state("orders", WsStatus::Disconnected);

            let _handle =
                runtime.subscribe(subscription_with_channel_state(2, "orders", next_state));
            let command = last_command(&commands);

            assert_eq!(next_state.get_untracked().status, WsStatus::Connected);
            assert!(command.is_none());
        });
    }

    #[test]
    fn runtime_existing_channel_inherits_full_state_without_resubscribe() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .push(subscriber(1, "orders"));
            runtime.record_channel_subscribed("orders");
            runtime.record_channel_message("orders");
            let problem =
                ApiProblem::new("WS_BROADCAST_LAGGED", "lagged").with_retry_after_ms(Some(2_000));
            let expected = runtime.record_channel_problem("orders", &problem, None);
            let commands = observe_commands(&runtime);
            let channel_state = RwSignal::new(WsChannelState::new("orders"));

            let _handle =
                runtime.subscribe(subscription_with_channel_state(2, "orders", channel_state));
            let command = last_command(&commands);

            assert!(command.is_none());
            assert_eq!(channel_state.get_untracked(), expected);
            assert_eq!(
                channel_state
                    .get_untracked()
                    .last_error
                    .as_ref()
                    .map(|problem| problem.code.as_str()),
                Some("WS_BROADCAST_LAGGED")
            );
            assert_eq!(channel_state.get_untracked().retry_after_ms, Some(2_000));
            assert!(channel_state.get_untracked().last_message_at_ms.is_some());
        });
    }

    #[test]
    fn runtime_subscribe_ack_preserves_unresolved_payload_problem() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let problem = ApiProblem::new("WS_PAYLOAD_DECODE", "bad order payload");

            runtime.record_channel_problem("orders", &problem, None);
            let subscribed = runtime.record_channel_subscribed("orders");
            let repeated = runtime.record_channel_problem("orders", &problem, None);

            assert_eq!(subscribed.status, WsStatus::Connected);
            assert!(subscribed.subscribed);
            assert_eq!(subscribed.last_error.as_ref(), Some(&problem));
            assert_eq!(repeated.problem_count, 1);
        });
    }

    #[test]
    fn runtime_subscribe_ack_clears_recovered_transport_problem() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let problem =
                ws_reconnect_problem("runtime", "WS_READ_ERROR", "connection interrupted");

            runtime.record_channel_problem("orders", &problem, Some(WsStatus::Disconnected));
            let subscribed = runtime.record_channel_subscribed("orders");

            assert_eq!(subscribed.status, WsStatus::Connected);
            assert!(subscribed.subscribed);
            assert!(subscribed.last_error.is_none());
            assert!(subscribed.retry_after_ms.is_none());
            assert_eq!(subscribed.problem_count, 1);
        });
    }

    #[test]
    fn runtime_removing_last_channel_uses_hot_unsubscribe_without_restart() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .extend([subscriber(1, "orders"), subscriber(2, "execution")]);
            let commands = observe_commands(&runtime);
            let generation = runtime.inner.generation.get();

            runtime.unsubscribe(1);
            let command = last_command(&commands);

            assert_eq!(runtime.inner.generation.get(), generation);
            assert!(matches!(
                command,
                Some(RuntimeCommand::Unsubscribe(channels)) if channels == vec!["orders"]
            ));
            assert_eq!(runtime.active_channels(), vec!["execution"]);
        });
    }

    #[test]
    fn runtime_ack_only_connects_subscribed_channels() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let orders = channel_state("orders", WsStatus::Disconnected);
            let execution = channel_state("execution", WsStatus::Disconnected);
            runtime.inner.subscribers.borrow_mut().extend([
                subscriber_with_channel_state(1, "orders", orders),
                subscriber_with_channel_state(2, "execution", execution),
            ]);
            runtime.queue_pending_subscribe(
                "web-legacy-ack".into(),
                vec!["orders".into(), "execution".into()],
            );

            runtime.apply_ack(&["orders".into()], None);

            assert_eq!(orders.get_untracked().status, WsStatus::Connected);
            assert_eq!(execution.get_untracked().status, WsStatus::Disconnected);
        });
    }

    #[test]
    fn runtime_hot_subscribe_ack_missing_requested_channel_reports_problem() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let orders_state = RwSignal::new(WsChannelState::new("orders"));
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .push(RuntimeSubscriber {
                    id: 1,
                    channel: "orders",
                    channel_state: orders_state,
                    on_payload: Rc::new(|_| Ok(())),
                    on_problem: Rc::new(|_| {}),
                });
            runtime.record_channel_subscribed("orders");
            let execution_state = RwSignal::new(WsChannelState::new("execution"));
            let execution_problem_count = Rc::new(Cell::new(0));
            let commands = observe_commands(&runtime);

            let _handle = runtime.subscribe(RuntimeSubscription {
                channel: "execution",
                channel_state: execution_state,
                on_payload: Rc::new(|_| Ok(())),
                on_problem: Rc::new({
                    let execution_problem_count = Rc::clone(&execution_problem_count);
                    move |_| execution_problem_count.set(execution_problem_count.get() + 1)
                }),
            });
            let command = last_command(&commands);
            assert!(matches!(
                command,
                Some(RuntimeCommand::Subscribe(channels)) if channels == vec!["execution"]
            ));
            runtime.queue_pending_subscribe("rid-hot-1".into(), vec!["execution".into()]);
            let frame = serde_json::json!({
                "type": "ack",
                "subscribed": ["orders"],
                "requestId": "rid-hot-1"
            })
            .to_string();

            assert!(runtime.handle_text(&frame));

            assert_eq!(execution_problem_count.get(), 1);
            assert_eq!(orders_state.get_untracked().last_error, None);
            assert_eq!(
                execution_state
                    .get_untracked()
                    .last_error
                    .as_ref()
                    .map(|problem| problem.code.as_str()),
                Some("WS_SUBSCRIBE_ACK_MISSING")
            );
            assert_eq!(
                execution_state
                    .get_untracked()
                    .last_error
                    .as_ref()
                    .and_then(|problem| problem.request_id.as_deref()),
                Some("rid-hot-1")
            );
        });
    }

    #[test]
    fn runtime_ack_missing_channel_preserves_request_id() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let channel_state = RwSignal::new(WsChannelState::new("execution"));
            let problem_count = Rc::new(Cell::new(0));
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .push(RuntimeSubscriber {
                    id: 1,
                    channel: "execution",
                    channel_state,
                    on_payload: Rc::new(|_| Ok(())),
                    on_problem: Rc::new({
                        let problem_count = Rc::clone(&problem_count);
                        move |_| problem_count.set(problem_count.get() + 1)
                    }),
                });
            runtime.queue_pending_subscribe("rid-ack-1".into(), vec!["execution".into()]);

            runtime.apply_ack(&[], Some("rid-ack-1"));

            let state = channel_state.get_untracked();
            assert_eq!(problem_count.get(), 1);
            assert_eq!(
                state
                    .last_error
                    .as_ref()
                    .map(|problem| problem.code.as_str()),
                Some("WS_SUBSCRIBE_ACK_MISSING")
            );
            assert_eq!(
                state
                    .last_error
                    .as_ref()
                    .and_then(|problem| problem.request_id.as_deref()),
                Some("rid-ack-1")
            );
        });
    }

    #[test]
    fn runtime_out_of_order_ack_correlates_by_request_id() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let orders = channel_state("orders", WsStatus::Disconnected);
            let execution = channel_state("execution", WsStatus::Disconnected);
            runtime.inner.subscribers.borrow_mut().extend([
                subscriber_with_channel_state(1, "orders", orders),
                subscriber_with_channel_state(2, "execution", execution),
            ]);
            runtime.queue_pending_subscribe("web-orders".into(), vec!["orders".into()]);
            runtime.queue_pending_subscribe("web-execution".into(), vec!["execution".into()]);

            runtime.apply_ack(&["execution".into()], Some("web-execution"));

            assert_eq!(orders.get_untracked().status, WsStatus::Disconnected);
            assert_eq!(execution.get_untracked().status, WsStatus::Connected);
            assert_eq!(runtime.inner.pending_subscribe_batches.borrow().len(), 1);

            runtime.apply_ack(&["orders".into()], Some("web-orders"));

            assert_eq!(orders.get_untracked().status, WsStatus::Connected);
            assert!(runtime.inner.pending_subscribe_batches.borrow().is_empty());
        });
    }

    #[test]
    fn runtime_unknown_ack_request_id_is_typed_and_non_connecting() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let channel_state = RwSignal::new(WsChannelState::new("orders"));
            let problem_count = Rc::new(Cell::new(0));
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .push(RuntimeSubscriber {
                    id: 1,
                    channel: "orders",
                    channel_state,
                    on_payload: Rc::new(|_| Ok(())),
                    on_problem: Rc::new({
                        let problem_count = Rc::clone(&problem_count);
                        move |_| problem_count.set(problem_count.get() + 1)
                    }),
                });
            runtime.queue_pending_subscribe("web-right".into(), vec!["orders".into()]);

            runtime.apply_ack(&["orders".into()], Some("web-wrong"));

            let state = channel_state.get_untracked();
            assert_eq!(problem_count.get(), 1);
            assert_eq!(state.status, WsStatus::Disconnected);
            assert_eq!(
                state
                    .last_error
                    .as_ref()
                    .map(|problem| problem.code.as_str()),
                Some("WS_ACK_REQUEST_ID_UNKNOWN")
            );
            assert_eq!(
                state
                    .last_error
                    .as_ref()
                    .and_then(|problem| problem.request_id.as_deref()),
                Some("web-wrong")
            );
            assert_eq!(runtime.inner.pending_subscribe_batches.borrow().len(), 1);
        });
    }

    #[test]
    fn runtime_batch_dispatches_only_matching_channel() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let orders_count = Rc::new(Cell::new(0));
            let execution_count = Rc::new(Cell::new(0));
            runtime.inner.subscribers.borrow_mut().extend([
                counting_subscriber(1, "orders", Rc::clone(&orders_count)),
                counting_subscriber(2, "execution", Rc::clone(&execution_count)),
            ]);

            runtime.deliver_batch(vec![
                ChannelPayload {
                    channel: "orders".into(),
                    payload: Value::Null,
                },
                ChannelPayload {
                    channel: "execution".into(),
                    payload: Value::Null,
                },
                ChannelPayload {
                    channel: "portfolio".into(),
                    payload: Value::Null,
                },
            ]);

            assert_eq!(orders_count.get(), 1);
            assert_eq!(execution_count.get(), 1);
        });
    }

    #[test]
    fn runtime_server_error_with_channel_is_scoped() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let orders_problem_count = Rc::new(Cell::new(0));
            let execution_problem_count = Rc::new(Cell::new(0));
            runtime.inner.subscribers.borrow_mut().extend([
                problem_counting_subscriber(1, "orders", Rc::clone(&orders_problem_count)),
                problem_counting_subscriber(2, "execution", Rc::clone(&execution_problem_count)),
            ]);

            let frame = serde_json::json!({
                "type": "error",
                "channel": "orders",
                "code": "WS_BROADCAST_LAGGED",
                "message": "channel orders lagged; skipped 3 broadcast message(s)",
                "retryAfterMs": 100,
                "requestId": "rid-lag"
            })
            .to_string();

            assert!(runtime.handle_text(&frame));
            assert_eq!(orders_problem_count.get(), 1);
            assert_eq!(execution_problem_count.get(), 0);
        });
    }

    #[test]
    fn runtime_successful_payload_is_counted_once() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let channel_state = channel_state("orders", WsStatus::Connected);
            let payload_count = Rc::new(Cell::new(0));
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .push(RuntimeSubscriber {
                    id: 1,
                    channel: "orders",
                    channel_state,
                    on_payload: Rc::new({
                        let payload_count = Rc::clone(&payload_count);
                        move |_| {
                            payload_count.set(payload_count.get() + 1);
                            Ok(())
                        }
                    }),
                    on_problem: Rc::new(|_| {}),
                });

            runtime.deliver("orders", &serde_json::json!({"id": "order-1"}));

            let state = channel_state.get_untracked();
            assert_eq!(payload_count.get(), 1);
            assert_eq!(state.message_count, 1);
            assert_eq!(state.problem_count, 0);
            assert!(state.last_message_at_ms.is_some());
        });
    }

    #[test]
    fn runtime_payload_problem_persists_for_late_subscriber() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            let first_state = channel_state("orders", WsStatus::Connected);
            let problem_count = Rc::new(Cell::new(0));
            runtime
                .inner
                .subscribers
                .borrow_mut()
                .push(RuntimeSubscriber {
                    id: 1,
                    channel: "orders",
                    channel_state: first_state,
                    on_payload: Rc::new(|_| {
                        Err(ApiProblem::new("WS_PAYLOAD_DECODE", "bad order payload")
                            .with_request_id(Some("req-payload-1".into())))
                    }),
                    on_problem: Rc::new({
                        let problem_count = Rc::clone(&problem_count);
                        move |_| problem_count.set(problem_count.get() + 1)
                    }),
                });

            runtime.deliver("orders", &Value::Null);

            let first = first_state.get_untracked();
            assert_eq!(problem_count.get(), 1);
            assert_eq!(first.message_count, 0);
            assert_eq!(first.problem_count, 1);
            assert_eq!(
                first
                    .last_error
                    .as_ref()
                    .and_then(|problem| problem.request_id.as_deref()),
                Some("req-payload-1")
            );

            let late_state = channel_state("orders", WsStatus::Disconnected);
            let _handle =
                runtime.subscribe(subscription_with_channel_state(2, "orders", late_state));
            assert_eq!(late_state.get_untracked(), first);
        });
    }

    #[test]
    fn inactive_channel_with_retained_state_resubscribes() {
        Owner::new().with(|| {
            let runtime = test_runtime();
            runtime.record_channel_subscribed("orders");
            let commands = observe_commands(&runtime);

            let _handle = runtime.subscribe(subscription(1, "orders"));
            let command = last_command(&commands);

            assert!(matches!(
                command,
                Some(RuntimeCommand::Subscribe(channels)) if channels == vec!["orders"]
            ));
        });
    }

    #[test]
    fn auth_ticket_and_write_problems_keep_retry_context() {
        let upstream = ApiProblem::new("RATE_LIMITED", "ticket limited")
            .with_status(429)
            .with_request_id(Some("req-ticket-1".into()))
            .with_retry_after_ms(Some(7_000));
        let ticket = ws_auth_ticket_problem(ApiError::from_problem(upstream));
        let write = ws_command_write_problem("socket closed");

        assert_eq!(ticket.code, "WS_AUTH_TICKET_FAILED");
        assert_eq!(ticket.status, Some(429));
        assert_eq!(ticket.request_id.as_deref(), Some("req-ticket-1"));
        assert_eq!(ticket.retry_after_ms, Some(7_000));
        assert_eq!(ticket.source.as_deref(), Some("frontend-ws-runtime"));
        assert_eq!(write.code, "WS_WRITE_ERROR");
        assert_eq!(write.retry_after_ms, Some(RECONNECT_DELAY_MS.into()));
    }

    #[test]
    fn runtime_auth_frame_uses_ticket_not_query() -> Result<(), serde_json::Error> {
        let frame = auth_frame("ticket-1");
        let value: Value = serde_json::from_str(&frame)?;

        assert_eq!(value["type"], "auth");
        assert_eq!(value["ticket"], "ticket-1");
        assert!(!frame.contains("Bearer"));
        assert!(!frame.contains("?ticket="));
        Ok(())
    }

    fn test_runtime() -> WsRuntime {
        WsRuntime::new(
            RwSignal::new("http://127.0.0.1:8000".into()),
            RwSignal::new(String::new()),
        )
    }

    fn observe_commands(runtime: &WsRuntime) -> Rc<RefCell<Vec<RuntimeCommand>>> {
        let commands = Rc::new(RefCell::new(Vec::new()));
        *runtime.inner.command_observer.borrow_mut() = Some(Rc::new({
            let commands = Rc::clone(&commands);
            move |command| commands.borrow_mut().push(command)
        }));
        commands
    }

    fn last_command(commands: &Rc<RefCell<Vec<RuntimeCommand>>>) -> Option<RuntimeCommand> {
        commands.borrow().last().cloned()
    }

    fn subscriber(id: u64, channel: &'static str) -> RuntimeSubscriber {
        subscriber_with_channel_state(id, channel, channel_state(channel, WsStatus::Disconnected))
    }

    fn subscription(id: u64, channel: &'static str) -> RuntimeSubscription {
        subscription_with_channel_state(id, channel, channel_state(channel, WsStatus::Disconnected))
    }

    fn subscription_with_channel_state(
        id: u64,
        channel: &'static str,
        channel_state: RwSignal<WsChannelState>,
    ) -> RuntimeSubscription {
        let _ = id;
        RuntimeSubscription {
            channel,
            channel_state,
            on_payload: Rc::new(|_| Ok(())),
            on_problem: Rc::new(|_| {}),
        }
    }

    fn subscriber_with_channel_state(
        id: u64,
        channel: &'static str,
        channel_state: RwSignal<WsChannelState>,
    ) -> RuntimeSubscriber {
        RuntimeSubscriber {
            id,
            channel,
            channel_state,
            on_payload: Rc::new(|_| Ok(())),
            on_problem: Rc::new(|_| {}),
        }
    }

    fn channel_state(channel: &str, status: WsStatus) -> RwSignal<WsChannelState> {
        let mut state = WsChannelState::new(channel);
        state.status = status;
        RwSignal::new(state)
    }

    fn counting_subscriber(
        id: u64,
        channel: &'static str,
        count: Rc<Cell<u32>>,
    ) -> RuntimeSubscriber {
        RuntimeSubscriber {
            id,
            channel,
            channel_state: channel_state(channel, WsStatus::Disconnected),
            on_payload: Rc::new(move |_| {
                count.set(count.get() + 1);
                Ok(())
            }),
            on_problem: Rc::new(|_| {}),
        }
    }

    fn problem_counting_subscriber(
        id: u64,
        channel: &'static str,
        count: Rc<Cell<u32>>,
    ) -> RuntimeSubscriber {
        RuntimeSubscriber {
            id,
            channel,
            channel_state: channel_state(channel, WsStatus::Disconnected),
            on_payload: Rc::new(|_| Ok(())),
            on_problem: Rc::new(move |_| count.set(count.get() + 1)),
        }
    }
}
