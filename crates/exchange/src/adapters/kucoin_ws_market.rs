//! KuCoin Futures public WebSocket order book subscriber.
//!
//! Official docs:
//! - Public token: <https://www.kucoin.com/docs-new/websocket-api/base-info/get-public-token-futures>
//! - Futures public channels: <https://www.kucoin.com/docs-new/websocket-api/base-info/futures-public-channels>

use super::kucoin_ws_session::{
    fetch_public_session, spawn_public_session_refresh, PublicSession, PublicSessionKind,
};
use crate::adapter::strip_common_suffixes;
use crate::error::ExchangeResult;
use crate::http::HttpClient;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use shared_types::OrderBookInfo;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::OnceCell;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "kucoin";
const TOPIC_PREFIX: &str = "/contractMarket/level2Depth50";
const DEPTH_LEVEL: u32 = 50;
const BOOK_STALE_AFTER_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedBook {
    book: OrderBookInfo,
    sequence: i64,
    observed_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct MarketStream {
    stream: OnceCell<Arc<MarketStreamInner>>,
}

#[derive(Debug)]
struct MarketStreamInner {
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl MarketStream {
    pub(crate) fn new(http: HttpClient) -> Arc<Self> {
        let stream = Arc::new(Self {
            stream: OnceCell::new(),
        });
        let bootstrap = Arc::clone(&stream);
        tokio::spawn(async move {
            if let Err(error) = bootstrap.init(http).await {
                warn!(error = %error, "kucoin market ws bootstrap failed");
            }
        });
        stream
    }

    pub(crate) fn latest(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        self.stream
            .get()
            .and_then(|stream| stream.latest(symbol, depth))
    }

    pub(crate) fn touch(&self, symbol: &str) {
        if let Some(stream) = self.stream.get() {
            stream.touch(symbol);
        }
    }

    async fn init(&self, http: HttpClient) -> ExchangeResult<()> {
        let session = fetch_public_session(&http).await?;
        let inner = MarketStreamInner::new(session, http);
        let _ = self.stream.set(Arc::clone(&inner));
        Ok(())
    }
}

impl MarketStreamInner {
    fn new(session: PublicSession, http: HttpClient) -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: session.ws_url,
                exchange: EXCHANGE.into(),
                heartbeat_interval: Duration::from_millis(session.ping_interval_ms),
                heartbeat: WsHeartbeat::Text(r#"{"id":"ping","type":"ping"}"#.into()),
                inbound_codec: WsInboundCodec::Plain,
                server_ping: WsServerPing::None,
                initial_reconnect_delay: Duration::from_secs(1),
                max_reconnect_delay: Duration::from_secs(30),
                circuit_breaker_threshold: 10,
            })
            .with_demand_control(),
        );
        spawn_public_session_refresh(http, Arc::clone(&manager), PublicSessionKind::Futures);
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "kucoin market ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());

        stream
    }

    fn latest(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        let entry = self.books.get(&stream_symbol(symbol))?;
        (now_ms().saturating_sub(entry.observed_at_ms) <= BOOK_STALE_AFTER_MS)
            .then(|| truncate_book(&entry.book, depth))
    }

    fn touch(&self, symbol: &str) {
        self.manager.activate_scope("perp-book");
        let symbol = stream_symbol(symbol);
        let now = now_ms();
        let mut needs_subscribe = false;
        self.subscriptions
            .entry(symbol.clone())
            .and_modify(|state| state.last_touched_ms = now)
            .or_insert_with(|| {
                needs_subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: false,
                }
            });
        if needs_subscribe {
            self.spawn_subscribe(symbol);
        }
    }

    fn spawn_subscribe(&self, symbol: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            let deadline = std::time::Instant::now() + SUBSCRIBE_CONNECT_WAIT;
            while std::time::Instant::now() < deadline {
                if manager.is_connected().await {
                    break;
                }
                tokio::time::sleep(SUBSCRIBE_POLL_INTERVAL).await;
            }
            if !manager.is_connected().await {
                debug!(
                    symbol = %symbol,
                    "kucoin market ws not connected yet; subscribe will resume on next Connected event"
                );
                return;
            }
            match manager
                .send(Message::Text(subscribe_payload(&symbol)))
                .await
            {
                Ok(()) => {
                    if let Some(mut state) = subscriptions.get_mut(&symbol) {
                        state.sent_on_current_connection = true;
                    }
                }
                Err(error) => {
                    warn!(symbol = %symbol, error = %error, "kucoin market ws subscribe failed");
                }
            }
        });
    }

    async fn run_dispatch(self: Arc<Self>) {
        let mut rx = self.manager.subscribe();
        loop {
            if !self.handle_dispatch_result(rx.recv().await) {
                return;
            }
        }
    }

    fn handle_dispatch_result(&self, event: Result<WsEvent, RecvError>) -> bool {
        match event {
            Ok(event) => {
                self.handle_ws_event(event);
                true
            }
            Err(RecvError::Lagged(missed)) => {
                warn!(missed, "kucoin market ws broadcast receiver lagged");
                self.books.clear();
                true
            }
            Err(RecvError::Closed) => false,
        }
    }

    fn handle_ws_event(&self, event: WsEvent) {
        match event {
            WsEvent::Text(text) => self.on_text(&text),
            WsEvent::Connected => self.on_connected(),
            WsEvent::Disconnected(reason) => self.on_disconnected(&reason),
            WsEvent::CircuitOpened => {
                self.books.clear();
                warn!("kucoin market ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let symbols: Vec<String> = self
            .subscriptions
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for symbol in symbols {
            if let Some(mut state) = self.subscriptions.get_mut(&symbol) {
                state.sent_on_current_connection = false;
            }
            self.spawn_subscribe(symbol);
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "kucoin market ws disconnected");
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_text(&self, text: &str) {
        let Some(parsed) = parse_depth_snapshot(text) else {
            return;
        };
        if self
            .books
            .get(&parsed.stream_symbol)
            .is_some_and(|cached| parsed.sequence <= cached.sequence)
        {
            return;
        }
        self.books.insert(
            parsed.stream_symbol.clone(),
            CachedBook {
                book: parsed.book,
                sequence: parsed.sequence,
                observed_at_ms: now_ms(),
            },
        );
    }

    async fn run_cleaner(self: Arc<Self>) {
        let mut tick = tokio::time::interval(CLEAN_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            self.prune_idle().await;
        }
    }

    async fn prune_idle(&self) {
        let pruned = self.take_idle_symbols();
        self.remove_pruned_books(&pruned);
        self.unsubscribe_pruned(&pruned).await;
        self.log_pruned(&pruned);
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("perp-book").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("perp-book");
            }
        }
    }

    fn remove_pruned_books(&self, symbols: &[String]) {
        for symbol in symbols {
            self.books.remove(symbol);
        }
    }

    async fn unsubscribe_pruned(&self, symbols: &[String]) {
        for symbol in symbols {
            self.unsubscribe_symbol(symbol).await;
        }
    }

    async fn unsubscribe_symbol(&self, symbol: &str) {
        if let Err(error) = self
            .manager
            .send(Message::Text(unsubscribe_payload(symbol)))
            .await
        {
            debug!(symbol = %symbol, error = %error, "kucoin market ws unsubscribe failed");
        }
    }

    fn log_pruned(&self, pruned: &[String]) {
        if pruned.is_empty() {
            return;
        }
        info!(
            pruned = pruned.len(),
            remaining = self.subscriptions.len(),
            "kucoin market ws idle prune"
        );
    }

    fn take_idle_symbols(&self) -> Vec<String> {
        let cutoff = now_ms().saturating_sub(SUBSCRIPTION_IDLE_TTL_MS);
        let mut pruned = Vec::new();
        self.subscriptions.retain(|symbol, state| {
            if state.last_touched_ms < cutoff {
                pruned.push(symbol.clone());
                false
            } else {
                true
            }
        });
        pruned
    }
}

// PublicSession / fetch_public_session live in `kucoin_ws_session` so both
// the depth (`kucoin_ws_market`) and ticker (`kucoin_ws_ticker`) consumers
// share the same bullet-public handshake.

fn stream_symbol(symbol: &str) -> String {
    normalized_to_kucoin(symbol)
}

pub(super) fn normalized_to_kucoin(symbol: &str) -> String {
    let upper = symbol.to_ascii_uppercase();
    if upper.ends_with("USDTM") {
        return upper;
    }
    let base = strip_common_suffixes(&upper);
    let mapped = if base == "BTC" {
        "XBT".to_owned()
    } else {
        base
    };
    format!("{mapped}USDTM")
}

pub(super) fn kucoin_to_normalized(symbol: &str) -> String {
    let upper = symbol.to_ascii_uppercase();
    let stripped = upper
        .strip_suffix("USDTM")
        .or_else(|| upper.strip_suffix("USDM"))
        .unwrap_or(&upper);
    if stripped == "XBT" {
        "BTC".into()
    } else {
        stripped.into()
    }
}

fn subscribe_payload(symbol: &str) -> String {
    command_payload("subscribe", symbol)
}

fn unsubscribe_payload(symbol: &str) -> String {
    command_payload("unsubscribe", symbol)
}

fn command_payload(kind: &str, symbol: &str) -> String {
    json!({
        "id": format!("depth-{}-{}", stream_symbol(symbol), now_ms()),
        "type": kind,
        "topic": depth_topic(symbol),
        "privateChannel": false,
        "response": true
    })
    .to_string()
}

fn depth_topic(symbol: &str) -> String {
    format!("{TOPIC_PREFIX}:{}", stream_symbol(symbol))
}

#[derive(Debug)]
struct ParsedDepthSnapshot {
    stream_symbol: String,
    book: OrderBookInfo,
    sequence: i64,
}

fn parse_depth_snapshot(text: &str) -> Option<ParsedDepthSnapshot> {
    let envelope: DepthEnvelope = serde_json::from_str(text).ok()?;
    if !envelope.topic.starts_with(TOPIC_PREFIX) {
        return None;
    }
    let data = envelope.data?;
    let stream_symbol = envelope.topic.split_once(':')?.1.to_owned();
    let bids = parse_levels(&data.bids);
    let asks = parse_levels(&data.asks);
    if bids.is_empty() && asks.is_empty() {
        return None;
    }
    let sequence = data.sequence.or(envelope.sequence).unwrap_or_default();
    if sequence <= 0 {
        return None;
    }
    let book = OrderBookInfo {
        symbol: kucoin_to_normalized(&stream_symbol),
        exchange: EXCHANGE.into(),
        bids,
        asks,
        timestamp: data.timestamp.unwrap_or_else(now_ms),
    };
    Some(ParsedDepthSnapshot {
        stream_symbol,
        book,
        sequence,
    })
}

fn parse_levels(levels: &[[serde_json::Value; 2]]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|[price, size]| Some([value_f64(price)?, value_f64(size)?]))
        .filter(|[price, size]| *price > 0.0 && *size >= 0.0)
        .collect()
}

fn value_f64(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn truncate_book(book: &OrderBookInfo, depth: u32) -> OrderBookInfo {
    let depth = depth.clamp(1, DEPTH_LEVEL) as usize;
    let mut book = book.clone();
    book.bids.truncate(depth);
    book.asks.truncate(depth);
    book
}

#[derive(Debug, Deserialize)]
struct DepthEnvelope {
    #[serde(default)]
    topic: String,
    #[serde(default)]
    data: Option<DepthData>,
    #[serde(default, rename = "sn")]
    sequence: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct DepthData {
    #[serde(default)]
    bids: Vec<[serde_json::Value; 2]>,
    #[serde(default)]
    asks: Vec<[serde_json::Value; 2]>,
    #[serde(default)]
    timestamp: Option<i64>,
    #[serde(default)]
    sequence: Option<i64>,
}

#[cfg(test)]
#[path = "kucoin_ws_market_tests.rs"]
mod tests;
