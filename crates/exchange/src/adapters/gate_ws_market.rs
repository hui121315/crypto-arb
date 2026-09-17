//! Gate.io futures public WebSocket order book subscriber.
//!
//! Official docs:
//! - Futures WS API: <https://www.gate.com/docs/developers/futures/ws/>
//! - Order book V2 channel: <https://www.gate.com/docs/developers/futures/ws/en/#order-book-v2-subscription>

use crate::adapter::strip_common_suffixes;
use crate::adapters::gate_public_rest::{SIZE_DECIMAL_HEADER, SIZE_DECIMAL_HEADER_VALUE};
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::{now_ms, now_secs};
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::OrderBookInfo;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "gate";
const WS_URL: &str = "wss://fx-ws.gateio.ws/v4/ws/usdt";
const CHANNEL: &str = "futures.obu";
const DEPTH_LEVEL: u32 = 50;
const BOOK_STALE_AFTER_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const MAX_ACTIVE_SUBSCRIPTIONS: usize = 1;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

static SHARED_STREAM: OnceLock<Arc<MarketStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedBook {
    book: OrderBookInfo,
    last_update_id: i64,
    observed_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct MarketStream {
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl MarketStream {
    pub(crate) fn shared() -> Arc<Self> {
        Arc::clone(SHARED_STREAM.get_or_init(Self::new))
    }

    fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: WS_URL.into(),
                exchange: EXCHANGE.into(),
                heartbeat_interval: Duration::from_secs(20),
                heartbeat: WsHeartbeat::PingFrame,
                inbound_codec: WsInboundCodec::Plain,
                server_ping: WsServerPing::None,
                initial_reconnect_delay: Duration::from_secs(1),
                max_reconnect_delay: Duration::from_secs(30),
                circuit_breaker_threshold: 10,
            })
            .with_connect_header(SIZE_DECIMAL_HEADER, SIZE_DECIMAL_HEADER_VALUE)
            .with_demand_control(),
        );
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "gate market ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());

        stream
    }

    pub(crate) fn latest(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        let entry = self.books.get(&stream_symbol(symbol))?;
        (now_ms().saturating_sub(entry.observed_at_ms) <= BOOK_STALE_AFTER_MS)
            .then(|| truncate_book(&entry.book, depth))
    }

    pub(crate) fn touch(&self, symbol: &str) {
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
            self.evict_excess_subscriptions(&symbol);
            self.spawn_subscribe(symbol);
        }
    }

    fn evict_excess_subscriptions(&self, protected: &str) {
        let evicted = take_over_capacity(&self.subscriptions, protected, MAX_ACTIVE_SUBSCRIPTIONS);
        let evicted_count = evicted.len();
        for symbol in &evicted {
            self.books.remove(symbol);
        }
        self.spawn_unsubscribe(evicted);
        if evicted_count > 0 {
            info!(
                evicted = evicted_count,
                remaining = self.subscriptions.len(),
                limit = MAX_ACTIVE_SUBSCRIPTIONS,
                "gate market ws subscription capacity enforced"
            );
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
                    "gate market ws not connected yet; subscribe will resume on next Connected event"
                );
                return;
            }
            if !claim_subscription(&subscriptions, &symbol) {
                return;
            }
            match manager
                .send(Message::Text(subscribe_payload(&symbol)))
                .await
            {
                Ok(()) => {}
                Err(error) => {
                    release_subscription(&subscriptions, &symbol);
                    warn!(symbol = %symbol, error = %error, "gate market ws subscribe failed");
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
                warn!(missed, "gate market ws broadcast receiver lagged");
                self.books.clear();
                self.resubscribe_all();
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
                warn!("gate market ws circuit opened");
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
        debug!(%reason, "gate market ws disconnected");
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn resubscribe_all(&self) {
        let symbols = self
            .subscriptions
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for symbol in symbols {
            self.spawn_resubscribe(symbol);
        }
    }

    fn on_text(&self, text: &str) {
        let Some(update) = parse_update(text) else {
            return;
        };
        if update.full {
            self.books.insert(
                update.contract.clone(),
                CachedBook {
                    book: update.to_book(),
                    last_update_id: update.last_id(),
                    observed_at_ms: now_ms(),
                },
            );
            return;
        }

        let mut gap = false;
        if let Some(mut entry) = self.books.get_mut(&update.contract) {
            if update.is_stale(entry.last_update_id) {
                return;
            }
            if !update.is_next_for(entry.last_update_id) {
                gap = true;
            } else {
                apply_update(&mut entry.book, &update);
                entry.last_update_id = update.last_id();
                entry.observed_at_ms = now_ms();
            }
        }
        if gap {
            self.books.remove(&update.contract);
            self.spawn_resubscribe(update.contract);
        }
    }

    fn spawn_resubscribe(&self, symbol: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            if let Err(error) = manager
                .send(Message::Text(unsubscribe_payload(&symbol)))
                .await
            {
                debug!(symbol = %symbol, error = %error, "gate market ws gap unsubscribe failed");
                return;
            }
            if let Some(mut state) = subscriptions.get_mut(&symbol) {
                state.sent_on_current_connection = false;
            }
            tokio::time::sleep(SUBSCRIBE_POLL_INTERVAL).await;
            if !manager.is_connected().await {
                return;
            }
            if !claim_subscription(&subscriptions, &symbol) {
                return;
            }
            match manager
                .send(Message::Text(subscribe_payload(&symbol)))
                .await
            {
                Ok(()) => {}
                Err(error) => {
                    release_subscription(&subscriptions, &symbol);
                    warn!(symbol = %symbol, error = %error, "gate market ws gap resubscribe failed");
                }
            }
        });
    }

    fn spawn_unsubscribe(&self, symbols: Vec<String>) {
        if symbols.is_empty() {
            return;
        }
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            for symbol in symbols {
                if let Err(error) = manager
                    .send(Message::Text(unsubscribe_payload(&symbol)))
                    .await
                {
                    debug!(
                        symbol = %symbol,
                        error = %error,
                        "gate market ws capacity unsubscribe failed"
                    );
                }
            }
        });
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
            debug!(symbol = %symbol, error = %error, "gate market ws unsubscribe failed");
        }
    }

    fn log_pruned(&self, pruned: &[String]) {
        if pruned.is_empty() {
            return;
        }
        info!(
            pruned = pruned.len(),
            remaining = self.subscriptions.len(),
            "gate market ws idle prune"
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

fn take_over_capacity(
    subscriptions: &DashMap<String, SubscriptionState>,
    protected: &str,
    limit: usize,
) -> Vec<String> {
    let mut evicted = Vec::new();
    while subscriptions.len() > limit {
        let oldest = subscriptions
            .iter()
            .filter(|entry| entry.key().as_str() != protected)
            .min_by_key(|entry| entry.value().last_touched_ms)
            .map(|entry| entry.key().clone());
        let Some(symbol) = oldest else {
            break;
        };
        if subscriptions.remove(&symbol).is_some() {
            evicted.push(symbol);
        }
    }
    evicted
}

fn claim_subscription(subscriptions: &DashMap<String, SubscriptionState>, symbol: &str) -> bool {
    let Some(mut state) = subscriptions.get_mut(symbol) else {
        return false;
    };
    if state.sent_on_current_connection {
        return false;
    }
    state.sent_on_current_connection = true;
    true
}

fn release_subscription(subscriptions: &DashMap<String, SubscriptionState>, symbol: &str) {
    if let Some(mut state) = subscriptions.get_mut(symbol) {
        state.sent_on_current_connection = false;
    }
}

fn stream_symbol(symbol: &str) -> String {
    let upper = strip_common_suffixes(symbol);
    format!("{upper}_USDT")
}

fn subscribe_payload(symbol: &str) -> String {
    command_payload("subscribe", symbol)
}

fn unsubscribe_payload(symbol: &str) -> String {
    command_payload("unsubscribe", symbol)
}

fn command_payload(event: &str, symbol: &str) -> String {
    json!({
        "time": now_secs(),
        "channel": CHANNEL,
        "event": event,
        "payload": [depth_stream(symbol)]
    })
    .to_string()
}

fn depth_stream(symbol: &str) -> String {
    format!("ob.{}.{DEPTH_LEVEL}", stream_symbol(symbol))
}

#[derive(Debug)]
struct BookUpdate {
    contract: String,
    first_id: i64,
    update_id: i64,
    timestamp_ms: i64,
    full: bool,
    bids: Vec<[f64; 2]>,
    asks: Vec<[f64; 2]>,
}

impl BookUpdate {
    fn last_id(&self) -> i64 {
        self.update_id
    }

    fn is_stale(&self, last_id: i64) -> bool {
        self.update_id <= last_id
    }

    fn is_next_for(&self, last_id: i64) -> bool {
        self.first_id == last_id.saturating_add(1)
    }

    fn to_book(&self) -> OrderBookInfo {
        OrderBookInfo {
            symbol: strip_common_suffixes(&self.contract),
            exchange: EXCHANGE.into(),
            bids: sorted_side(self.bids.clone(), true),
            asks: sorted_side(self.asks.clone(), false),
            timestamp: self.timestamp_ms,
        }
    }
}

fn parse_update(text: &str) -> Option<BookUpdate> {
    let envelope: GateEnvelope = serde_json::from_str(text).ok()?;
    if envelope.channel != CHANNEL || envelope.event != "update" {
        return None;
    }
    let result = envelope.result?;
    let bids = parse_levels(&result.bids);
    let asks = parse_levels(&result.asks);
    if result.full && bids.is_empty() && asks.is_empty() {
        return None;
    }
    Some(BookUpdate {
        contract: contract_from_stream(&result.stream)?,
        first_id: result.first_update_id,
        update_id: result.update_id,
        timestamp_ms: gate_timestamp_ms(result.update_time),
        full: result.full,
        bids,
        asks,
    })
}

fn contract_from_stream(stream: &str) -> Option<String> {
    let contract = stream.strip_prefix("ob.")?.rsplit_once('.')?.0;
    (!contract.is_empty()).then(|| contract.to_owned())
}

fn gate_timestamp_ms(value: f64) -> i64 {
    if value >= 1_000_000_000_000.0 {
        value as i64
    } else {
        (value * 1000.0) as i64
    }
}

fn parse_levels(levels: &[[Value; 2]]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|[price, size]| Some([parse_number(price)?, parse_number(size)?]))
        .filter(|[price, size]| *price > 0.0 && *size >= 0.0)
        .collect()
}

fn parse_number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn apply_update(book: &mut OrderBookInfo, update: &BookUpdate) {
    update_side(&mut book.bids, &update.bids, true);
    update_side(&mut book.asks, &update.asks, false);
    book.timestamp = update.timestamp_ms;
}

fn update_side(side: &mut Vec<[f64; 2]>, updates: &[[f64; 2]], descending: bool) {
    for &[price, size] in updates {
        if let Some(level) = side
            .iter_mut()
            .find(|level| (level[0] - price).abs() < f64::EPSILON)
        {
            level[1] = size;
        } else if size > 0.0 {
            side.push([price, size]);
        }
        side.retain(|level| level[1] > 0.0);
    }
    *side = sorted_side(side.clone(), descending);
    side.truncate(DEPTH_LEVEL as usize);
}

fn sorted_side(mut side: Vec<[f64; 2]>, descending: bool) -> Vec<[f64; 2]> {
    side.retain(|level| level[1] > 0.0);
    side.sort_by(|a, b| {
        if descending {
            b[0].total_cmp(&a[0])
        } else {
            a[0].total_cmp(&b[0])
        }
    });
    side
}

fn truncate_book(book: &OrderBookInfo, depth: u32) -> OrderBookInfo {
    let depth = depth.clamp(1, DEPTH_LEVEL) as usize;
    let mut book = book.clone();
    book.bids.truncate(depth);
    book.asks.truncate(depth);
    book
}

#[derive(Debug, Deserialize)]
struct GateEnvelope {
    channel: String,
    event: String,
    #[serde(default)]
    result: Option<GateResult>,
}

#[derive(Debug, Deserialize)]
struct GateResult {
    #[serde(rename = "s")]
    stream: String,
    #[serde(default, rename = "U")]
    first_update_id: i64,
    #[serde(default, rename = "u")]
    update_id: i64,
    #[serde(default, rename = "t")]
    update_time: f64,
    #[serde(default)]
    full: bool,
    #[serde(default, rename = "b")]
    bids: Vec<[Value; 2]>,
    #[serde(default, rename = "a")]
    asks: Vec<[Value; 2]>,
}

#[cfg(test)]
#[path = "gate_ws_market_tests.rs"]
mod tests;
