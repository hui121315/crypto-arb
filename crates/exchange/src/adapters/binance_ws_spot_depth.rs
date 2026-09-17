//! Binance Spot partial-depth WebSocket snapshots.
//!
//! Official docs: <https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#partial-book-depth-streams>

use super::binance_config::BinanceConfig;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use shared_types::OrderBookInfo;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

#[path = "binance_ws_spot_depth_data.rs"]
mod data;
use data::{parse_snapshot, subscription_payload};

const EXCHANGE: &str = "binance";
const WS_URL: &str = "wss://stream.binance.com:9443/stream";
const BOOK_STALE_AFTER_MS: i64 = 10_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const CONTROL_FLUSH_INTERVAL: Duration = Duration::from_millis(500);
const MAX_STREAMS_PER_CONTROL_MESSAGE: usize = 1_024;

static SHARED_STREAM: OnceLock<Arc<SpotDepthStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedBook {
    book: OrderBookInfo,
    observed_at_ms: i64,
}

#[derive(Debug)]
struct SpotDepthStream {
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    pending_unsubscribes: Arc<DashMap<String, ()>>,
    next_request_id: AtomicU64,
}

pub(crate) fn latest_spot_orderbook(
    config: &BinanceConfig,
    symbol: &str,
    depth: u32,
) -> Option<OrderBookInfo> {
    if config.testnet || config.base_url_override.is_some() {
        return None;
    }
    let symbol = crate::spot::compact_pair_symbol(symbol)?.to_ascii_lowercase();
    let stream = SHARED_STREAM.get_or_init(SpotDepthStream::new);
    stream.touch(&symbol);
    stream.latest(&symbol, depth)
}

impl SpotDepthStream {
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
            .with_demand_control(),
        );
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            pending_unsubscribes: Arc::new(DashMap::new()),
            next_request_id: AtomicU64::new(1),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "binance spot depth ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_subscription_control());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch(&self, symbol: &str) {
        self.manager.activate_scope("spot-book");
        let now = now_ms();
        self.pending_unsubscribes.remove(symbol);
        self.subscriptions
            .entry(symbol.to_owned())
            .and_modify(|state| state.last_touched_ms = now)
            .or_insert(SubscriptionState {
                last_touched_ms: now,
                sent_on_current_connection: false,
            });
    }

    fn latest(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        let cached = self.books.get(symbol)?;
        if now_ms().saturating_sub(cached.observed_at_ms) > BOOK_STALE_AFTER_MS {
            return None;
        }
        let mut book = cached.book.clone();
        let cap = usize::try_from(depth.max(1)).ok()?;
        book.bids.truncate(cap);
        book.asks.truncate(cap);
        Some(book)
    }

    async fn run_subscription_control(self: Arc<Self>) {
        let mut tick = tokio::time::interval(CONTROL_FLUSH_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if !self.manager.is_connected().await {
                continue;
            }
            self.flush_subscriptions().await;
            self.flush_unsubscribes().await;
        }
    }

    async fn flush_subscriptions(&self) {
        let symbols = self
            .subscriptions
            .iter()
            .filter(|entry| !entry.value().sent_on_current_connection)
            .map(|entry| entry.key().clone())
            .take(MAX_STREAMS_PER_CONTROL_MESSAGE)
            .collect::<Vec<_>>();
        if symbols.is_empty() {
            return;
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let payload = subscription_payload("SUBSCRIBE", &symbols, request_id);
        match self.manager.send(Message::Text(payload)).await {
            Ok(()) => mark_sent(&self.subscriptions, &symbols),
            Err(error) => warn!(
                streams = symbols.len(),
                error = %error,
                "binance spot depth ws batched subscription failed"
            ),
        }
    }

    async fn flush_unsubscribes(&self) {
        let symbols = self
            .pending_unsubscribes
            .iter()
            .map(|entry| entry.key().clone())
            .take(MAX_STREAMS_PER_CONTROL_MESSAGE)
            .collect::<Vec<_>>();
        if symbols.is_empty() {
            return;
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let payload = subscription_payload("UNSUBSCRIBE", &symbols, request_id);
        match self.manager.send(Message::Text(payload)).await {
            Ok(()) => {
                for symbol in symbols {
                    self.pending_unsubscribes.remove(&symbol);
                }
            }
            Err(error) => warn!(
                streams = symbols.len(),
                error = %error,
                "binance spot depth ws batched unsubscribe failed"
            ),
        }
    }

    async fn run_dispatch(self: Arc<Self>) {
        let mut rx = self.manager.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => self.handle_event(event),
                Err(RecvError::Lagged(missed)) => {
                    warn!(missed, "binance spot depth ws receiver lagged");
                    self.books.clear();
                }
                Err(RecvError::Closed) => {
                    self.books.clear();
                    return;
                }
            }
        }
    }

    fn handle_event(&self, event: WsEvent) {
        match event {
            WsEvent::Text(text) => self.on_text(&text),
            WsEvent::Connected => self.on_connected(),
            WsEvent::Disconnected(reason) => self.on_disconnected(&reason),
            WsEvent::CircuitOpened => {
                self.books.clear();
                warn!("binance spot depth ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        self.pending_unsubscribes.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "binance spot depth ws disconnected");
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_text(&self, text: &str) {
        let Some((symbol, book)) = parse_snapshot(text) else {
            return;
        };
        self.books.insert(
            symbol,
            CachedBook {
                book,
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
        for symbol in pruned {
            self.books.remove(&symbol);
            self.pending_unsubscribes.insert(symbol, ());
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("spot-book").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("spot-book");
            }
        }
    }
}

fn mark_sent(subscriptions: &DashMap<String, SubscriptionState>, symbols: &[String]) {
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = true;
        }
    }
}

#[cfg(test)]
#[path = "binance_ws_spot_depth_tests.rs"]
mod tests;
