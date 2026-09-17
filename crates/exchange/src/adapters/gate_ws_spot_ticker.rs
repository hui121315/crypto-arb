//! Gate Spot public WebSocket ticker subscriber.
//!
//! Gate Spot `spot.tickers` carries last price, quote volume and best bid / ask.
//! Discovery therefore uses one bounded channel and leaves sizes unknown; the
//! separate order-book channel is activated only by an opportunity preview.
//!
//! Official docs:
//! - Spot WS base URL: <https://www.gate.com/docs/developers/apiv4/ws/en/>
//! - `spot.tickers`: <https://www.gate.com/docs/developers/apiv4/ws/en/#tickers-channel>

use super::gate_config::GateConfig;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::{now_ms, now_secs};
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use shared_types::{OrderBookInfo, SpotTick};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "gate";
const WS_URL: &str = "wss://api.gateio.ws/ws/v4/";
const TICKERS_CHANNEL: &str = "spot.tickers";
const ORDER_BOOK_CHANNEL: &str = "spot.order_book";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const DEPTH_SUBSCRIPTION_IDLE_TTL_MS: i64 = 60_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const EVENT_BUFFER_CAPACITY: usize = 8_192;
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

static SHARED_STREAM: OnceLock<Arc<SpotTickerStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedTicker {
    item: SpotTickerUpdate,
    cached_at_ms: i64,
    data_timestamp_ms: i64,
}

#[derive(Debug)]
pub(crate) struct SpotTickerStream {
    manager: Arc<WsManager>,
    tickers: Arc<DashMap<String, CachedTicker>>,
    depth_books: Arc<DashMap<String, CachedDepthBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    depth_subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

#[derive(Debug, Clone)]
struct CachedDepthBook {
    book: OrderBookInfo,
    cached_at_ms: i64,
}

pub(crate) fn snapshot_spot_ticks(
    config: &GateConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<SpotTick>> {
    let symbols = symbols.and_then(stream_symbols)?;
    let stream = enabled_stream(config)?;
    stream.touch_many(&symbols);
    let rows = stream.snapshot(&symbols);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn spot_connection_problem(config: &GateConfig) -> Option<String> {
    enabled_stream(config)?.manager.connection_problem()
}

pub(crate) fn latest_spot_orderbook(
    config: &GateConfig,
    symbol: &str,
    depth: u32,
) -> Option<OrderBookInfo> {
    let symbol = stream_symbol(symbol)?;
    let stream = enabled_stream(config)?;
    stream.touch_depth(&symbol);
    stream.latest_depth(&symbol, depth)
}

fn enabled_stream(config: &GateConfig) -> Option<Arc<SpotTickerStream>> {
    if config.testnet || config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(SpotTickerStream::new)))
}

impl SpotTickerStream {
    fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new_with_event_capacity(
                WsConfig {
                    url: WS_URL.into(),
                    exchange: EXCHANGE.into(),
                    heartbeat_interval: Duration::from_secs(20),
                    heartbeat: WsHeartbeat::PingFrame,
                    inbound_codec: WsInboundCodec::Plain,
                    server_ping: WsServerPing::None,
                    initial_reconnect_delay: Duration::from_secs(1),
                    max_reconnect_delay: Duration::from_secs(30),
                    circuit_breaker_threshold: 10,
                },
                EVENT_BUFFER_CAPACITY,
            )
            .with_demand_control(),
        );
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            tickers: Arc::new(DashMap::new()),
            depth_books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            depth_subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "gate spot ticker ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch_many(&self, symbols: &[String]) {
        self.manager.activate_scope("spot-market");
        let now = now_ms();
        let mut new_symbols = Vec::new();
        for symbol in symbols {
            let mut needs_subscribe = false;
            self.subscriptions
                .entry(symbol.clone())
                .and_modify(|state| {
                    state.last_touched_ms = now;
                    needs_subscribe = claim_subscription(state);
                })
                .or_insert_with(|| {
                    needs_subscribe = true;
                    SubscriptionState {
                        last_touched_ms: now,
                        sent_on_current_connection: true,
                    }
                });
            if needs_subscribe {
                new_symbols.push(symbol.clone());
            }
        }
        self.spawn_subscribe_batches(&new_symbols);
    }

    fn snapshot(&self, symbols: &[String]) -> Vec<SpotTick> {
        let now = now_ms();
        symbols
            .iter()
            .filter_map(|symbol| self.latest_at(symbol, now))
            .collect()
    }

    fn touch_depth(&self, symbol: &str) {
        self.manager.activate_scope("spot-market");
        let now = now_ms();
        let mut needs_subscribe = false;
        self.depth_subscriptions
            .entry(symbol.to_owned())
            .and_modify(|state| {
                state.last_touched_ms = now;
                needs_subscribe = claim_subscription(state);
            })
            .or_insert_with(|| {
                needs_subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: true,
                }
            });
        if needs_subscribe {
            self.spawn_depth_subscription("subscribe", symbol.to_owned());
        }
    }

    fn latest_depth(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        let cached = self.depth_books.get(symbol)?;
        if !is_fresh(cached.cached_at_ms, now_ms()) {
            return None;
        }
        let mut book = cached.book.clone();
        let cap = usize::try_from(depth.max(1)).ok()?;
        book.bids.truncate(cap);
        book.asks.truncate(cap);
        Some(book)
    }

    fn latest_at(&self, symbol: &str, now: i64) -> Option<SpotTick> {
        let ticker = self.tickers.get(symbol)?;
        if !is_fresh(ticker.cached_at_ms, now) {
            return None;
        }
        parse_spot_tick(&ticker)
    }

    fn spawn_subscribe_batches(&self, symbols: &[String]) {
        for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
            self.spawn_subscription("subscribe", TICKERS_CHANNEL, batch.to_vec());
        }
    }

    fn spawn_subscription(&self, op: &'static str, channel: &'static str, symbols: Vec<String>) {
        if symbols.is_empty() {
            return;
        }
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                release_subscriptions(&subscriptions, &symbols, op);
                debug!(
                    channel,
                    count = symbols.len(),
                    "gate spot ticker ws not connected yet; subscribe resumes after reconnect"
                );
                return;
            }
            let payload = channel_payload(op, channel, &symbols);
            if let Err(error) = manager.send(Message::Text(payload)).await {
                release_subscriptions(&subscriptions, &symbols, op);
                warn!(
                    op,
                    channel,
                    count = symbols.len(),
                    error = %error,
                    "gate spot ticker ws subscription send failed"
                );
            }
        });
    }

    fn spawn_depth_subscription(&self, op: &'static str, symbol: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.depth_subscriptions);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                release_subscriptions(&subscriptions, std::slice::from_ref(&symbol), op);
                return;
            }
            let payload = depth_channel_payload(op, &symbol);
            if let Err(error) = manager.send(Message::Text(payload)).await {
                release_subscriptions(&subscriptions, std::slice::from_ref(&symbol), op);
                warn!(
                    op,
                    symbol,
                    error = %error,
                    "gate spot depth ws subscription send failed"
                );
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
                self.clear_cache();
                warn!(missed, "gate spot ticker ws broadcast receiver lagged");
                true
            }
            Err(RecvError::Closed) => {
                self.clear_cache();
                false
            }
        }
    }

    fn handle_ws_event(&self, event: WsEvent) {
        match event {
            WsEvent::Text(text) => self.on_text(&text),
            WsEvent::Connected => self.on_connected(),
            WsEvent::Disconnected(reason) => self.on_disconnected(&reason),
            WsEvent::CircuitOpened => {
                self.clear_cache();
                warn!("gate spot ticker ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let mut symbols = Vec::with_capacity(self.subscriptions.len());
        for mut entry in self.subscriptions.iter_mut() {
            if claim_subscription(entry.value_mut()) {
                symbols.push(entry.key().clone());
            }
        }
        self.spawn_subscribe_batches(&symbols);
        let mut depth_symbols = Vec::with_capacity(self.depth_subscriptions.len());
        for mut entry in self.depth_subscriptions.iter_mut() {
            if claim_subscription(entry.value_mut()) {
                depth_symbols.push(entry.key().clone());
            }
        }
        for symbol in depth_symbols {
            self.spawn_depth_subscription("subscribe", symbol);
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "gate spot ticker ws disconnected");
        self.clear_cache();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
        for mut entry in self.depth_subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.tickers.clear();
        self.depth_books.clear();
    }

    fn on_text(&self, text: &str) {
        if let Some(row) = parse_ticker_update(text) {
            let symbol = row.item.currency_pair.clone();
            if self.subscriptions.contains_key(&symbol) {
                self.tickers.insert(symbol, row);
            }
        }
        if let Some((symbol, row)) = parse_depth_update(text) {
            if self.depth_subscriptions.contains_key(&symbol) {
                self.depth_books.insert(symbol, row);
            }
        }
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
        for symbol in &pruned {
            self.tickers.remove(symbol);
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "gate spot ticker ws idle prune"
            );
            self.spawn_unsubscribe_batches(pruned);
        }
        self.prune_idle_depth().await;
        if self.subscriptions.is_empty() && self.depth_subscriptions.is_empty() {
            self.manager.suspend_scope("spot-market").await;
            if !self.subscriptions.is_empty() || !self.depth_subscriptions.is_empty() {
                self.manager.activate_scope("spot-market");
            }
        }
    }

    async fn prune_idle_depth(&self) {
        let cutoff = now_ms().saturating_sub(DEPTH_SUBSCRIPTION_IDLE_TTL_MS);
        let mut pruned = Vec::new();
        self.depth_subscriptions.retain(|symbol, state| {
            if state.last_touched_ms < cutoff {
                pruned.push(symbol.clone());
                false
            } else {
                true
            }
        });
        for symbol in &pruned {
            self.depth_books.remove(symbol);
        }
        if !pruned.is_empty() {
            self.spawn_depth_unsubscribes(pruned);
        }
    }

    fn spawn_unsubscribe_batches(&self, symbols: Vec<String>) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
                if let Err(error) = manager
                    .send(Message::Text(channel_payload(
                        "unsubscribe",
                        TICKERS_CHANNEL,
                        batch,
                    )))
                    .await
                {
                    debug!(
                        count = symbols.len(),
                        error = %error,
                        "gate spot ticker ws idle unsubscribe stopped after disconnect"
                    );
                    return;
                }
            }
        });
    }

    fn spawn_depth_unsubscribes(&self, symbols: Vec<String>) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            for symbol in &symbols {
                if let Err(error) = manager
                    .send(Message::Text(depth_channel_payload("unsubscribe", symbol)))
                    .await
                {
                    debug!(
                        count = symbols.len(),
                        error = %error,
                        "gate spot depth ws idle unsubscribe stopped after disconnect"
                    );
                    return;
                }
            }
        });
    }
}

async fn wait_until_connected(manager: &WsManager) {
    let deadline = std::time::Instant::now() + SUBSCRIBE_CONNECT_WAIT;
    while std::time::Instant::now() < deadline {
        if manager.is_connected().await {
            return;
        }
        tokio::time::sleep(SUBSCRIBE_POLL_INTERVAL).await;
    }
}

fn claim_subscription(state: &mut SubscriptionState) -> bool {
    if state.sent_on_current_connection {
        return false;
    }
    state.sent_on_current_connection = true;
    true
}

fn release_subscriptions(
    subscriptions: &DashMap<String, SubscriptionState>,
    symbols: &[String],
    op: &str,
) {
    if op != "subscribe" {
        return;
    }
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = false;
        }
    }
}

fn stream_symbols(symbols: &[String]) -> Option<Vec<String>> {
    if symbols.is_empty() {
        return None;
    }
    let normalized: Vec<String> = symbols
        .iter()
        .filter_map(|symbol| stream_symbol(symbol))
        .collect();
    (!normalized.is_empty()).then_some(normalized)
}

fn stream_symbol(symbol: &str) -> Option<String> {
    crate::spot::native_pair_symbol(symbol, '_')
}

fn channel_payload(op: &str, channel: &str, symbols: &[String]) -> String {
    json!({
        "time": now_secs(),
        "channel": channel,
        "event": op,
        "payload": symbols,
    })
    .to_string()
}

fn depth_channel_payload(op: &str, symbol: &str) -> String {
    json!({
        "time": now_secs(),
        "channel": ORDER_BOOK_CHANNEL,
        "event": op,
        "payload": [symbol, "50", "100ms"],
    })
    .to_string()
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

fn parse_ticker_update(text: &str) -> Option<CachedTicker> {
    let envelope: GateEnvelope<SpotTickerUpdate> = serde_json::from_str(text).ok()?;
    if envelope.channel != TICKERS_CHANNEL || envelope.event != "update" {
        return None;
    }
    let now = now_ms();
    Some(CachedTicker {
        data_timestamp_ms: envelope_timestamp_ms(&envelope, now),
        cached_at_ms: now,
        item: envelope.result?,
    })
}

fn parse_depth_update(text: &str) -> Option<(String, CachedDepthBook)> {
    let envelope: GateEnvelope<SpotDepthUpdate> = serde_json::from_str(text).ok()?;
    if envelope.channel != ORDER_BOOK_CHANNEL || envelope.event != "update" {
        return None;
    }
    let now = now_ms();
    let envelope_ts = envelope_timestamp_ms(&envelope, now);
    let item = envelope.result?;
    let symbol = item.currency_pair.to_ascii_uppercase();
    let bids = parse_depth_levels(&item.bids);
    let asks = parse_depth_levels(&item.asks);
    if bids.is_empty() || asks.is_empty() {
        return None;
    }
    let timestamp = item.timestamp_ms.unwrap_or(envelope_ts);
    let pair = crate::spot::native_pair_symbol(&symbol, '/')?;
    Some((
        symbol,
        CachedDepthBook {
            book: OrderBookInfo {
                symbol: pair,
                exchange: EXCHANGE.into(),
                bids,
                asks,
                timestamp,
            },
            cached_at_ms: now,
        },
    ))
}

fn parse_depth_levels(levels: &[Vec<String>]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|level| Some([level.first()?.parse().ok()?, level.get(1)?.parse().ok()?]))
        .collect()
}

fn envelope_timestamp_ms<T>(envelope: &GateEnvelope<T>, fallback: i64) -> i64 {
    envelope
        .time_ms
        .or_else(|| envelope.time.map(|seconds| seconds * 1000))
        .unwrap_or(fallback)
}

fn parse_spot_tick(ticker: &CachedTicker) -> Option<SpotTick> {
    let (base, quote) = crate::spot::delimited_pair(&ticker.item.currency_pair, '_')?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: EXCHANGE,
        base: &base,
        quote: &quote,
        bid: &ticker.item.highest_bid,
        ask: &ticker.item.lowest_ask,
        last: &ticker.item.last_price,
        bid_size: None,
        ask_size: None,
        volume_24h: &ticker.item.quote_volume,
        exchange_ts_ms: Some(ticker.data_timestamp_ms),
    })
}

#[derive(Debug, Deserialize)]
struct GateEnvelope<T> {
    channel: String,
    event: String,
    time: Option<i64>,
    time_ms: Option<i64>,
    result: Option<T>,
}

#[derive(Debug, Clone, Deserialize)]
struct SpotTickerUpdate {
    currency_pair: String,
    #[serde(default, rename = "last")]
    last_price: String,
    #[serde(default)]
    lowest_ask: String,
    #[serde(default)]
    highest_bid: String,
    #[serde(default)]
    quote_volume: String,
}

#[derive(Debug, Deserialize)]
struct SpotDepthUpdate {
    #[serde(default, rename = "s")]
    currency_pair: String,
    #[serde(default)]
    bids: Vec<Vec<String>>,
    #[serde(default)]
    asks: Vec<Vec<String>>,
    #[serde(default, rename = "t")]
    timestamp_ms: Option<i64>,
}

#[cfg(test)]
#[path = "gate_ws_spot_ticker_tests.rs"]
mod tests;
