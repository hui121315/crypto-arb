//! Hyperliquid `l2Book` WebSocket market data subscriber.
//!
//! Maintains a single persistent WebSocket connection to
//! `wss://api.hyperliquid.xyz/ws`, exposes `subscribe` / `unsubscribe`, and
//! caches the latest snapshot per `coin` in a [`dashmap::DashMap`]. Callers
//! (the [`super::hyperliquid::Hyperliquid`] adapter) read snapshots via
//! [`MarketStream::latest`] and refresh-track via [`MarketStream::touch`] to
//! keep coins subscribed.
//!
//! The persistent connection avoids the `POST /info` REST rate-limit (HL
//! 1200 req/min IP-wide) that previously blocked hedge-ticket previews when
//! multiple symbols were inspected in quick succession.
//!
//! Reference: <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket>

use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::OrderBookInfo;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
const BOOK_STALE_AFTER_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Per-coin subscription metadata kept in `subscriptions`.
#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    /// Last `touch()` time; used by the idle cleaner.
    last_touched_ms: i64,
    /// Whether a subscribe frame has been acknowledged on the current connection.
    /// Reset to `false` on disconnect so reconnect can re-subscribe.
    sent_on_current_connection: bool,
}

/// Cached `l2Book` snapshot keyed by raw `coin` (as sent in subscribe).
#[derive(Debug, Clone)]
struct CachedBook {
    book: OrderBookInfo,
    observed_at_ms: i64,
}

/// Market-data WebSocket stream for Hyperliquid `l2Book`.
///
/// One process-wide instance is owned by the [`super::hyperliquid::Hyperliquid`]
/// adapter. Independent supervisor/dispatcher/cleaner tasks are spawned in
/// [`MarketStream::new`]; cloning the returned `Arc` shares the same caches.
#[derive(Debug)]
pub(super) struct MarketStream {
    venue: &'static str,
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl MarketStream {
    /// Build a stream and spawn supervisor / dispatcher / idle-cleaner tasks.
    ///
    /// The supervisor connects to `WS_URL` and reconnects on failure with
    /// exponential backoff (handled by [`WsManager`]). Callers may invoke
    /// [`MarketStream::touch`] immediately; subscribe frames are deferred
    /// until the connection is up.
    pub(super) fn new(venue: &'static str) -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: WS_URL.into(),
                exchange: venue.to_owned(),
                heartbeat_interval: Duration::from_secs(30),
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
            venue,
            manager: Arc::clone(&manager),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        let venue_for_supervisor = venue;
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(
                    venue = %venue_for_supervisor,
                    error = %error,
                    "hyperliquid market ws supervisor exited"
                );
            }
        });

        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());

        stream
    }

    /// Read the latest cached `l2Book` snapshot for `coin`.
    ///
    /// `coin` must match the value used when subscribing (Hyperliquid
    /// builder-dex perps use the `"dex:SYMBOL"` form; core perps use
    /// `"SYMBOL"`). Returns `None` when no snapshot has been received yet.
    pub(super) fn latest(&self, coin: &str) -> Option<OrderBookInfo> {
        self.books
            .get(coin)
            .filter(|entry| now_ms().saturating_sub(entry.observed_at_ms) <= BOOK_STALE_AFTER_MS)
            .map(|entry| entry.book.clone())
    }

    /// Touch `coin` to mark it as still needed.
    ///
    /// On first touch, schedules a subscribe frame asynchronously and creates
    /// a [`SubscriptionState`] entry. Subsequent touches simply bump
    /// `last_touched_ms` so the idle cleaner does not unsubscribe.
    ///
    /// This call returns immediately; the first snapshot will be available
    /// via [`MarketStream::latest`] after the server pushes it.
    pub(super) fn touch(&self, coin: &str) {
        self.manager.activate_scope("market-book");
        let now = now_ms();
        let mut needs_subscribe = false;
        self.subscriptions
            .entry(coin.to_owned())
            .and_modify(|state| state.last_touched_ms = now)
            .or_insert_with(|| {
                needs_subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: false,
                }
            });
        if needs_subscribe {
            self.spawn_subscribe(coin.to_owned());
        }
    }

    fn spawn_subscribe(&self, coin: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        let venue = self.venue;
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
                    venue = %venue,
                    coin = %coin,
                    "hl market ws not connected yet; subscribe will resume on next Connected event"
                );
                return;
            }
            let payload = subscribe_payload(&coin);
            match manager.send(Message::Text(payload)).await {
                Ok(()) => {
                    if let Some(mut state) = subscriptions.get_mut(&coin) {
                        state.sent_on_current_connection = true;
                    }
                    debug!(venue = %venue, coin = %coin, "hl market ws subscribed l2Book");
                }
                Err(error) => {
                    warn!(
                        venue = %venue,
                        coin = %coin,
                        error = %error,
                        "hl market ws subscribe send failed"
                    );
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
                warn!(
                    venue = %self.venue,
                    missed,
                    "hl market ws broadcast receiver lagged"
                );
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
                warn!(venue = %self.venue, "hl market ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let coins: Vec<String> = self
            .subscriptions
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for coin in coins {
            if let Some(mut state) = self.subscriptions.get_mut(&coin) {
                state.sent_on_current_connection = false;
            }
            self.spawn_subscribe(coin);
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(venue = %self.venue, %reason, "hl market ws disconnected");
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_text(&self, text: &str) {
        let Some(parsed) = parse_l2_book(text) else {
            return;
        };
        let book = OrderBookInfo {
            symbol: clean_coin(&parsed.coin),
            exchange: self.venue.into(),
            bids: parsed.bids,
            asks: parsed.asks,
            timestamp: parsed.time,
        };
        self.books.insert(
            parsed.coin,
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
        let pruned = self.take_idle_coins();
        self.remove_pruned_books(&pruned);
        self.unsubscribe_pruned(&pruned).await;
        self.log_pruned(&pruned);
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("market-book").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("market-book");
            }
        }
    }

    fn remove_pruned_books(&self, coins: &[String]) {
        for coin in coins {
            self.books.remove(coin);
        }
    }

    async fn unsubscribe_pruned(&self, coins: &[String]) {
        for coin in coins {
            self.unsubscribe_coin(coin).await;
        }
    }

    async fn unsubscribe_coin(&self, coin: &str) {
        let payload = unsubscribe_payload(coin);
        if let Err(error) = self.manager.send(Message::Text(payload)).await {
            debug!(
                venue = %self.venue,
                coin = %coin,
                error = %error,
                "hl market ws unsubscribe failed (ignored)"
            );
        }
    }

    fn log_pruned(&self, pruned: &[String]) {
        if pruned.is_empty() {
            return;
        }
        info!(
            venue = %self.venue,
            pruned = pruned.len(),
            remaining = self.subscriptions.len(),
            "hl market ws idle prune"
        );
    }

    fn take_idle_coins(&self) -> Vec<String> {
        let cutoff = now_ms().saturating_sub(SUBSCRIPTION_IDLE_TTL_MS);
        let mut pruned: Vec<String> = Vec::new();
        self.subscriptions.retain(|coin, state| {
            if state.last_touched_ms < cutoff {
                pruned.push(coin.clone());
                false
            } else {
                true
            }
        });
        pruned
    }
}

fn subscribe_payload(coin: &str) -> String {
    json!({
        "method": "subscribe",
        "subscription": {"type": "l2Book", "coin": coin}
    })
    .to_string()
}

fn unsubscribe_payload(coin: &str) -> String {
    json!({
        "method": "unsubscribe",
        "subscription": {"type": "l2Book", "coin": coin}
    })
    .to_string()
}

fn clean_coin(coin: &str) -> String {
    coin.split_once(':')
        .map_or(coin, |(_, base)| base)
        .to_owned()
}

#[derive(Debug, Deserialize)]
struct RawEnvelope {
    channel: String,
    #[serde(default)]
    data: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct RawL2BookData {
    coin: String,
    levels: [Vec<RawL2Level>; 2],
    time: i64,
}

#[derive(Debug, Deserialize)]
struct RawL2Level {
    px: String,
    sz: String,
}

struct ParsedL2Book {
    coin: String,
    bids: Vec<[f64; 2]>,
    asks: Vec<[f64; 2]>,
    time: i64,
}

fn parse_l2_book(text: &str) -> Option<ParsedL2Book> {
    let envelope: RawEnvelope = serde_json::from_str(text).ok()?;
    if envelope.channel != "l2Book" {
        return None;
    }
    let data: RawL2BookData = serde_json::from_value(envelope.data?).ok()?;
    let bids = parse_levels(&data.levels[0]);
    let asks = parse_levels(&data.levels[1]);
    Some(ParsedL2Book {
        coin: data.coin,
        bids,
        asks,
        time: data.time,
    })
}

fn parse_levels(levels: &[RawL2Level]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|level| {
            let price = level.px.parse::<f64>().ok()?;
            let size = level.sz.parse::<f64>().ok()?;
            Some([price, size])
        })
        .collect()
}

#[cfg(test)]
#[path = "hyperliquid_ws_market_tests.rs"]
mod tests;
