//! OKX public WebSocket order book subscriber.
//!
//! Official docs:
//! - Public WS URL: <https://www.okx.com/docs-v5/en/#overview-production-trading-services>
//! - Subscribe schema: <https://www.okx.com/docs-v5/en/#overview-websocket-subscribe>
//! - Order book channel: <https://www.okx.com/docs-v5/en/#order-book-trading-market-data-ws-order-book-channel>

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

const EXCHANGE: &str = "okx";
const WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const SWAP_SUFFIX: &str = "-USDT-SWAP";
const CHANNEL: &str = "books";
const DEPTH_LEVEL: u32 = 400;
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
    last_seq_id: i64,
    observed_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct MarketStream {
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl MarketStream {
    pub(crate) fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: WS_URL.into(),
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
            manager: Arc::clone(&manager),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "okx market ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());

        stream
    }

    pub(crate) fn latest(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        self.books
            .get(&stream_symbol(symbol))
            .filter(|entry| now_ms().saturating_sub(entry.observed_at_ms) <= BOOK_STALE_AFTER_MS)
            .map(|entry| truncate_book(&entry.book, depth))
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
                    "okx market ws not connected yet; subscribe will resume on next Connected event"
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
                    warn!(symbol = %symbol, error = %error, "okx market ws subscribe failed");
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
                warn!(missed, "okx market ws broadcast receiver lagged");
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
                warn!("okx market ws circuit opened");
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
        debug!(%reason, "okx market ws disconnected");
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
        let Some(update) = parse_book_update(text) else {
            return;
        };
        if update.action == BookAction::Snapshot {
            self.books.insert(
                update.stream_symbol.clone(),
                CachedBook {
                    book: update.to_book(),
                    last_seq_id: update.seq_id,
                    observed_at_ms: now_ms(),
                },
            );
            return;
        }

        let mut gap = false;
        if let Some(mut entry) = self.books.get_mut(&update.stream_symbol) {
            if update.seq_id == entry.last_seq_id {
                entry.observed_at_ms = now_ms();
            } else if update.prev_seq_id != entry.last_seq_id {
                gap = true;
            } else {
                apply_update(&mut entry.book, &update);
                entry.last_seq_id = update.seq_id;
                entry.observed_at_ms = now_ms();
            }
        }
        if gap {
            self.books.remove(&update.stream_symbol);
            self.spawn_resubscribe(update.stream_symbol);
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
                debug!(symbol = %symbol, error = %error, "okx market ws gap unsubscribe failed");
                return;
            }
            release_subscription(&subscriptions, &symbol);
            tokio::time::sleep(SUBSCRIBE_POLL_INTERVAL).await;
            if !manager.is_connected().await || !claim_subscription(&subscriptions, &symbol) {
                return;
            }
            if let Err(error) = manager
                .send(Message::Text(subscribe_payload(&symbol)))
                .await
            {
                release_subscription(&subscriptions, &symbol);
                warn!(symbol = %symbol, error = %error, "okx market ws gap resubscribe failed");
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
            debug!(symbol = %symbol, error = %error, "okx market ws unsubscribe failed");
        }
    }

    fn log_pruned(&self, pruned: &[String]) {
        if pruned.is_empty() {
            return;
        }
        info!(
            pruned = pruned.len(),
            remaining = self.subscriptions.len(),
            "okx market ws idle prune"
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
    format!("{upper}{SWAP_SUFFIX}")
}

fn subscribe_payload(symbol: &str) -> String {
    json!({
        "op": "subscribe",
        "args": [{
            "channel": CHANNEL,
            "instId": stream_symbol(symbol)
        }]
    })
    .to_string()
}

fn unsubscribe_payload(symbol: &str) -> String {
    json!({
        "op": "unsubscribe",
        "args": [{
            "channel": CHANNEL,
            "instId": stream_symbol(symbol)
        }]
    })
    .to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BookAction {
    Snapshot,
    Update,
}

#[derive(Debug)]
struct BookUpdate {
    stream_symbol: String,
    action: BookAction,
    bids: Vec<[f64; 2]>,
    asks: Vec<[f64; 2]>,
    timestamp_ms: i64,
    seq_id: i64,
    prev_seq_id: i64,
}

impl BookUpdate {
    fn to_book(&self) -> OrderBookInfo {
        OrderBookInfo {
            symbol: strip_common_suffixes(&self.stream_symbol),
            exchange: EXCHANGE.into(),
            bids: sorted_side(nonzero_levels(&self.bids), true),
            asks: sorted_side(nonzero_levels(&self.asks), false),
            timestamp: self.timestamp_ms,
        }
    }
}

fn parse_book_update(text: &str) -> Option<BookUpdate> {
    let envelope: BookEnvelope = serde_json::from_str(text).ok()?;
    if envelope.arg.channel != CHANNEL {
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
    if action == BookAction::Snapshot && bids.is_empty() && asks.is_empty() {
        return None;
    }
    let seq_id = item.seq_id?;
    let prev_seq_id = item.prev_seq_id?;
    if action == BookAction::Snapshot && prev_seq_id != -1 {
        return None;
    }
    Some(BookUpdate {
        stream_symbol: envelope.arg.inst_id,
        action,
        bids,
        asks,
        timestamp_ms: item.ts.parse().unwrap_or_else(|_| now_ms()),
        seq_id,
        prev_seq_id,
    })
}

fn parse_levels(levels: &[Vec<String>]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|level| {
            let price = level.first()?.parse::<f64>().ok()?;
            let size = level.get(1)?.parse::<f64>().ok()?;
            (price > 0.0 && size >= 0.0).then_some([price, size])
        })
        .collect()
}

fn nonzero_levels(levels: &[[f64; 2]]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .copied()
        .filter(|level| level[1] > 0.0)
        .collect()
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
    }
    side.retain(|level| level[1] > 0.0);
    *side = sorted_side(std::mem::take(side), descending);
    side.truncate(DEPTH_LEVEL as usize);
}

fn sorted_side(mut side: Vec<[f64; 2]>, descending: bool) -> Vec<[f64; 2]> {
    side.sort_by(|left, right| {
        if descending {
            right[0].total_cmp(&left[0])
        } else {
            left[0].total_cmp(&right[0])
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
struct BookEnvelope {
    arg: BookArg,
    #[serde(default)]
    action: String,
    #[serde(default)]
    data: Vec<BookData>,
}

#[derive(Debug, Deserialize)]
struct BookArg {
    channel: String,
    #[serde(rename = "instId")]
    inst_id: String,
}

#[derive(Debug, Deserialize)]
struct BookData {
    #[serde(default)]
    bids: Vec<Vec<String>>,
    #[serde(default)]
    asks: Vec<Vec<String>>,
    #[serde(default)]
    ts: String,
    #[serde(default, rename = "seqId")]
    seq_id: Option<i64>,
    #[serde(default, rename = "prevSeqId")]
    prev_seq_id: Option<i64>,
}

#[cfg(test)]
#[path = "okx_ws_market_tests.rs"]
mod tests;
