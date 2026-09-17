//! Bitget V3 / UTA public order-book WebSocket subscriber.
//!
//! V3 differs from V2 in three places:
//! - URL → `wss://ws.bitget.com/v3/ws/public`.
//! - Subscribe args → `{instType: "usdt-futures", topic: "books", symbol: "BTCUSDT"}`
//!   (V2 used `{instType: "USDT-FUTURES", channel: "books15", instId: "BTCUSDT"}`).
//! - Payload uses short keys `a` / `b` with numeric levels (`[price, size]`
//!   as floats) instead of V2 string-encoded `asks` / `bids`.
//!
//! Official references:
//! - WS intro: <https://www.bitget.com/api-doc/uta/websocket/Intro>
//! - WS public orderbook channel: <https://www.bitget.com/api-doc/uta/websocket/public/OrderBook-Channel>
//! - REST `OrderBook` (same shape): <https://www.bitget.com/api-doc/uta/public/OrderBook>

use super::bitget_uta_config::{BitgetUtaCategory, BitgetUtaWsArgs, PROD_WS_PUBLIC};
use crate::adapter::strip_common_suffixes;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use shared_types::OrderBookInfo;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "bitget";
const TOPIC: &str = "books";
/// Adapter callers request at most 100 levels; the stream cache retains deeper
/// levels so incremental deletes can still refill the visible window.
const DEPTH_LEVEL: u32 = 100;
const MAX_CACHED_DEPTH: usize = 1_000;
const MAX_BOOK_AGE_MS: i64 = 5_000;
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
    last_seq: u64,
    incremental_started: bool,
}

#[derive(Debug)]
pub(crate) struct MarketStream {
    category: BitgetUtaCategory,
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl MarketStream {
    pub(crate) fn new() -> Arc<Self> {
        Self::new_for(BitgetUtaCategory::UsdtFutures)
    }

    pub(crate) fn new_spot() -> Arc<Self> {
        Self::new_for(BitgetUtaCategory::Spot)
    }

    fn new_for(category: BitgetUtaCategory) -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: PROD_WS_PUBLIC.into(),
                exchange: EXCHANGE.into(),
                heartbeat_interval: Duration::from_secs(20),
                heartbeat: WsHeartbeat::Text("ping".into()),
                inbound_codec: WsInboundCodec::Plain,
                server_ping: WsServerPing::None,
                initial_reconnect_delay: Duration::from_secs(1),
                max_reconnect_delay: Duration::from_secs(30),
                circuit_breaker_threshold: 10,
            })
            .with_demand_control(),
        );
        let stream = Arc::new(Self {
            category,
            manager: Arc::clone(&manager),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "bitget uta market ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());

        stream
    }

    pub(crate) fn latest(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        self.books
            .get(&stream_symbol(symbol, self.category))
            .filter(|entry| book_is_fresh_and_complete(&entry.book, now_ms()))
            .map(|entry| truncate_book(&entry.book, depth))
    }

    pub(crate) fn touch(&self, symbol: &str) {
        self.manager.activate_scope("market-book");
        let symbol = stream_symbol(symbol, self.category);
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
        let category = self.category;
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
                    "bitget uta market ws not connected yet; subscribe resumes on next Connected"
                );
                return;
            }
            match manager
                .send(Message::Text(subscribe_payload_for(&symbol, category)))
                .await
            {
                Ok(()) => {
                    if let Some(mut state) = subscriptions.get_mut(&symbol) {
                        state.sent_on_current_connection = true;
                    }
                }
                Err(error) => {
                    warn!(symbol = %symbol, error = %error, "bitget uta market ws subscribe failed");
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
                warn!(missed, "bitget uta market ws broadcast receiver lagged");
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
                warn!("bitget uta market ws circuit opened");
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
        debug!(%reason, "bitget uta market ws disconnected");
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_text(&self, text: &str) {
        let Some(parsed) = parse_book_message_for(text, self.category) else {
            return;
        };
        match parsed.action {
            BookAction::Snapshot => {
                if let Some(book) = snapshot_book(&parsed) {
                    self.books.insert(parsed.stream_symbol, book);
                }
            }
            BookAction::Update => self.apply_update(&parsed),
        }
    }

    fn apply_update(&self, update: &ParsedBookMessage) {
        let symbol = update.stream_symbol.clone();
        let outcome = {
            let Some(mut cached) = self.books.get_mut(&symbol) else {
                return;
            };
            apply_incremental(&mut cached, update)
        };
        if outcome == MergeOutcome::Gap {
            self.books.remove(&symbol);
            self.spawn_resubscribe(symbol);
        }
    }

    fn spawn_resubscribe(&self, symbol: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        let category = self.category;
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            if let Err(error) = manager
                .send(Message::Text(unsubscribe_payload_for(&symbol, category)))
                .await
            {
                warn!(symbol = %symbol, error = %error, "bitget uta market ws reset unsubscribe failed");
                return;
            }
            match manager
                .send(Message::Text(subscribe_payload_for(&symbol, category)))
                .await
            {
                Ok(()) => {
                    if let Some(mut state) = subscriptions.get_mut(&symbol) {
                        state.sent_on_current_connection = true;
                    }
                }
                Err(error) => {
                    warn!(symbol = %symbol, error = %error, "bitget uta market ws reset subscribe failed");
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
        self.drop_books(&pruned);
        self.send_unsubscribes(&pruned).await;
        self.log_prune(&pruned);
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("market-book").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("market-book");
            }
        }
    }

    fn drop_books(&self, pruned: &[String]) {
        for symbol in pruned {
            self.books.remove(symbol);
        }
    }

    async fn send_unsubscribes(&self, pruned: &[String]) {
        for symbol in pruned {
            if let Err(error) = self
                .manager
                .send(Message::Text(unsubscribe_payload_for(
                    symbol,
                    self.category,
                )))
                .await
            {
                debug!(symbol = %symbol, error = %error, "bitget uta market ws unsubscribe failed");
            }
        }
    }

    fn log_prune(&self, pruned: &[String]) {
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "bitget uta market ws idle prune"
            );
        }
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

fn stream_symbol(symbol: &str, category: BitgetUtaCategory) -> String {
    if category == BitgetUtaCategory::Spot {
        return crate::spot::compact_pair_symbol(symbol).unwrap_or_default();
    }
    format!("{}USDT", strip_common_suffixes(symbol))
}

#[cfg(test)]
fn subscribe_payload(symbol: &str) -> String {
    subscribe_payload_for(symbol, BitgetUtaCategory::UsdtFutures)
}

fn subscribe_payload_for(symbol: &str, category: BitgetUtaCategory) -> String {
    json!({
        "op": "subscribe",
        "args": [BitgetUtaWsArgs::new(category, TOPIC, &stream_symbol(symbol, category))],
    })
    .to_string()
}

#[cfg(test)]
fn unsubscribe_payload(symbol: &str) -> String {
    unsubscribe_payload_for(symbol, BitgetUtaCategory::UsdtFutures)
}

fn unsubscribe_payload_for(symbol: &str, category: BitgetUtaCategory) -> String {
    json!({
        "op": "unsubscribe",
        "args": [BitgetUtaWsArgs::new(category, TOPIC, &stream_symbol(symbol, category))],
    })
    .to_string()
}

#[derive(Debug)]
struct ParsedBookMessage {
    category: BitgetUtaCategory,
    stream_symbol: String,
    action: BookAction,
    bids: Vec<[f64; 2]>,
    asks: Vec<[f64; 2]>,
    timestamp: i64,
    seq: u64,
    previous_seq: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BookAction {
    Snapshot,
    Update,
}

#[cfg(test)]
fn parse_book_message(text: &str) -> Option<ParsedBookMessage> {
    parse_book_message_for(text, BitgetUtaCategory::UsdtFutures)
}

fn parse_book_message_for(text: &str, category: BitgetUtaCategory) -> Option<ParsedBookMessage> {
    let envelope: BookEnvelope = serde_json::from_str(text).ok()?;
    if envelope.arg.topic != TOPIC
        || (!envelope.arg.inst_type.is_empty()
            && envelope.arg.inst_type != category.as_ws_inst_type())
    {
        return None;
    }
    let action = match envelope.action.as_str() {
        "snapshot" => BookAction::Snapshot,
        "update" => BookAction::Update,
        _ => return None,
    };
    let item = envelope.data.into_iter().next()?;
    let bids = parse_levels(&item.bids);
    let asks = parse_levels(&item.asks);
    if bids.is_empty() && asks.is_empty() {
        return None;
    }
    let stream_symbol = envelope.arg.symbol;
    let timestamp = item
        .ts
        .parse()
        .ok()
        .or(envelope.ts.map(|v| v as i64))
        .unwrap_or_else(now_ms);
    Some(ParsedBookMessage {
        category,
        stream_symbol,
        action,
        bids,
        asks,
        timestamp,
        seq: parse_u64(&item.seq)?,
        previous_seq: parse_optional_u64(&item.previous_seq),
    })
}

fn parse_levels(raw: &[Vec<serde_json::Value>]) -> Vec<[f64; 2]> {
    raw.iter()
        .filter_map(|level| parse_level(level.as_slice()))
        .filter(|[price, size]| {
            price.is_finite() && size.is_finite() && *price > 0.0 && *size >= 0.0
        })
        .collect()
}

fn parse_level(level: &[serde_json::Value]) -> Option<[f64; 2]> {
    Some([parse_number(level.first()?)?, parse_number(level.get(1)?)?])
}

fn parse_number(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn parse_u64(value: &serde_json::Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

fn parse_optional_u64(value: &serde_json::Value) -> Option<u64> {
    (!value.is_null()).then(|| parse_u64(value)).flatten()
}

fn snapshot_book(message: &ParsedBookMessage) -> Option<CachedBook> {
    let mut book = OrderBookInfo {
        symbol: if message.category == BitgetUtaCategory::Spot {
            crate::spot::native_pair_symbol(&message.stream_symbol, '/')?
        } else {
            strip_common_suffixes(&message.stream_symbol)
        },
        exchange: EXCHANGE.into(),
        bids: positive_levels(&message.bids, true),
        asks: positive_levels(&message.asks, false),
        timestamp: message.timestamp,
    };
    trim_book(&mut book);
    book_is_complete(&book).then_some(CachedBook {
        book,
        last_seq: message.seq,
        incremental_started: false,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MergeOutcome {
    Applied,
    Stale,
    Gap,
}

fn apply_incremental(cached: &mut CachedBook, update: &ParsedBookMessage) -> MergeOutcome {
    if update.seq <= cached.last_seq {
        return MergeOutcome::Stale;
    }
    let Some(previous_seq) = update.previous_seq else {
        return MergeOutcome::Gap;
    };
    let contiguous = if cached.incremental_started {
        previous_seq == cached.last_seq
    } else {
        previous_seq <= cached.last_seq && cached.last_seq <= update.seq
    };
    if !contiguous {
        return MergeOutcome::Gap;
    }
    merge_side(&mut cached.book.bids, &update.bids, true);
    merge_side(&mut cached.book.asks, &update.asks, false);
    cached.book.timestamp = update.timestamp;
    if !book_is_complete(&cached.book) {
        return MergeOutcome::Gap;
    }
    cached.last_seq = update.seq;
    cached.incremental_started = true;
    MergeOutcome::Applied
}

fn merge_side(side: &mut Vec<[f64; 2]>, updates: &[[f64; 2]], descending: bool) {
    for &[price, size] in updates {
        if let Some(index) = side.iter().position(|level| level[0] == price) {
            if size > 0.0 {
                side[index][1] = size;
            } else {
                side.remove(index);
            }
        } else if size > 0.0 {
            side.push([price, size]);
        }
    }
    sort_side(side, descending);
    side.truncate(MAX_CACHED_DEPTH);
}

fn positive_levels(levels: &[[f64; 2]], descending: bool) -> Vec<[f64; 2]> {
    let mut levels: Vec<_> = levels
        .iter()
        .copied()
        .filter(|level| level[1] > 0.0)
        .collect();
    sort_side(&mut levels, descending);
    levels
}

fn sort_side(side: &mut [[f64; 2]], descending: bool) {
    side.sort_by(|left, right| {
        if descending {
            right[0].total_cmp(&left[0])
        } else {
            left[0].total_cmp(&right[0])
        }
    });
}

fn trim_book(book: &mut OrderBookInfo) {
    book.bids.truncate(MAX_CACHED_DEPTH);
    book.asks.truncate(MAX_CACHED_DEPTH);
}

fn book_is_complete(book: &OrderBookInfo) -> bool {
    matches!(
        (book.best_bid(), book.best_ask()),
        (Some(bid), Some(ask)) if bid < ask
    )
}

fn book_is_fresh_and_complete(book: &OrderBookInfo, now_ms: i64) -> bool {
    book_is_complete(book)
        && book.timestamp > 0
        && now_ms.saturating_sub(book.timestamp) <= MAX_BOOK_AGE_MS
}

fn truncate_book(book: &OrderBookInfo, depth: u32) -> OrderBookInfo {
    let depth = depth.clamp(1, DEPTH_LEVEL) as usize;
    let mut book = book.clone();
    book.bids.truncate(depth);
    book.asks.truncate(depth);
    book
}

#[derive(Debug, Deserialize)]
struct BookEnvelope {
    arg: BookArg,
    #[serde(default)]
    action: String,
    #[serde(default)]
    data: Vec<BookData>,
    #[serde(default)]
    ts: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct BookArg {
    #[serde(default, rename = "instType")]
    inst_type: String,
    topic: String,
    symbol: String,
}

#[derive(Debug, Deserialize)]
struct BookData {
    #[serde(default, rename = "b")]
    bids: Vec<Vec<serde_json::Value>>,
    #[serde(default, rename = "a")]
    asks: Vec<Vec<serde_json::Value>>,
    #[serde(default)]
    ts: String,
    #[serde(default)]
    seq: serde_json::Value,
    #[serde(default, rename = "pseq")]
    previous_seq: serde_json::Value,
}

#[cfg(test)]
#[path = "bitget_uta_ws_market_tests.rs"]
mod tests;
