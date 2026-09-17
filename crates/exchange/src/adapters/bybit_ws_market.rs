//! Bybit V5 public WebSocket order book subscriber.
//!
//! Official docs:
//! - Public WS connection: <https://bybit-exchange.github.io/docs/v5/ws/connect>
//! - Orderbook topic: <https://bybit-exchange.github.io/docs/v5/websocket/public/orderbook>

use super::bybit_market_data::linear_stream_symbol;
use crate::adapter::strip_common_suffixes;
use crate::ws::manager::{WsEvent, WsManager};
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

const EXCHANGE: &str = "bybit";
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
    update_id: i64,
    observed_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct MarketStream {
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl MarketStream {
    pub(crate) fn new(manager: Arc<WsManager>) -> Arc<Self> {
        let stream = Arc::new(Self {
            manager,
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
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
                    "bybit market ws not connected yet; subscribe will resume on next Connected event"
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
                    warn!(symbol = %symbol, error = %error, "bybit market ws subscribe failed");
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
                warn!(missed, "bybit market ws broadcast receiver lagged");
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
                warn!("bybit market ws circuit opened");
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
        debug!(%reason, "bybit market ws disconnected");
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_text(&self, text: &str) {
        let Some(parsed) = parse_book_message(text) else {
            return;
        };
        match parsed.kind {
            BookMessageKind::Snapshot(snapshot) => {
                self.books.insert(
                    parsed.stream_symbol,
                    CachedBook {
                        book: snapshot.book,
                        update_id: snapshot.update_id,
                        observed_at_ms: now_ms(),
                    },
                );
            }
            BookMessageKind::Delta(delta) => {
                if let Some(mut entry) = self.books.get_mut(&parsed.stream_symbol) {
                    if delta.update_id <= entry.update_id {
                        return;
                    }
                    apply_delta(&mut entry.book, &delta);
                    entry.update_id = delta.update_id;
                    entry.observed_at_ms = now_ms();
                }
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
            debug!(symbol = %symbol, error = %error, "bybit market ws unsubscribe failed");
        }
    }

    fn log_pruned(&self, pruned: &[String]) {
        if pruned.is_empty() {
            return;
        }
        info!(
            pruned = pruned.len(),
            remaining = self.subscriptions.len(),
            "bybit market ws idle prune"
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

fn stream_symbol(symbol: &str) -> String {
    linear_stream_symbol(symbol)
}

fn subscribe_payload(symbol: &str) -> String {
    json!({
        "op": "subscribe",
        "args": [topic(symbol)]
    })
    .to_string()
}

fn unsubscribe_payload(symbol: &str) -> String {
    json!({
        "op": "unsubscribe",
        "args": [topic(symbol)]
    })
    .to_string()
}

fn topic(symbol: &str) -> String {
    format!("orderbook.{DEPTH_LEVEL}.{}", stream_symbol(symbol))
}

#[derive(Debug)]
struct ParsedBookMessage {
    stream_symbol: String,
    kind: BookMessageKind,
}

#[derive(Debug)]
enum BookMessageKind {
    Snapshot(BookSnapshot),
    Delta(BookDelta),
}

#[derive(Debug)]
struct BookSnapshot {
    book: OrderBookInfo,
    update_id: i64,
}

#[derive(Debug)]
struct BookDelta {
    bids: Vec<[f64; 2]>,
    asks: Vec<[f64; 2]>,
    timestamp: i64,
    update_id: i64,
}

fn parse_book_message(text: &str) -> Option<ParsedBookMessage> {
    let envelope: BookEnvelope = serde_json::from_str(text).ok()?;
    if !envelope.topic.starts_with("orderbook.") {
        return None;
    }
    let stream_symbol = envelope.data.symbol.clone();
    let bids = parse_levels(&envelope.data.bids);
    let asks = parse_levels(&envelope.data.asks);
    if bids.is_empty() && asks.is_empty() {
        return None;
    }
    let timestamp = envelope.ts.unwrap_or_else(now_ms);
    let update_id = envelope.data.update_id;
    if update_id <= 0 {
        return None;
    }
    let kind = if envelope.kind == "snapshot" || update_id == 1 {
        BookMessageKind::Snapshot(BookSnapshot {
            book: OrderBookInfo {
                symbol: strip_common_suffixes(&stream_symbol),
                exchange: EXCHANGE.into(),
                bids,
                asks,
                timestamp,
            },
            update_id,
        })
    } else {
        BookMessageKind::Delta(BookDelta {
            bids,
            asks,
            timestamp,
            update_id,
        })
    };
    Some(ParsedBookMessage {
        stream_symbol,
        kind,
    })
}

fn parse_levels(levels: &[[String; 2]]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|[price, size]| Some([price.parse().ok()?, size.parse().ok()?]))
        .collect()
}

fn apply_delta(book: &mut OrderBookInfo, delta: &BookDelta) {
    update_side(&mut book.bids, &delta.bids, true);
    update_side(&mut book.asks, &delta.asks, false);
    book.timestamp = delta.timestamp;
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
    sort_side(side, descending);
    side.truncate(DEPTH_LEVEL as usize);
}

fn sort_side(side: &mut [[f64; 2]], descending: bool) {
    side.sort_by(|a, b| {
        if descending {
            b[0].total_cmp(&a[0])
        } else {
            a[0].total_cmp(&b[0])
        }
    });
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
    topic: String,
    #[serde(rename = "type")]
    kind: String,
    data: BookData,
    ts: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct BookData {
    #[serde(rename = "s")]
    symbol: String,
    #[serde(default, rename = "b")]
    bids: Vec<[String; 2]>,
    #[serde(default, rename = "a")]
    asks: Vec<[String; 2]>,
    #[serde(default, rename = "u")]
    update_id: i64,
}

#[cfg(test)]
#[path = "bybit_ws_market_tests.rs"]
mod tests;
