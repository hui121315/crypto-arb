//! Persistent WebSocket trade session actor.
//!
//! The exchange-specific adapters still own request signing and response
//! parsing. This module only keeps one authenticated socket warm and serializes
//! write requests through it, avoiding TCP/TLS/WS/login setup per order.

use crate::error::{ExchangeError, ExchangeResult};
use crate::ws::manager::WsHeartbeat;
use futures_util::{
    future::BoxFuture,
    stream::{SplitSink, SplitStream},
    SinkExt, StreamExt,
};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::time::{interval, timeout, Instant, MissedTickBehavior};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
use tokio_tungstenite::tungstenite::Message;

const SESSION_QUEUE_CAPACITY: usize = 256;
const HEARTBEAT_SECS: u64 = 25;
const INFLIGHT_SWEEP_MILLIS: u64 = 100;

pub(crate) type WsFinalPredicate = Box<dyn FnMut(&str) -> ExchangeResult<bool> + Send + 'static>;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type WsWriter = SplitSink<WsStream, Message>;
type WsReader = SplitStream<WsStream>;
type LoginPayloadBuilder = Arc<dyn Fn() -> ExchangeResult<String> + Send + Sync + 'static>;
type LoginResponseMatcher = Arc<dyn Fn(&str) -> ExchangeResult<bool> + Send + Sync + 'static>;
type ChallengeMatcher = Arc<dyn Fn(&str) -> ExchangeResult<bool> + Send + Sync + 'static>;
type ChallengeResponseBuilder = Arc<dyn Fn(&str) -> ExchangeResult<String> + Send + Sync + 'static>;
type UrlBuilder = Arc<dyn Fn() -> ExchangeResult<String> + Send + Sync + 'static>;
type HeartbeatResponseMatcher = Arc<dyn Fn(&str) -> bool + Send + Sync + 'static>;
type IncomingTextHandler = Arc<dyn Fn(&str) -> ExchangeResult<()> + Send + Sync + 'static>;
type PayloadBuilder = Box<dyn FnOnce() -> ExchangeResult<String> + Send + 'static>;
type ReconnectPayloadBuilder = Arc<dyn Fn() -> ExchangeResult<String> + Send + Sync + 'static>;
type ConnectPrepare =
    Arc<dyn Fn() -> BoxFuture<'static, ExchangeResult<()>> + Send + Sync + 'static>;

#[derive(Clone)]
pub(crate) struct WsTradeSession {
    tx: mpsc::Sender<WsCommand>,
    timeout: Duration,
}

pub(crate) struct WsLoginSpec {
    build_payload: LoginPayloadBuilder,
    is_response: LoginResponseMatcher,
}

pub(crate) struct WsChallengeLoginSpec {
    build_request: Option<LoginPayloadBuilder>,
    is_challenge: ChallengeMatcher,
    build_response: ChallengeResponseBuilder,
    is_response: LoginResponseMatcher,
}

pub(crate) struct WsSessionSpec {
    url: String,
    build_url: Option<UrlBuilder>,
    timeout: Duration,
    login: Option<WsLoginSpec>,
    challenge_login: Option<WsChallengeLoginSpec>,
    headers: Vec<(String, String)>,
    heartbeat: WsHeartbeat,
    heartbeat_interval: Duration,
    heartbeat_response: Option<HeartbeatResponseMatcher>,
    reconnect_payloads: Vec<ReconnectPayloadBuilder>,
    incoming_text_handler: Option<IncomingTextHandler>,
    connect_prepare: Option<ConnectPrepare>,
}

pub(crate) struct WsCommand {
    build_payload: Option<PayloadBuilder>,
    is_final: Option<WsFinalPredicate>,
    deadline: Instant,
    reply: oneshot::Sender<ExchangeResult<String>>,
}

struct WsConnection {
    write: WsWriter,
    read: WsReader,
}

struct WsInflight {
    is_final: WsFinalPredicate,
    deadline: Instant,
    reply: oneshot::Sender<ExchangeResult<String>>,
}

struct WsIncoming {
    generation: u64,
    event: WsIncomingEvent,
}

enum WsIncomingEvent {
    Text(String),
    Ping(Vec<u8>),
    Closed,
    Error(String),
}

impl WsTradeSession {
    pub(crate) fn spawn(spec: WsSessionSpec) -> Self {
        let timeout = spec.timeout;
        let (tx, rx) = mpsc::channel(SESSION_QUEUE_CAPACITY);
        tokio::spawn(run_session(spec, rx));
        Self { tx, timeout }
    }

    pub(crate) async fn send(
        &self,
        payload: String,
        is_final: WsFinalPredicate,
    ) -> ExchangeResult<String> {
        self.send_fresh(move || Ok(payload), is_final).await
    }

    /// Queues a request but creates its payload only after the socket is ready.
    /// Timestamped signatures therefore do not age during queueing or a cold
    /// TCP/TLS/WebSocket connection.
    pub(crate) async fn send_fresh(
        &self,
        build_payload: impl FnOnce() -> ExchangeResult<String> + Send + 'static,
        is_final: WsFinalPredicate,
    ) -> ExchangeResult<String> {
        let (reply, wait) = oneshot::channel();
        let command = WsCommand {
            build_payload: Some(Box::new(build_payload)),
            is_final: Some(is_final),
            deadline: Instant::now() + self.timeout,
            reply,
        };
        self.tx
            .send(command)
            .await
            .map_err(|_| ExchangeError::WsClosed("ws trade session stopped".into()))?;
        timeout(self.timeout, wait)
            .await
            .map_err(|_| ExchangeError::Timeout {
                seconds: self.timeout.as_secs(),
            })?
            .map_err(|_| ExchangeError::WsClosed("ws trade response dropped".into()))?
    }

    /// Establishes and authenticates the socket without sending a trade
    /// command. Reconnect payloads are sent before this returns.
    pub(crate) async fn warm(&self) -> ExchangeResult<()> {
        let (reply, wait) = oneshot::channel();
        self.tx
            .send(WsCommand {
                build_payload: None,
                is_final: None,
                deadline: Instant::now() + self.timeout,
                reply,
            })
            .await
            .map_err(|_| ExchangeError::WsClosed("ws trade session stopped".into()))?;
        timeout(self.timeout, wait)
            .await
            .map_err(|_| ExchangeError::Timeout {
                seconds: self.timeout.as_secs(),
            })?
            .map_err(|_| ExchangeError::WsClosed("ws trade warmup dropped".into()))??;
        Ok(())
    }
}

impl WsLoginSpec {
    pub(crate) fn new(
        build_payload: impl Fn() -> ExchangeResult<String> + Send + Sync + 'static,
        is_response: impl Fn(&str) -> ExchangeResult<bool> + Send + Sync + 'static,
    ) -> Self {
        Self {
            build_payload: Arc::new(build_payload),
            is_response: Arc::new(is_response),
        }
    }
}

impl WsChallengeLoginSpec {
    pub(crate) fn new(
        is_challenge: impl Fn(&str) -> ExchangeResult<bool> + Send + Sync + 'static,
        build_response: impl Fn(&str) -> ExchangeResult<String> + Send + Sync + 'static,
        is_response: impl Fn(&str) -> ExchangeResult<bool> + Send + Sync + 'static,
    ) -> Self {
        Self {
            build_request: None,
            is_challenge: Arc::new(is_challenge),
            build_response: Arc::new(build_response),
            is_response: Arc::new(is_response),
        }
    }

    /// Some challenge protocols require an explicit request after connect;
    /// others send the challenge immediately.
    pub(crate) fn with_request(
        mut self,
        build_request: impl Fn() -> ExchangeResult<String> + Send + Sync + 'static,
    ) -> Self {
        self.build_request = Some(Arc::new(build_request));
        self
    }
}

impl WsSessionSpec {
    pub(crate) fn new(url: impl Into<String>, timeout_secs: u64) -> Self {
        Self {
            url: url.into(),
            build_url: None,
            timeout: Duration::from_secs(timeout_secs),
            login: None,
            challenge_login: None,
            headers: Vec::new(),
            heartbeat: WsHeartbeat::PingFrame,
            heartbeat_interval: Duration::from_secs(HEARTBEAT_SECS),
            heartbeat_response: None,
            reconnect_payloads: Vec::new(),
            incoming_text_handler: None,
            connect_prepare: None,
        }
    }

    pub(crate) fn with_login(mut self, login: WsLoginSpec) -> Self {
        self.login = Some(login);
        self.challenge_login = None;
        self
    }

    pub(crate) fn with_fresh_url(
        mut self,
        build_url: impl Fn() -> ExchangeResult<String> + Send + Sync + 'static,
    ) -> Self {
        self.build_url = Some(Arc::new(build_url));
        self
    }

    pub(crate) fn with_challenge_login(mut self, login: WsChallengeLoginSpec) -> Self {
        self.login = None;
        self.challenge_login = Some(login);
        self
    }

    pub(crate) fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.push((name.into(), value.into()));
        self
    }

    pub(crate) fn with_heartbeat(mut self, heartbeat: WsHeartbeat) -> Self {
        self.heartbeat = heartbeat;
        self
    }

    pub(crate) fn with_heartbeat_interval(mut self, heartbeat_interval: Duration) -> Self {
        self.heartbeat_interval = heartbeat_interval;
        self
    }

    pub(crate) fn with_heartbeat_response(
        mut self,
        is_response: impl Fn(&str) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.heartbeat_response = Some(Arc::new(is_response));
        self
    }

    /// Sends a subscription/bootstrap payload after every successful login,
    /// including reconnects.
    pub(crate) fn with_reconnect_payload(
        mut self,
        build_payload: impl Fn() -> ExchangeResult<String> + Send + Sync + 'static,
    ) -> Self {
        self.reconnect_payloads.push(Arc::new(build_payload));
        self
    }

    /// Observes server-pushed frames that are not tied to one request. The
    /// handler also sees acknowledgements and should ignore unrelated frames.
    pub(crate) fn with_incoming_text_handler(
        mut self,
        handler: impl Fn(&str) -> ExchangeResult<()> + Send + Sync + 'static,
    ) -> Self {
        self.incoming_text_handler = Some(Arc::new(handler));
        self
    }

    /// Performs asynchronous per-connection preparation before the socket is
    /// opened. This is used for short-lived authentication tokens that must be
    /// refreshed on every reconnect.
    pub(crate) fn with_connect_prepare(
        mut self,
        prepare: impl Fn() -> BoxFuture<'static, ExchangeResult<()>> + Send + Sync + 'static,
    ) -> Self {
        self.connect_prepare = Some(Arc::new(prepare));
        self
    }
}

pub(crate) fn session_key(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        hasher.update((part.len() as u64).to_be_bytes());
        hasher.update(part.as_bytes());
    }
    hex::encode(&hasher.finalize()[..12])
}

async fn run_session(spec: WsSessionSpec, mut rx: mpsc::Receiver<WsCommand>) {
    let (incoming_tx, mut incoming_rx) = mpsc::channel(SESSION_QUEUE_CAPACITY);
    let mut write = None;
    let mut inflight = Vec::new();
    let mut generation = 0_u64;
    let mut heartbeat = interval(spec.heartbeat_interval);
    let mut expiry = interval(Duration::from_millis(INFLIGHT_SWEEP_MILLIS));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
    expiry.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            command = rx.recv() => {
                let Some(command) = command else { return };
                let should_reset = send_command(
                    &spec,
                    &mut write,
                    &incoming_tx,
                    &mut generation,
                    &mut inflight,
                    command,
                ).await;
                if should_reset {
                    reset_connection(&mut write, &mut inflight, &mut generation, "ws trade send failed");
                }
            }
            incoming = incoming_rx.recv() => {
                if let Some(incoming) = incoming {
                    if incoming.generation == generation
                        && handle_incoming(&spec, &mut write, &mut inflight, incoming).await.is_err()
                    {
                        reset_connection(&mut write, &mut inflight, &mut generation, "ws trade stream closed");
                    }
                }
            }
            _ = heartbeat.tick() => {
                if send_heartbeat(&mut write, &spec.heartbeat).await.is_err() {
                    reset_connection(&mut write, &mut inflight, &mut generation, "ws trade heartbeat failed");
                }
            }
            _ = expiry.tick() => expire_inflight(&mut inflight, spec.timeout),
        }
    }
}

async fn send_command(
    spec: &WsSessionSpec,
    write: &mut Option<WsWriter>,
    incoming_tx: &mpsc::Sender<WsIncoming>,
    generation: &mut u64,
    inflight: &mut Vec<WsInflight>,
    command: WsCommand,
) -> bool {
    let WsCommand {
        build_payload,
        is_final,
        deadline,
        reply,
    } = command;
    if deadline <= Instant::now() {
        send_timeout(reply, spec.timeout);
        return false;
    }
    if let Err(error) = ensure_connected(spec, write, incoming_tx, generation).await {
        let _ = reply.send(Err(error));
        return false;
    }
    if deadline <= Instant::now() {
        send_timeout(reply, spec.timeout);
        return false;
    }
    let Some(build_payload) = build_payload else {
        let _ = reply.send(Ok(String::new()));
        return false;
    };
    let Some(is_final) = is_final else {
        let _ = reply.send(Err(ExchangeError::Parse(
            "ws trade command missing response matcher".into(),
        )));
        return false;
    };
    let payload = match build_payload() {
        Ok(payload) => payload,
        Err(error) => {
            let _ = reply.send(Err(error));
            return false;
        }
    };
    let Some(sink) = write.as_mut() else {
        let _ = reply.send(Err(ExchangeError::WsClosed("missing ws writer".into())));
        return true;
    };
    if let Err(error) = sink.send(Message::Text(payload)).await {
        let _ = reply.send(Err(ExchangeError::WsClosed(error.to_string())));
        return true;
    }
    inflight.push(WsInflight {
        is_final,
        deadline,
        reply,
    });
    false
}

async fn ensure_connected(
    spec: &WsSessionSpec,
    write: &mut Option<WsWriter>,
    incoming_tx: &mpsc::Sender<WsIncoming>,
    generation: &mut u64,
) -> ExchangeResult<()> {
    if write.is_some() {
        return Ok(());
    }
    let connection = connect_and_login(spec).await?;
    *generation = generation.wrapping_add(1);
    spawn_reader(*generation, connection.read, incoming_tx.clone());
    *write = Some(connection.write);
    Ok(())
}

async fn connect_and_login(spec: &WsSessionSpec) -> ExchangeResult<WsConnection> {
    if let Some(prepare) = &spec.connect_prepare {
        prepare().await?;
    }
    let url = spec
        .build_url
        .as_ref()
        .map_or_else(|| Ok(spec.url.clone()), |build_url| build_url())?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
    for (name, value) in &spec.headers {
        let name = HeaderName::from_bytes(name.as_bytes())
            .map_err(|error| ExchangeError::Parse(format!("ws header name {name}: {error}")))?;
        let value = HeaderValue::from_str(value)
            .map_err(|error| ExchangeError::Parse(format!("ws header value: {error}")))?;
        request.headers_mut().insert(name, value);
    }
    let (mut stream, _) = {
        let connect_permit = super::connect_budget::acquire(url.as_str())
            .await
            .map_err(ExchangeError::WsClosed)?;
        let stream = timeout(spec.timeout, connect_async(request))
            .await
            .map_err(|_| ExchangeError::Timeout {
                seconds: spec.timeout.as_secs().max(1),
            })?
            .map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
        connect_permit.mark_succeeded();
        stream
    };
    if let Some(login) = &spec.login {
        let payload = (login.build_payload)()?;
        stream
            .send(Message::Text(payload))
            .await
            .map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
        wait_for_login(&mut stream, login, spec.timeout).await?;
    } else if let Some(login) = &spec.challenge_login {
        if let Some(build_request) = &login.build_request {
            stream
                .send(Message::Text(build_request()?))
                .await
                .map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
        }
        wait_for_challenge_login(&mut stream, login, spec.timeout).await?;
    }
    for build_payload in &spec.reconnect_payloads {
        let payload = build_payload()?;
        stream
            .send(Message::Text(payload))
            .await
            .map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
    }
    let (write, read) = stream.split();
    Ok(WsConnection { write, read })
}

async fn wait_for_challenge_login(
    stream: &mut WsStream,
    login: &WsChallengeLoginSpec,
    wait: Duration,
) -> ExchangeResult<()> {
    timeout(wait, async {
        while let Some(message) = stream.next().await {
            let message = message.map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
            let Message::Text(text) = message else {
                continue;
            };
            if !(login.is_challenge)(text.as_ref())? {
                continue;
            }
            let response = (login.build_response)(text.as_ref())?;
            stream
                .send(Message::Text(response))
                .await
                .map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
            while let Some(message) = stream.next().await {
                let message =
                    message.map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
                if let Message::Text(text) = message {
                    if (login.is_response)(text.as_ref())? {
                        return Ok(());
                    }
                }
            }
        }
        Err(ExchangeError::WsClosed(
            "ws trade ended before challenge login completed".into(),
        ))
    })
    .await
    .map_err(|_| ExchangeError::Timeout {
        seconds: wait.as_secs(),
    })?
}

async fn wait_for_login(
    stream: &mut WsStream,
    login: &WsLoginSpec,
    wait: Duration,
) -> ExchangeResult<()> {
    timeout(wait, async {
        while let Some(message) = stream.next().await {
            let message = message.map_err(|error| ExchangeError::WsClosed(error.to_string()))?;
            if let Message::Text(text) = message {
                if (login.is_response)(text.as_ref())? {
                    return Ok(());
                }
            }
        }
        Err(ExchangeError::WsClosed(
            "ws trade ended before login response".into(),
        ))
    })
    .await
    .map_err(|_| ExchangeError::Timeout {
        seconds: wait.as_secs(),
    })?
}

fn spawn_reader(generation: u64, mut read: WsReader, incoming_tx: mpsc::Sender<WsIncoming>) {
    tokio::spawn(async move {
        while let Some(message) = read.next().await {
            let event = match message {
                Ok(Message::Text(text)) => WsIncomingEvent::Text(text),
                Ok(Message::Ping(payload)) => WsIncomingEvent::Ping(payload),
                Ok(Message::Close(_)) => WsIncomingEvent::Closed,
                Ok(_) => continue,
                Err(error) => WsIncomingEvent::Error(error.to_string()),
            };
            let closed = matches!(event, WsIncomingEvent::Closed | WsIncomingEvent::Error(_));
            if incoming_tx
                .send(WsIncoming { generation, event })
                .await
                .is_err()
                || closed
            {
                return;
            }
        }
        let _ = incoming_tx
            .send(WsIncoming {
                generation,
                event: WsIncomingEvent::Closed,
            })
            .await;
    });
}

async fn handle_incoming(
    spec: &WsSessionSpec,
    write: &mut Option<WsWriter>,
    inflight: &mut Vec<WsInflight>,
    incoming: WsIncoming,
) -> ExchangeResult<()> {
    match incoming.event {
        WsIncomingEvent::Text(text) => {
            if let Some(handler) = &spec.incoming_text_handler {
                handler(&text)?;
            }
            if !is_heartbeat_response(spec, &text) {
                handle_text(inflight, text);
            }
            Ok(())
        }
        WsIncomingEvent::Ping(payload) => {
            let Some(sink) = write.as_mut() else {
                return Err(ExchangeError::WsClosed("missing ws writer".into()));
            };
            sink.send(Message::Pong(payload))
                .await
                .map_err(|error| ExchangeError::WsClosed(error.to_string()))
        }
        WsIncomingEvent::Closed => Err(ExchangeError::WsClosed("ws trade stream ended".into())),
        WsIncomingEvent::Error(error) => Err(ExchangeError::WsClosed(error)),
    }
}

fn is_heartbeat_response(spec: &WsSessionSpec, text: &str) -> bool {
    spec.heartbeat_response
        .as_ref()
        .is_some_and(|matcher| matcher(text))
}

fn handle_text(inflight: &mut Vec<WsInflight>, text: String) {
    let mut index = 0;
    while index < inflight.len() {
        let matched = {
            let pending = &mut inflight[index];
            (pending.is_final)(&text)
        };
        match matched {
            Ok(true) => {
                let pending = inflight.swap_remove(index);
                let _ = pending.reply.send(Ok(text));
                return;
            }
            Ok(false) => index += 1,
            Err(error) => {
                let pending = inflight.swap_remove(index);
                let _ = pending.reply.send(Err(error));
                return;
            }
        }
    }
}

fn expire_inflight(inflight: &mut Vec<WsInflight>, wait: Duration) {
    let now = Instant::now();
    let mut index = 0;
    while index < inflight.len() {
        if inflight[index].deadline <= now {
            let pending = inflight.swap_remove(index);
            send_timeout(pending.reply, wait);
        } else {
            index += 1;
        }
    }
}

fn send_timeout(reply: oneshot::Sender<ExchangeResult<String>>, wait: Duration) {
    let _ = reply.send(Err(timeout_error(wait)));
}

fn timeout_error(wait: Duration) -> ExchangeError {
    ExchangeError::Timeout {
        seconds: wait.as_secs(),
    }
}

fn reset_connection(
    write: &mut Option<WsWriter>,
    inflight: &mut Vec<WsInflight>,
    generation: &mut u64,
    reason: &str,
) {
    *write = None;
    *generation = generation.wrapping_add(1);
    fail_inflight(inflight, reason);
}

fn fail_inflight(inflight: &mut Vec<WsInflight>, reason: &str) {
    for pending in inflight.drain(..) {
        let _ = pending
            .reply
            .send(Err(ExchangeError::WsClosed(reason.to_owned())));
    }
}

async fn send_heartbeat(
    write: &mut Option<WsWriter>,
    heartbeat: &WsHeartbeat,
) -> ExchangeResult<()> {
    let Some(sink) = write else {
        return Ok(());
    };
    sink.send(heartbeat_message(heartbeat))
        .await
        .map_err(|error| ExchangeError::WsClosed(error.to_string()))
}

fn heartbeat_message(heartbeat: &WsHeartbeat) -> Message {
    match heartbeat {
        WsHeartbeat::PingFrame => Message::Ping(Vec::new()),
        WsHeartbeat::Text(payload) => Message::Text(payload.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trade_session_supports_protocol_text_heartbeat() {
        let spec = WsSessionSpec::new("wss://example.test", 1)
            .with_heartbeat(WsHeartbeat::Text("ping".to_owned()))
            .with_heartbeat_interval(Duration::from_secs(20))
            .with_heartbeat_response(|text| text == "pong");
        assert_eq!(spec.heartbeat, WsHeartbeat::Text("ping".to_owned()));
        assert_eq!(spec.heartbeat_interval, Duration::from_secs(20));
        assert!(is_heartbeat_response(&spec, "pong"));
        assert!(!is_heartbeat_response(&spec, "order response"));
        assert_eq!(
            heartbeat_message(&spec.heartbeat),
            Message::Text("ping".to_owned())
        );
    }

    #[test]
    fn trade_session_preserves_handshake_headers() {
        let spec =
            WsSessionSpec::new("wss://example.test", 1).with_header("X-Gate-Size-Decimal", "1");

        assert_eq!(
            spec.headers,
            vec![("X-Gate-Size-Decimal".to_owned(), "1".to_owned())]
        );
    }

    #[test]
    fn trade_session_timeout_reports_configured_seconds() {
        assert!(matches!(
            timeout_error(Duration::from_secs(10)),
            ExchangeError::Timeout { seconds: 10 }
        ));
    }
}
