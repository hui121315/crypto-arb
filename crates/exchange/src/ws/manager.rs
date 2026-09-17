//! 通用 WebSocket supervisor。
//!
//! 职责：
//! 1. 建立 WS 连接，失败时按指数退避（含抖动）重连
//! 2. 周期性发送心跳（默认 30s）
//! 3. 连续失败次数超过阈值时打开熔断器，向订阅者广播
//! 4. 订阅恢复：调用方在 [`WsEvent::Connected`] 回调中重新发送 subscribe 消息
//!
//! 具体的 ping 报文格式与订阅协议由各家适配器自行决定，本层只提供消息收发管道。

use crate::error::{ExchangeError, ExchangeResult};
use arc_swap::{ArcSwap, ArcSwapOption};
use dashmap::DashMap;
use flate2::read::GzDecoder;
use futures_util::{SinkExt, StreamExt};
use hpx_yawc::frame::{Frame as DeflateFrame, OpCode as DeflateOpCode};
use hpx_yawc::{Options as DeflateOptions, TcpWebSocket as DeflateWsStream, WebSocket};
use rand::Rng;
use std::collections::BTreeSet;
use std::fmt;
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, Mutex, Notify, RwLock};
use tokio::time::sleep;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Error as WsError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{
    client_async_tls_with_config, connect_async, MaybeTlsStream, WebSocketStream,
};
use tracing::{debug, error, info, warn};

#[derive(Clone)]
pub struct WsConfig {
    pub url: String,
    pub exchange: String,
    pub heartbeat_interval: Duration,
    pub heartbeat: WsHeartbeat,
    pub inbound_codec: WsInboundCodec,
    pub server_ping: WsServerPing,
    pub initial_reconnect_delay: Duration,
    pub max_reconnect_delay: Duration,
    pub circuit_breaker_threshold: u32,
}

impl fmt::Debug for WsConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WsConfig")
            .field("url", &diagnostic_ws_url(&self.exchange, &self.url))
            .field("exchange", &self.exchange)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("heartbeat", &self.heartbeat)
            .field("inbound_codec", &self.inbound_codec)
            .field("server_ping", &self.server_ping)
            .field("initial_reconnect_delay", &self.initial_reconnect_delay)
            .field("max_reconnect_delay", &self.max_reconnect_delay)
            .field("circuit_breaker_threshold", &self.circuit_breaker_threshold)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsHeartbeat {
    PingFrame,
    Text(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsInboundCodec {
    Plain,
    Utf8Text,
    GzipText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsServerPing {
    None,
}

impl Default for WsConfig {
    fn default() -> Self {
        Self {
            url: String::new(),
            exchange: "unknown".to_owned(),
            heartbeat_interval: Duration::from_secs(30),
            heartbeat: WsHeartbeat::PingFrame,
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(60),
            circuit_breaker_threshold: 5,
        }
    }
}

/// WS 连接状态机。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsState {
    Disconnected,
    Connecting,
    Connected,
    CircuitOpen,
}

/// WS 事件（向订阅者广播）。
#[derive(Debug, Clone)]
pub enum WsEvent {
    Connected,
    Disconnected(String),
    CircuitOpened,
    Text(String),
    Binary(Vec<u8>),
}

/// WS ingest 可解释性快照：消息量与解码失败累计 + 最近一次失败原因。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WsIngestStats {
    pub text_messages: u64,
    pub text_bytes: u64,
    pub binary_messages: u64,
    pub binary_bytes: u64,
    pub decode_failures: u64,
    pub ping_probe_failures: u64,
    pub send_failures: u64,
    pub last_failure: Option<String>,
}

/// 单个 WS 连接的 ingest 快照（`exchange` + 连接 URL 维度）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsIngestSnapshot {
    pub instance_id: u64,
    pub exchange: String,
    pub url: String,
    pub callsite: String,
    pub stats: WsIngestStats,
}

#[derive(Debug)]
struct IngestRegistration {
    exchange: String,
    url: String,
    callsite: String,
    counters: Weak<IngestCounters>,
}

type IngestRegistry = DashMap<u64, IngestRegistration>;

static NEXT_MANAGER_ID: AtomicU64 = AtomicU64::new(1);
static INGEST_REGISTRY: LazyLock<IngestRegistry> = LazyLock::new(DashMap::new);
static RUSTLS_PROVIDER_READY: LazyLock<()> = LazyLock::new(|| {
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        let _ = rustls::crypto::ring::default_provider().install_default();
    }
});
const DEFAULT_WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(12);
const KRAKEN_WS_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(30);
// The Shadowrocket TUN route on this workstation consistently needs a little
// over 10s to finish Kraken's TLS/WebSocket handshake. Keep enough headroom for
// that route while preserving time for a fresh HTTP CONNECT fallback.
const MACOS_SYSTEM_ROUTE_WS_BUDGET: Duration = Duration::from_secs(16);
const PRIORITY_RECONNECT_MIN_INTERVAL: Duration = Duration::from_secs(10);

/// 全部存活 WS 连接的 ingest 快照（诊断消费口径；已释放的连接会被剔除）。
pub fn ingest_snapshots() -> Vec<WsIngestSnapshot> {
    INGEST_REGISTRY.retain(|_, registration| registration.counters.strong_count() > 0);
    let mut snapshots: Vec<WsIngestSnapshot> = INGEST_REGISTRY
        .iter()
        .filter_map(|entry| {
            let instance_id = *entry.key();
            let registration = entry.value();
            registration
                .counters
                .upgrade()
                .map(|counters| WsIngestSnapshot {
                    instance_id,
                    url: diagnostic_ws_url(&registration.exchange, &registration.url),
                    exchange: registration.exchange.clone(),
                    callsite: registration.callsite.clone(),
                    stats: counters.snapshot(),
                })
        })
        .collect();
    snapshots.sort_by(|a, b| {
        (&a.exchange, &a.url, a.instance_id).cmp(&(&b.exchange, &b.url, b.instance_id))
    });
    snapshots
}

#[derive(Debug, Default)]
struct IngestCounters {
    text_messages: AtomicU64,
    text_bytes: AtomicU64,
    binary_messages: AtomicU64,
    binary_bytes: AtomicU64,
    decode_failures: AtomicU64,
    ping_probe_failures: AtomicU64,
    send_failures: AtomicU64,
    last_failure: ArcSwapOption<String>,
}

impl IngestCounters {
    fn record_failure(&self, kind: &str, error: &dyn fmt::Display) {
        self.last_failure
            .store(Some(Arc::new(format!("{kind}: {error}"))));
    }

    fn snapshot(&self) -> WsIngestStats {
        WsIngestStats {
            text_messages: self.text_messages.load(Ordering::Relaxed),
            text_bytes: self.text_bytes.load(Ordering::Relaxed),
            binary_messages: self.binary_messages.load(Ordering::Relaxed),
            binary_bytes: self.binary_bytes.load(Ordering::Relaxed),
            decode_failures: self.decode_failures.load(Ordering::Relaxed),
            ping_probe_failures: self.ping_probe_failures.load(Ordering::Relaxed),
            send_failures: self.send_failures.load(Ordering::Relaxed),
            last_failure: self.last_failure.load_full().map(|e| (*e).clone()),
        }
    }
}

type PlainWsStream = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
type PlainWriter = futures_util::stream::SplitSink<PlainWsStream, Message>;
type PlainReader = futures_util::stream::SplitStream<PlainWsStream>;
type DeflateWriter = futures_util::stream::SplitSink<DeflateWsStream, DeflateFrame>;
type DeflateReader = futures_util::stream::SplitStream<DeflateWsStream>;
type Writer = Arc<Mutex<Option<ActiveWriter>>>;
const STABLE_SESSION_MIN_UPTIME: Duration = Duration::from_secs(30);

enum ConnectedStream {
    Plain(Box<PlainWsStream>),
    Deflate(Box<DeflateWsStream>),
}

enum ActiveWriter {
    Plain(PlainWriter),
    Deflate(DeflateWriter),
}

enum ActiveReader {
    Plain(PlainReader),
    Deflate(DeflateReader),
}

impl ConnectedStream {
    fn split(self) -> (ActiveWriter, ActiveReader) {
        match self {
            Self::Plain(stream) => {
                let (writer, reader) = (*stream).split();
                (ActiveWriter::Plain(writer), ActiveReader::Plain(reader))
            }
            Self::Deflate(stream) => {
                let (writer, reader) = (*stream).split();
                (ActiveWriter::Deflate(writer), ActiveReader::Deflate(reader))
            }
        }
    }
}

impl ActiveWriter {
    async fn send(&mut self, message: Message) -> Result<(), String> {
        match self {
            Self::Plain(writer) => writer
                .send(message)
                .await
                .map_err(|error| error.to_string()),
            Self::Deflate(writer) => writer
                .send(message_to_deflate_frame(message)?)
                .await
                .map_err(|error| error.to_string()),
        }
    }
}

impl ActiveReader {
    async fn next_message(&mut self) -> Option<Result<Message, String>> {
        match self {
            Self::Plain(reader) => reader
                .next()
                .await
                .map(|result| result.map_err(|error| error.to_string())),
            Self::Deflate(reader) => reader.next().await.map(deflate_frame_to_message),
        }
    }
}

enum SessionStep {
    Continue,
    Break(String),
}

struct SessionOutcome {
    reason: String,
    uptime: Duration,
}

pub struct WsManager {
    instance_id: u64,
    config: WsConfig,
    connect_url: ArcSwap<String>,
    connect_headers: Vec<(&'static str, &'static str)>,
    state: Arc<RwLock<WsState>>,
    consecutive_failures: Arc<RwLock<u32>>,
    tx: broadcast::Sender<WsEvent>,
    bootstrap_receiver: std::sync::Mutex<Option<broadcast::Receiver<WsEvent>>>,
    writer: Writer,
    ingest: Arc<IngestCounters>,
    connection_problem: ArcSwapOption<String>,
    active: AtomicBool,
    activity_changed: Notify,
    demand_scopes: std::sync::Mutex<BTreeSet<&'static str>>,
    priority_connect: bool,
    priority_reconnect_requested: AtomicBool,
    last_priority_reconnect: std::sync::Mutex<Option<Instant>>,
}

impl fmt::Debug for WsManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WsManager")
            .field("instance_id", &self.instance_id)
            .field("config", &self.config)
            .field("tx_receiver_count", &self.tx.receiver_count())
            .finish_non_exhaustive()
    }
}

impl WsManager {
    #[track_caller]
    pub fn new(config: WsConfig) -> Self {
        Self::new_with_event_capacity(config, 1_024)
    }

    #[track_caller]
    pub fn new_with_event_capacity(config: WsConfig, event_capacity: usize) -> Self {
        LazyLock::force(&RUSTLS_PROVIDER_READY);
        // Keep the channel's first receiver until the first dispatcher
        // subscribes. Without it, a fast handshake can publish Connected
        // before adapter construction reaches run_dispatch(), permanently
        // losing the only event that sends venue subscription commands.
        let (tx, bootstrap_receiver) = broadcast::channel(event_capacity.max(1));
        let ingest = Arc::new(IngestCounters::default());
        let instance_id = NEXT_MANAGER_ID.fetch_add(1, Ordering::Relaxed);
        let caller = std::panic::Location::caller();
        INGEST_REGISTRY.insert(
            instance_id,
            IngestRegistration {
                exchange: config.exchange.clone(),
                url: config.url.clone(),
                callsite: format!("{}:{}", caller.file(), caller.line()),
                counters: Arc::downgrade(&ingest),
            },
        );
        Self {
            instance_id,
            connect_url: ArcSwap::from_pointee(config.url.clone()),
            config,
            connect_headers: Vec::new(),
            state: Arc::new(RwLock::new(WsState::Disconnected)),
            consecutive_failures: Arc::new(RwLock::new(0)),
            tx,
            bootstrap_receiver: std::sync::Mutex::new(Some(bootstrap_receiver)),
            writer: Arc::new(Mutex::new(None)),
            ingest,
            connection_problem: ArcSwapOption::empty(),
            active: AtomicBool::new(true),
            activity_changed: Notify::new(),
            demand_scopes: std::sync::Mutex::new(BTreeSet::new()),
            priority_connect: false,
            priority_reconnect_requested: AtomicBool::new(false),
            last_priority_reconnect: std::sync::Mutex::new(None),
        }
    }

    /// Start a public stream dormant. A consumer calls [`Self::activate_scope`] on first demand.
    pub fn with_demand_control(self) -> Self {
        self.active.store(false, Ordering::Release);
        self
    }

    /// Reserve the bounded operator-critical reconnect lane for this manager.
    pub fn with_priority_connect(mut self) -> Self {
        self.priority_connect = true;
        self
    }

    pub fn with_connect_header(mut self, name: &'static str, value: &'static str) -> Self {
        self.connect_headers.push((name, value));
        self
    }

    /// 当前 ingest 累计口径（进程内，不随重连重置）。
    pub fn ingest_stats(&self) -> WsIngestStats {
        self.ingest.snapshot()
    }

    /// Current connection failure only. A successful handshake clears it so
    /// callers never mistake an old reconnect error for the live session state.
    pub fn connection_problem(&self) -> Option<String> {
        self.connection_problem
            .load_full()
            .map(|problem| (*problem).clone())
    }

    pub fn config(&self) -> &WsConfig {
        &self.config
    }

    /// Replace the URL used by the next connection attempt. Existing sessions
    /// stay untouched; this is primarily used for expiring KuCoin bullet tokens.
    pub fn replace_connect_url(&self, url: String) -> ExchangeResult<()> {
        url.as_str().into_client_request().map_err(|error| {
            ExchangeError::Parse(format!("invalid websocket refresh URL: {error}"))
        })?;
        self.connect_url.store(Arc::new(url));
        Ok(())
    }

    pub fn subscribe(&self) -> broadcast::Receiver<WsEvent> {
        self.bootstrap_receiver
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            .unwrap_or_else(|| self.tx.subscribe())
    }

    pub async fn state(&self) -> WsState {
        *self.state.read().await
    }

    pub async fn is_connected(&self) -> bool {
        matches!(self.state().await, WsState::Connected)
    }

    /// Resume a deliberately suspended public stream. The supervisor reconnects on demand.
    pub fn activate(&self) {
        if !self.active.swap(true, Ordering::AcqRel) {
            self.activity_changed.notify_waiters();
        }
    }

    /// Keep a demand-controlled connection alive for one logical public feed.
    pub fn activate_scope(&self, scope: &'static str) {
        let mut scopes = self
            .demand_scopes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        scopes.insert(scope);
        self.activate();
    }

    /// Wake a reconnect backoff for a newly important exact feed without
    /// allowing a hot read loop to create a reconnect storm.
    pub fn prioritize_reconnect(&self) {
        let now = Instant::now();
        let mut last = self
            .last_priority_reconnect
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if last
            .is_some_and(|previous| now.duration_since(previous) < PRIORITY_RECONNECT_MIN_INTERVAL)
        {
            return;
        }
        *last = Some(now);
        self.priority_reconnect_requested
            .store(true, Ordering::Release);
        self.connection_problem.store(None);
        self.activity_changed.notify_waiters();
    }

    /// Release one logical public feed and stop reconnecting once no feed still needs the socket.
    pub async fn suspend_scope(&self, scope: &'static str) {
        let should_suspend = {
            let mut scopes = self
                .demand_scopes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            scopes.remove(scope);
            if scopes.is_empty() {
                let was_active = self.active.swap(false, Ordering::AcqRel);
                if was_active {
                    self.activity_changed.notify_waiters();
                }
                was_active
            } else {
                false
            }
        };
        if should_suspend {
            self.close_if_suspended().await;
        }
    }

    /// Stop the current session and suppress reconnects until [`Self::activate`] is called.
    pub async fn suspend(&self) {
        if !self.active.swap(false, Ordering::AcqRel) {
            return;
        }
        self.activity_changed.notify_waiters();
        self.close_if_suspended().await;
    }

    async fn close_if_suspended(&self) {
        let mut writer = self.writer.lock().await;
        if !self.is_active() {
            if let Some(writer) = writer.as_mut() {
                let _ = writer.send(Message::Close(None)).await;
            }
        }
    }

    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire)
    }

    /// 向连接中的对端发送一条消息（如订阅请求 / pong）。连接未就绪返回错误。
    pub async fn send(&self, msg: Message) -> ExchangeResult<()> {
        let outcome = self.send_inner(msg).await;
        if let Err(error) = &outcome {
            self.ingest.send_failures.fetch_add(1, Ordering::Relaxed);
            self.ingest.record_failure("send", error);
        }
        outcome
    }

    async fn send_inner(&self, msg: Message) -> ExchangeResult<()> {
        let mut guard = self.writer.lock().await;
        let writer = guard
            .as_mut()
            .ok_or_else(|| ExchangeError::WsClosed("not connected".into()))?;
        writer.send(msg).await.map_err(ExchangeError::WsClosed)
    }

    pub async fn send_text(&self, text: String) -> ExchangeResult<()> {
        self.send(Message::Text(text)).await
    }

    /// 启动 supervisor 主循环：连接 → 维持 → 失败重连；熔断打开时冷却后半开重试，
    /// 循环只会被外部 abort 终止。
    ///
    /// 历史教训：熔断曾直接 `return Err` 终止 supervisor，而所有 adapter 的启动
    /// 代码都是一次性 `tokio::spawn` + `warn!`，结果一次几分钟的交易所维护足以让
    /// 该 venue 的行情永久断流、直到进程重启。
    pub async fn run(self: Arc<Self>) -> ExchangeResult<()> {
        loop {
            self.wait_until_active().await;
            self.ensure_circuit_closed().await;
            if !self.is_active() {
                continue;
            }
            match self.connect_and_run_once().await {
                Ok(outcome) if self.is_active() => self.disconnect_session(outcome).await,
                Ok(_) => self.mark_suspended().await,
                Err(error) if self.is_active() => self.record_connect_error(error).await,
                Err(_) => self.mark_suspended().await,
            }
            if self.is_active() {
                self.sleep_before_reconnect().await;
            }
        }
    }

    async fn wait_until_active(&self) {
        while !self.is_active() {
            self.activity_changed.notified().await;
        }
    }

    async fn ensure_circuit_closed(&self) {
        if self.consume_priority_reconnect().await {
            return;
        }
        if *self.consecutive_failures.read().await < self.config.circuit_breaker_threshold {
            return;
        }
        let cooldown = circuit_cooldown(self.config.max_reconnect_delay);
        warn!(
            exchange = %self.config.exchange,
            cooldown_ms = cooldown.as_millis() as u64,
            "circuit breaker open; half-open retry after cooldown"
        );
        *self.state.write().await = WsState::CircuitOpen;
        let _ = self.tx.send(WsEvent::CircuitOpened);
        if self.circuit_cooldown_interrupted(cooldown).await {
            return;
        }
        // 半开：失败计数减半（而非清零），试探失败会快速再次打开熔断，
        // 成功且会话稳定则由 next_session_failure_count 归零。
        let mut failures = self.consecutive_failures.write().await;
        *failures /= 2;
    }

    async fn connect_and_run_once(&self) -> Result<SessionOutcome, String> {
        *self.state.write().await = WsState::Connecting;
        let connect_url = self.connect_url.load_full();
        info!(
            exchange = %self.config.exchange,
            url = %diagnostic_ws_url(&self.config.exchange, connect_url.as_str()),
            "ws connecting"
        );
        let stream = self.connect_stream(connect_url.as_str()).await?;
        let connected_at = Instant::now();
        Ok(SessionOutcome {
            reason: self.run_session(stream).await,
            uptime: connected_at.elapsed(),
        })
    }

    async fn connect_stream(&self, url: &str) -> Result<ConnectedStream, String> {
        let connect_permit = if self.priority_connect {
            super::connect_budget::acquire_priority(url).await?
        } else {
            super::connect_budget::acquire(url).await?
        };
        let handshake_timeout = ws_handshake_timeout(url);
        let stream = tokio::time::timeout(handshake_timeout, async {
            if should_offer_permessage_deflate(url) {
                return self.connect_deflate_stream(url).await;
            }
            if let Some(tunnel) = super::proxy::connect_tunnel(url).await? {
                let fail_closed = tunnel.fail_closed();
                if fail_closed {
                    let request = self
                        .connect_request_for(url)
                        .map_err(|error| error.to_string())?;
                    return client_async_tls_with_config(request, tunnel.into_stream(), None, None)
                        .await
                        .map(|(stream, _)| ConnectedStream::Plain(Box::new(stream)))
                        .map_err(|error| error.to_string());
                }

                // A macOS HTTP proxy often fronts the same TUN route as the
                // system socket, but can leave long-lived WS tunnels half-open.
                // Prefer the system route. If it fails, reopen a fresh CONNECT
                // tunnel instead of reusing one that sat idle during the direct
                // handshake. Explicit proxy environment variables stay
                // fail-closed above.
                drop(tunnel);
                let request = self
                    .connect_request_for(url)
                    .map_err(|error| error.to_string())?;
                match tokio::time::timeout(MACOS_SYSTEM_ROUTE_WS_BUDGET, connect_async(request))
                    .await
                {
                    Ok(Ok((stream, _))) => {
                        return Ok(ConnectedStream::Plain(Box::new(stream)));
                    }
                    Ok(Err(error)) => debug!(
                        exchange = %self.config.exchange,
                        %error,
                        "system-route websocket handshake failed; retrying via macOS proxy"
                    ),
                    Err(_) => debug!(
                        exchange = %self.config.exchange,
                        timeout_ms = MACOS_SYSTEM_ROUTE_WS_BUDGET.as_millis() as u64,
                        "system-route websocket handshake timed out; retrying via macOS proxy"
                    ),
                }
                let tunnel = super::proxy::connect_tunnel(url)
                    .await?
                    .ok_or_else(|| "macOS websocket proxy became unavailable".to_owned())?;
                let request = self
                    .connect_request_for(url)
                    .map_err(|error| error.to_string())?;
                return client_async_tls_with_config(request, tunnel.into_stream(), None, None)
                    .await
                    .map(|(stream, _)| ConnectedStream::Plain(Box::new(stream)))
                    .map_err(|error| error.to_string());
            }
            let request = self
                .connect_request_for(url)
                .map_err(|error| error.to_string())?;
            connect_async(request)
                .await
                .map(|(stream, _)| ConnectedStream::Plain(Box::new(stream)))
                .map_err(|error| error.to_string())
        })
        .await
        .map_err(|_| {
            format!(
                "websocket handshake timed out after {}s",
                handshake_timeout.as_secs()
            )
        })??;
        connect_permit.mark_succeeded();
        Ok(stream)
    }

    async fn connect_deflate_stream(&self, raw_url: &str) -> Result<ConnectedStream, String> {
        let url = raw_url
            .parse()
            .map_err(|error: url::ParseError| error.to_string())?;
        let mut request = hpx_yawc::HttpRequest::builder();
        for &(name, value) in &self.connect_headers {
            request = request.header(name, value);
        }
        let options = DeflateOptions::default()
            .with_low_latency_compression()
            .with_no_delay()
            .with_utf8();
        WebSocket::connect(url)
            .with_options(options)
            .with_request(request)
            .await
            .map(|stream| ConnectedStream::Deflate(Box::new(stream)))
            .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    fn connect_request(
        &self,
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, Box<WsError>> {
        let connect_url = self.connect_url.load_full();
        self.connect_request_for(connect_url.as_str())
    }

    fn connect_request_for(
        &self,
        url: &str,
    ) -> Result<tokio_tungstenite::tungstenite::http::Request<()>, Box<WsError>> {
        let mut request = url.into_client_request().map_err(Box::new)?;
        for &(name, value) in &self.connect_headers {
            request.headers_mut().insert(
                name,
                tokio_tungstenite::tungstenite::http::HeaderValue::from_static(value),
            );
        }
        Ok(request)
    }

    async fn mark_connected(&self) {
        self.connection_problem.store(None);
        *self.state.write().await = WsState::Connected;
        let _ = self.tx.send(WsEvent::Connected);
        info!(exchange = %self.config.exchange, "ws connected");
    }

    async fn disconnect_session(&self, outcome: SessionOutcome) {
        let consecutive_failures = self
            .record_session_failure(outcome.uptime, &outcome.reason)
            .await;
        self.connection_problem
            .store(Some(Arc::new(outcome.reason.clone())));
        *self.state.write().await = WsState::Disconnected;
        let _ = self.tx.send(WsEvent::Disconnected(outcome.reason.clone()));
        warn!(
            exchange = %self.config.exchange,
            reason = %outcome.reason,
            uptime_ms = outcome.uptime.as_millis(),
            consecutive_failures,
            "ws disconnected"
        );
    }

    async fn mark_suspended(&self) {
        self.connection_problem.store(None);
        *self.state.write().await = WsState::Disconnected;
        let _ = self
            .tx
            .send(WsEvent::Disconnected("ws manager suspended".to_owned()));
    }

    async fn record_connect_error(&self, error: String) {
        let n = self.increment_failures().await;
        let connect_url = self.connect_url.load_full();
        let error = redacted_connection_error(&self.config.exchange, connect_url.as_str(), &error);
        self.connection_problem.store(Some(Arc::new(error.clone())));
        error!(
            exchange = %self.config.exchange,
            error = %error,
            consecutive_failures = n,
            "ws connect error"
        );
        let _ = self.tx.send(WsEvent::Disconnected(error));
    }

    async fn increment_failures(&self) -> u32 {
        let mut fails = self.consecutive_failures.write().await;
        *fails = fails.saturating_add(1);
        *fails
    }

    async fn record_session_failure(&self, uptime: Duration, reason: &str) -> u32 {
        let mut failures = self.consecutive_failures.write().await;
        *failures = if is_rate_limited_disconnect(reason) {
            (*failures).max(self.config.circuit_breaker_threshold)
        } else {
            next_session_failure_count(*failures, uptime)
        };
        *failures
    }

    async fn sleep_before_reconnect(&self) {
        if self.consume_priority_reconnect().await {
            return;
        }
        let n = *self.consecutive_failures.read().await;
        let delay = self.backoff(n.saturating_sub(1));
        debug!(exchange = %self.config.exchange, ?delay, "reconnect after backoff");
        if self.wait_for_activity(delay).await {
            let _ = self.consume_priority_reconnect().await;
        }
    }

    async fn wait_for_activity(&self, delay: Duration) -> bool {
        tokio::select! {
            () = sleep(delay) => false,
            () = self.activity_changed.notified() => true,
        }
    }

    async fn circuit_cooldown_interrupted(&self, cooldown: Duration) -> bool {
        self.wait_for_activity(cooldown).await
            && (self.consume_priority_reconnect().await || !self.is_active())
    }

    async fn consume_priority_reconnect(&self) -> bool {
        if !self
            .priority_reconnect_requested
            .swap(false, Ordering::AcqRel)
        {
            return false;
        }
        let mut failures = self.consecutive_failures.write().await;
        *failures = (*failures).min(self.config.circuit_breaker_threshold.saturating_sub(1));
        true
    }

    /// 维持单次会话：分流消息 + 心跳。返回断开原因。
    async fn run_session(&self, stream: ConnectedStream) -> String {
        let (sink, mut source) = stream.split();
        self.install_writer(sink).await;
        // Connected is an action boundary: subscribers immediately send their
        // venue subscription commands. Publish it only after `send()` can use
        // the installed writer, otherwise the first plan can be lost until a
        // later reconnect.
        self.mark_connected().await;

        if !self.is_active() {
            self.clear_writer().await;
            return "ws manager suspended".to_owned();
        }

        let mut heartbeat_task = self.spawn_heartbeat();
        let reason = self
            .wait_for_session_end(&mut source, &mut heartbeat_task)
            .await;

        heartbeat_task.abort();
        self.clear_writer().await;
        reason
    }

    async fn install_writer(&self, sink: ActiveWriter) {
        let mut guard = self.writer.lock().await;
        *guard = Some(sink);
    }

    async fn clear_writer(&self) {
        let mut guard = self.writer.lock().await;
        *guard = None;
    }

    fn spawn_heartbeat(&self) -> tokio::task::JoinHandle<()> {
        let heartbeat_interval = self.config.heartbeat_interval;
        let heartbeat = self.config.heartbeat.clone();
        let writer = Arc::clone(&self.writer);
        let exchange = self.config.exchange.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(heartbeat_interval);
            tick.tick().await; // 立即触发一次（吃掉首次）
            loop {
                tick.tick().await;
                let mut guard = writer.lock().await;
                if let Some(w) = guard.as_mut() {
                    if let Err(e) = w.send(heartbeat_message(&heartbeat)).await {
                        warn!(%exchange, error = %e, "heartbeat ping failed");
                        return;
                    }
                } else {
                    return;
                }
            }
        })
    }

    async fn wait_for_session_end(
        &self,
        source: &mut ActiveReader,
        heartbeat_task: &mut tokio::task::JoinHandle<()>,
    ) -> String {
        // 入站空闲看门狗：TCP 半开（NAT 超时/网络切换）时 ping 帧会先写进本地
        // 发送缓冲"成功"，要等内核重传耗尽（分钟级）才报错——期间 source.next()
        // 一直 pending、状态仍是 Connected，下游持续消费冻结行情。任何入站消息
        // （含 Pong）都刷新计时；超时即主动断线走重连路径。
        let idle_timeout = inbound_idle_timeout(self.config.heartbeat_interval);
        let mut last_inbound = Instant::now();
        let mut idle_check = tokio::time::interval(idle_check_period(idle_timeout));
        idle_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                msg = source.next_message() => {
                    if matches!(msg, Some(Ok(_))) {
                        last_inbound = Instant::now();
                    }
                    match self.handle_stream_message(
                        msg,
                        self.config.inbound_codec,
                        self.config.server_ping,
                    ).await {
                        SessionStep::Continue => {}
                        SessionStep::Break(reason) => break reason,
                    }
                },
                _ = idle_check.tick() => {
                    let idle = last_inbound.elapsed();
                    if idle >= idle_timeout {
                        break format!(
                            "inbound idle timeout: no messages for {}ms (limit {}ms)",
                            idle.as_millis(),
                            idle_timeout.as_millis()
                        );
                    }
                },
                _ = &mut *heartbeat_task => break "heartbeat task exited".to_owned(),
                _ = self.activity_changed.notified() => {
                    if !self.is_active() {
                        break "ws manager suspended".to_owned();
                    }
                },
            }
        }
    }

    async fn handle_stream_message(
        &self,
        msg: Option<Result<Message, String>>,
        inbound_codec: WsInboundCodec,
        server_ping: WsServerPing,
    ) -> SessionStep {
        match msg {
            Some(Ok(message)) => {
                self.handle_socket_message(message, inbound_codec, server_ping)
                    .await
            }
            Some(Err(error)) => SessionStep::Break(format!("ws error: {error}")),
            None => SessionStep::Break("stream ended".to_owned()),
        }
    }

    async fn handle_socket_message(
        &self,
        message: Message,
        inbound_codec: WsInboundCodec,
        server_ping: WsServerPing,
    ) -> SessionStep {
        match message {
            Message::Text(text) => {
                self.handle_text_message(text, server_ping).await;
                SessionStep::Continue
            }
            Message::Binary(payload) => {
                self.handle_binary_message(payload, inbound_codec, server_ping)
                    .await;
                SessionStep::Continue
            }
            Message::Ping(payload) => {
                self.send_best_effort(Message::Pong(payload)).await;
                SessionStep::Continue
            }
            Message::Pong(_) | Message::Frame(_) => SessionStep::Continue,
            Message::Close(frame) => SessionStep::Break(format!("server close: {frame:?}")),
        }
    }

    async fn handle_text_message(&self, text: String, server_ping: WsServerPing) {
        self.ingest.text_messages.fetch_add(1, Ordering::Relaxed);
        self.ingest
            .text_bytes
            .fetch_add(text.len() as u64, Ordering::Relaxed);
        match server_pong(&text, server_ping) {
            Ok(Some(pong)) => self.send_best_effort(pong).await,
            Ok(None) => {
                let _ = self.tx.send(WsEvent::Text(text));
            }
            Err(error) => {
                self.ingest
                    .ping_probe_failures
                    .fetch_add(1, Ordering::Relaxed);
                self.ingest.record_failure("ping_probe_decode", &error);
                warn!(
                    exchange = %self.config.exchange,
                    error = %error,
                    "ws server ping probe decode failed"
                );
                let _ = self.tx.send(WsEvent::Text(text));
            }
        }
    }

    async fn handle_binary_message(
        &self,
        payload: Vec<u8>,
        inbound_codec: WsInboundCodec,
        server_ping: WsServerPing,
    ) {
        self.ingest.binary_messages.fetch_add(1, Ordering::Relaxed);
        self.ingest
            .binary_bytes
            .fetch_add(payload.len() as u64, Ordering::Relaxed);
        match decode_binary(&payload, inbound_codec) {
            Ok(Some(text)) => self.handle_text_message(text, server_ping).await,
            Ok(None) => {
                let _ = self.tx.send(WsEvent::Binary(payload));
            }
            Err(error) => {
                self.ingest.decode_failures.fetch_add(1, Ordering::Relaxed);
                self.ingest.record_failure("binary_decode", &error);
                warn!(exchange = %self.config.exchange, error = %error, "ws binary decode failed");
            }
        }
    }

    fn backoff(&self, attempt: u32) -> Duration {
        let initial = self.config.initial_reconnect_delay.as_millis() as u64;
        let max = self.config.max_reconnect_delay.as_millis() as u64;
        // 1s -> 2s -> 4s -> ... 含 ±25% 抖动
        let raw = initial.saturating_mul(2u64.saturating_pow(attempt.min(10)));
        let capped = raw.min(max);
        let jitter = (capped / 4).max(1);
        let lower = capped.saturating_sub(jitter);
        let upper = capped.saturating_add(jitter).min(max);
        let final_ms = rand::thread_rng().gen_range(lower..=upper);
        Duration::from_millis(final_ms)
    }

    async fn send_best_effort(&self, msg: Message) {
        let mut guard = self.writer.lock().await;
        if let Some(w) = guard.as_mut() {
            let _ = w.send(msg).await;
        }
    }
}

fn message_to_deflate_frame(message: Message) -> Result<DeflateFrame, String> {
    match message {
        Message::Text(text) => Ok(DeflateFrame::text(text)),
        Message::Binary(payload) => Ok(DeflateFrame::binary(payload)),
        Message::Ping(payload) => Ok(DeflateFrame::ping(payload)),
        Message::Pong(payload) => Ok(DeflateFrame::pong(payload)),
        Message::Close(frame) => {
            let (code, reason) = frame
                .map(|frame| {
                    (
                        hpx_yawc::close::CloseCode::from(u16::from(frame.code)),
                        frame.reason.into_owned(),
                    )
                })
                .unwrap_or((hpx_yawc::close::CloseCode::Normal, String::new()));
            Ok(DeflateFrame::close(code, reason))
        }
        Message::Frame(_) => Err("raw websocket frames are unsupported".to_owned()),
    }
}

fn deflate_frame_to_message(frame: DeflateFrame) -> Result<Message, String> {
    let (opcode, _, payload) = frame.into_parts();
    match opcode {
        DeflateOpCode::Text => String::from_utf8(payload.to_vec())
            .map(Message::Text)
            .map_err(|error| error.to_string()),
        DeflateOpCode::Binary | DeflateOpCode::Continuation => {
            Ok(Message::Binary(payload.to_vec()))
        }
        // hpx-yawc already answers incoming ping frames. Treat them as pong
        // observations here so the shared manager does not emit a duplicate pong.
        DeflateOpCode::Ping | DeflateOpCode::Pong => Ok(Message::Pong(payload.to_vec())),
        DeflateOpCode::Close => Ok(Message::Close(None)),
    }
}

/// Only hosts proven by a live RFC 7692 handshake and payload-byte comparison
/// use the compression-capable transport. The server remains authoritative: if
/// it declines the extension, hpx-yawc keeps the same uncompressed WS semantics.
fn should_offer_permessage_deflate(raw_url: &str) -> bool {
    let Ok(url) = url::Url::parse(raw_url) else {
        return false;
    };
    if url.scheme() != "wss" {
        return false;
    }
    matches!(
        url.host_str(),
        Some(
            "fstream.binance.com"
                | "stream.binance.com"
                | "ws.okx.com"
                | "wspap.okx.com"
                | "stream.bybit.com"
                | "stream-testnet.bybit.com"
        )
    )
}

fn diagnostic_ws_url(exchange: &str, raw_url: &str) -> String {
    let Ok(mut url) = url::Url::parse(raw_url) else {
        return "invalid-websocket-url".to_owned();
    };
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    if exchange.to_ascii_lowercase().contains("private") {
        url.set_path("/redacted");
    }
    url.to_string()
}

fn redacted_connection_error(exchange: &str, raw_url: &str, error: &str) -> String {
    error.replace(raw_url, &diagnostic_ws_url(exchange, raw_url))
}

fn next_session_failure_count(previous: u32, uptime: Duration) -> u32 {
    if uptime >= STABLE_SESSION_MIN_UPTIME {
        1
    } else {
        previous.saturating_add(1)
    }
}

fn is_rate_limited_disconnect(reason: &str) -> bool {
    let reason = reason.to_ascii_lowercase();
    reason.contains("rate limit")
        || reason.contains("too many request")
        || reason.contains("maximum capacity for websocket")
        || reason.contains("exceeded msg rate")
}

fn heartbeat_message(heartbeat: &WsHeartbeat) -> Message {
    match heartbeat {
        WsHeartbeat::PingFrame => Message::Ping(Vec::new()),
        WsHeartbeat::Text(value) => Message::Text(value.clone()),
    }
}

fn decode_binary(payload: &[u8], codec: WsInboundCodec) -> anyhow::Result<Option<String>> {
    match codec {
        WsInboundCodec::Plain => Ok(None),
        WsInboundCodec::Utf8Text => Ok(Some(String::from_utf8(payload.to_vec())?)),
        WsInboundCodec::GzipText => {
            let mut decoder = GzDecoder::new(payload);
            let mut text = String::new();
            decoder.read_to_string(&mut text)?;
            Ok(Some(text))
        }
    }
}

fn server_pong(text: &str, ping: WsServerPing) -> anyhow::Result<Option<Message>> {
    let _ = text;
    match ping {
        WsServerPing::None => Ok(None),
    }
}

/// 熔断冷却时长：重连退避上限的 3 倍，保证半开试探频率远低于普通重连。
fn circuit_cooldown(max_reconnect_delay: Duration) -> Duration {
    max_reconnect_delay.saturating_mul(3)
}

/// 入站空闲上限：心跳间隔的 3 倍（至少 10s），未收到任何消息即判定半开挂死。
fn inbound_idle_timeout(heartbeat_interval: Duration) -> Duration {
    heartbeat_interval
        .saturating_mul(3)
        .max(Duration::from_secs(10))
}

fn ws_handshake_timeout(raw_url: &str) -> Duration {
    let is_kraken = url::Url::parse(raw_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .is_some_and(|host| matches!(host.as_str(), "ws.kraken.com" | "futures.kraken.com"));
    if is_kraken {
        KRAKEN_WS_HANDSHAKE_TIMEOUT
    } else {
        DEFAULT_WS_HANDSHAKE_TIMEOUT
    }
}

fn idle_check_period(idle_timeout: Duration) -> Duration {
    (idle_timeout / 4).max(Duration::from_secs(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    fn config() -> WsConfig {
        WsConfig {
            url: "ws://localhost".into(),
            exchange: "test".into(),
            heartbeat_interval: Duration::from_secs(30),
            heartbeat: WsHeartbeat::PingFrame,
            inbound_codec: WsInboundCodec::Plain,
            server_ping: WsServerPing::None,
            initial_reconnect_delay: Duration::from_secs(1),
            max_reconnect_delay: Duration::from_secs(60),
            circuit_breaker_threshold: 5,
        }
    }

    #[test]
    fn circuit_cooldown_and_idle_watchdog_have_sane_bounds() {
        assert_eq!(
            circuit_cooldown(Duration::from_secs(60)),
            Duration::from_secs(180)
        );
        // 空闲上限跟随心跳间隔，且有 10s 下限兜底极短心跳配置。
        assert_eq!(
            inbound_idle_timeout(Duration::from_secs(30)),
            Duration::from_secs(90)
        );
        assert_eq!(
            inbound_idle_timeout(Duration::from_secs(1)),
            Duration::from_secs(10)
        );
        assert_eq!(
            idle_check_period(Duration::from_secs(90)),
            Duration::from_millis(22_500)
        );
        assert_eq!(
            idle_check_period(Duration::from_secs(2)),
            Duration::from_secs(1)
        );
    }

    #[tokio::test]
    async fn first_dispatcher_receives_connected_published_during_construction() {
        let manager = WsManager::new(config());
        manager
            .tx
            .send(WsEvent::Connected)
            .expect("bootstrap receiver keeps the event alive");

        let mut receiver = manager.subscribe();
        let event = tokio::time::timeout(Duration::from_millis(50), receiver.recv())
            .await
            .expect("connected event timeout")
            .expect("connected event");

        assert!(matches!(event, WsEvent::Connected));
    }

    #[test]
    fn only_kraken_receives_the_proxy_tolerant_handshake_timeout() {
        assert!(MACOS_SYSTEM_ROUTE_WS_BUDGET < KRAKEN_WS_HANDSHAKE_TIMEOUT);
        assert_eq!(
            ws_handshake_timeout("wss://ws.kraken.com/v2"),
            Duration::from_secs(30)
        );
        assert_eq!(
            ws_handshake_timeout("wss://futures.kraken.com/ws/v1"),
            Duration::from_secs(30)
        );
        assert_eq!(
            ws_handshake_timeout("wss://stream.binance.com/ws"),
            Duration::from_secs(12)
        );
        assert_eq!(ws_handshake_timeout("not-a-url"), Duration::from_secs(12));
    }

    #[tokio::test]
    async fn priority_reconnect_interrupts_backoff_once_and_closes_the_old_problem() {
        let manager = WsManager::new(config());
        *manager.consecutive_failures.write().await = manager.config.circuit_breaker_threshold;
        manager
            .connection_problem
            .store(Some(Arc::new("old handshake timeout".to_owned())));

        manager.prioritize_reconnect();

        assert!(manager.connection_problem().is_none());
        assert!(manager.consume_priority_reconnect().await);
        assert_eq!(
            *manager.consecutive_failures.read().await,
            manager.config.circuit_breaker_threshold - 1
        );
        manager.prioritize_reconnect();
        assert!(!manager.consume_priority_reconnect().await);
    }

    #[test]
    fn backoff_grows_then_caps() {
        let mgr = WsManager::new(config());
        let d0 = mgr.backoff(0).as_millis();
        let d1 = mgr.backoff(1).as_millis();
        let d2 = mgr.backoff(2).as_millis();
        let d_max = mgr.backoff(20).as_millis();
        assert!((750..=1250).contains(&d0));
        assert!((1500..=2500).contains(&d1));
        assert!((3000..=5000).contains(&d2));
        assert!((45_000..=60_000).contains(&d_max));
    }

    #[tokio::test]
    async fn demand_scopes_suspend_only_after_the_last_public_feed_releases() {
        let manager = WsManager::new(config()).with_demand_control();
        assert!(!manager.is_active());

        manager.activate_scope("spot-ticker");
        manager.activate_scope("spot-book");
        manager.suspend_scope("spot-ticker").await;
        assert!(manager.is_active());

        manager.suspend_scope("spot-book").await;
        assert!(!manager.is_active());

        manager.activate_scope("spot-ticker");
        assert!(manager.is_active());
    }

    #[test]
    fn priority_connect_lane_is_explicit_opt_in() {
        assert!(!WsManager::new(config()).priority_connect);
        assert!(
            WsManager::new(config())
                .with_priority_connect()
                .priority_connect
        );
    }

    #[test]
    fn short_sessions_accumulate_failures_until_a_stable_session_resets_the_streak() {
        assert_eq!(next_session_failure_count(3, Duration::from_secs(1)), 4);
        assert_eq!(next_session_failure_count(4, STABLE_SESSION_MIN_UPTIME), 1);
    }

    #[test]
    fn explicit_ws_rate_limits_are_distinct_from_ordinary_disconnects() {
        assert!(is_rate_limited_disconnect(
            "server close: Maximum websockets rate limit exceeded for this user"
        ));
        assert!(is_rate_limited_disconnect(
            r#"{"Error":"Exceeded msg rate"}"#
        ));
        assert!(!is_rate_limited_disconnect("server close: maintenance"));
    }

    #[test]
    fn private_ws_diagnostics_redact_path_credentials_and_query_values() {
        let raw = "wss://user:pass@fstream.binance.com/private/ws/listen-secret?token=query-secret#fragment";
        let redacted = diagnostic_ws_url("binance-private", raw);

        assert_eq!(redacted, "wss://fstream.binance.com/redacted");
        assert!(!redacted.contains("secret"));
        let debug = format!(
            "{:?}",
            WsConfig {
                url: raw.to_owned(),
                exchange: "binance-private".to_owned(),
                ..config()
            }
        );
        assert!(!debug.contains("listen-secret"));
        assert!(!debug.contains("query-secret"));
        assert!(!debug.contains("user:pass"));
    }

    #[test]
    fn public_ws_diagnostics_keep_endpoint_path_but_drop_query_and_fragment() {
        assert_eq!(
            diagnostic_ws_url(
                "binance",
                "wss://stream.binance.com:9443/stream?streams=btcusdt#book"
            ),
            "wss://stream.binance.com:9443/stream"
        );
    }

    #[test]
    fn deflate_offer_is_limited_to_live_proven_official_hosts() {
        for url in [
            "wss://fstream.binance.com/market/ws",
            "wss://stream.binance.com:9443/ws",
            "wss://ws.okx.com:8443/ws/v5/public",
            "wss://wspap.okx.com:8443/ws/v5/private",
            "wss://stream.bybit.com/v5/public/linear",
            "wss://stream-testnet.bybit.com/v5/private",
        ] {
            assert!(should_offer_permessage_deflate(url), "url: {url}");
        }
        for url in [
            "wss://ws.bitget.com/v3/ws/public",
            "wss://fx-ws.gateio.ws/v4/ws/usdt/sbe",
            "wss://ws-api-futures.kucoin.com/",
            "wss://fstream.binance.com.evil.test/ws",
            "ws://fstream.binance.com/ws",
        ] {
            assert!(!should_offer_permessage_deflate(url), "url: {url}");
        }
    }

    #[test]
    fn manager_installs_a_process_rustls_provider_before_connecting() {
        let _manager = WsManager::new(config());
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }

    #[test]
    fn deflate_transport_preserves_application_message_types() {
        for message in [
            Message::Text("subscribe".to_owned()),
            Message::Binary(vec![1, 2, 3]),
            Message::Ping(vec![4]),
            Message::Pong(vec![5]),
        ] {
            let frame = message_to_deflate_frame(message.clone()).expect("supported message");
            let decoded = deflate_frame_to_message(frame).expect("decoded message");
            match message {
                Message::Ping(payload) => assert_eq!(decoded, Message::Pong(payload)),
                _ => assert_eq!(decoded, message),
            }
        }
    }

    #[test]
    fn custom_connect_header_is_bound_to_websocket_handshake() {
        let mgr = WsManager::new(config()).with_connect_header("x-gate-size-decimal", "1");
        let request = mgr.connect_request().expect("websocket request");

        assert_eq!(
            request
                .headers()
                .get("x-gate-size-decimal")
                .and_then(|value| value.to_str().ok()),
            Some("1")
        );
    }

    #[test]
    fn refreshed_connect_url_is_used_by_the_next_handshake() {
        let mgr = WsManager::new(config());
        mgr.replace_connect_url("wss://example.test/refreshed?token=secret".into())
            .expect("valid refresh URL");

        let request = mgr.connect_request().expect("websocket request");
        assert_eq!(
            request.uri().to_string(),
            "wss://example.test/refreshed?token=secret"
        );
        assert_eq!(mgr.config().url, "ws://localhost");
    }

    #[tokio::test]
    async fn initial_state_is_disconnected() {
        let mgr = WsManager::new(config());
        assert_eq!(mgr.state().await, WsState::Disconnected);
        assert!(!mgr.is_connected().await);
    }

    #[tokio::test]
    async fn connected_event_is_published_only_after_the_writer_is_ready() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("test websocket listener");
        let address = listener.local_addr().expect("listener address");
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("client connects");
            let mut socket = tokio_tungstenite::accept_async(stream)
                .await
                .expect("websocket handshake");
            tokio::time::timeout(Duration::from_secs(2), socket.next())
                .await
                .expect("subscription arrives before timeout")
                .expect("subscription frame exists")
                .expect("subscription frame decodes")
        });
        let manager = Arc::new(WsManager::new(WsConfig {
            url: format!("ws://{address}"),
            ..config()
        }));
        let mut events = manager.subscribe();
        let runner = tokio::spawn(Arc::clone(&manager).run());

        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(events.recv().await, Ok(WsEvent::Connected)) {
                    break;
                }
            }
        })
        .await
        .expect("connected event");
        manager
            .send_text("subscribe-now".to_owned())
            .await
            .expect("writer is ready at the connected boundary");

        let message = server.await.expect("test server joins");
        assert_eq!(message, Message::Text("subscribe-now".to_owned()));
        runner.abort();
    }

    #[tokio::test]
    async fn send_before_connect_errors() {
        let mgr = WsManager::new(config());
        let result = mgr.send(Message::Text("hi".to_owned())).await;
        assert!(matches!(result, Err(ExchangeError::WsClosed(_))));
    }

    #[test]
    fn heartbeat_can_be_text_payload() {
        assert_eq!(
            heartbeat_message(&WsHeartbeat::Text("ping".into())),
            Message::Text("ping".to_owned())
        );
    }

    #[tokio::test]
    async fn ingest_stats_track_messages_and_failures() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mgr = WsManager::new(WsConfig {
            exchange: "ingest-stats-test".into(),
            url: "ws://ingest-stats-test".into(),
            inbound_codec: WsInboundCodec::GzipText,
            ..config()
        });
        assert_eq!(mgr.ingest_stats(), WsIngestStats::default());

        mgr.handle_binary_message(
            b"not-gzip".to_vec(),
            WsInboundCodec::GzipText,
            WsServerPing::None,
        )
        .await;
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(br#"{"status":"ok"}"#).unwrap();
        mgr.handle_binary_message(
            encoder.finish().unwrap(),
            WsInboundCodec::GzipText,
            WsServerPing::None,
        )
        .await;

        mgr.send_text("subscribe-while-disconnected".to_owned())
            .await
            .expect_err("send without connection must fail");

        let stats = mgr.ingest_stats();
        assert_eq!(stats.text_messages, 1);
        assert_eq!(stats.text_bytes, r#"{"status":"ok"}"#.len() as u64);
        assert_eq!(stats.binary_messages, 2);
        assert!(stats.binary_bytes > 0);
        assert_eq!(stats.decode_failures, 1);
        assert_eq!(stats.ping_probe_failures, 0);
        assert_eq!(stats.send_failures, 1);
        let last = stats
            .last_failure
            .as_deref()
            .expect("last failure recorded");
        assert!(last.starts_with("send:"), "last: {last}");
        assert!(last.contains("not connected"), "last: {last}");

        let snapshots = ingest_snapshots();
        let row = snapshots
            .iter()
            .find(|row| row.exchange == "ingest-stats-test")
            .expect("registry row for live manager");
        assert_eq!(row.stats, stats);

        drop(mgr);
        assert!(!ingest_snapshots()
            .iter()
            .any(|row| row.exchange == "ingest-stats-test"));
    }

    #[tokio::test]
    async fn current_connection_problem_clears_after_recovery() {
        let manager = WsManager::new(config());

        manager
            .record_connect_error("tls handshake eof".to_owned())
            .await;
        assert_eq!(
            manager.connection_problem().as_deref(),
            Some("tls handshake eof")
        );

        manager.mark_connected().await;
        assert!(manager.connection_problem().is_none());
    }

    #[test]
    fn ingest_registry_preserves_managers_with_the_same_exchange_and_url() {
        let first = WsManager::new(WsConfig {
            exchange: "duplicate-registry-test".into(),
            url: "ws://duplicate-registry-test".into(),
            ..config()
        });
        let second = WsManager::new(WsConfig {
            exchange: "duplicate-registry-test".into(),
            url: "ws://duplicate-registry-test".into(),
            ..config()
        });

        let rows = ingest_snapshots()
            .into_iter()
            .filter(|row| row.exchange == "duplicate-registry-test")
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 2);
        assert_ne!(rows[0].instance_id, rows[1].instance_id);

        drop((first, second));
        assert!(!ingest_snapshots()
            .iter()
            .any(|row| row.exchange == "duplicate-registry-test"));
    }

    #[test]
    fn gzip_binary_decodes_to_text() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(br#"{"ping":1}"#).unwrap();
        let payload = encoder.finish().unwrap();
        assert_eq!(
            decode_binary(&payload, WsInboundCodec::GzipText).unwrap(),
            Some(r#"{"ping":1}"#.into())
        );
        assert_eq!(
            decode_binary(&payload, WsInboundCodec::Plain).unwrap(),
            None
        );
    }

    #[test]
    fn utf8_binary_decodes_to_text_and_rejects_invalid_bytes() {
        assert_eq!(
            decode_binary(br#"{"T":"funding-fee"}"#, WsInboundCodec::Utf8Text).unwrap(),
            Some(r#"{"T":"funding-fee"}"#.into())
        );
        assert!(decode_binary(&[0xff, 0xfe], WsInboundCodec::Utf8Text).is_err());
    }
}
