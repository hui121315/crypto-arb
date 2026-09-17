//! OKX Spot incremental order-book WebSocket cache.
//!
//! Official docs:
//! - Order-book channel: <https://www.okx.com/docs-v5/en/#order-book-trading-market-data-ws-order-book-channel>
//! - Sequence validation: <https://www.okx.com/en-gb/help/okx-order-book-channels-checksum-field-deprecation>

use super::okx_config::OkxConfig;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use shared_types::OrderBookInfo;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

#[path = "okx_ws_spot_depth_data.rs"]
mod data;
use data::{
    apply_incremental, parse_update, snapshot_book, subscription_payload, BookAction, MergeOutcome,
    ParsedUpdate,
};

const EXCHANGE: &str = "okx";
const WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const CHANNEL: &str = "books";
const MAX_CACHED_DEPTH: usize = 400;
const BOOK_STALE_AFTER_MS: i64 = 10_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const CONNECT_POLL: Duration = Duration::from_millis(100);

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
    last_seq_id: i64,
}

#[derive(Debug)]
struct SpotDepthStream {
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

pub(crate) fn latest_spot_orderbook(
    config: &OkxConfig,
    symbol: &str,
    depth: u32,
) -> Option<OrderBookInfo> {
    if config.base_url_override.is_some() {
        return None;
    }
    let symbol = crate::spot::native_pair_symbol(symbol, '-')?;
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
                warn!(error = %error, "okx spot depth ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch(&self, symbol: &str) {
        self.manager.activate_scope("spot-book");
        let now = now_ms();
        let mut needs_subscribe = false;
        self.subscriptions
            .entry(symbol.to_owned())
            .and_modify(|state| state.last_touched_ms = now)
            .or_insert_with(|| {
                needs_subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: false,
                }
            });
        if needs_subscribe {
            self.spawn_subscription("subscribe", symbol.to_owned());
        }
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

    fn spawn_subscription(&self, op: &'static str, symbol: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                return;
            }
            match manager
                .send(Message::Text(subscription_payload(op, &symbol)))
                .await
            {
                Ok(()) if op == "subscribe" => mark_sent(&subscriptions, &symbol),
                Ok(()) => {}
                Err(error) => warn!(
                    op,
                    symbol = %symbol,
                    error = %error,
                    "okx spot depth ws subscription failed"
                ),
            }
        });
    }

    async fn run_dispatch(self: Arc<Self>) {
        let mut rx = self.manager.subscribe();
        loop {
            match rx.recv().await {
                Ok(event) => self.handle_event(event),
                Err(RecvError::Lagged(missed)) => {
                    warn!(missed, "okx spot depth ws receiver lagged");
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
                warn!("okx spot depth ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let symbols = self
            .subscriptions
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
        for symbol in symbols {
            self.spawn_subscription("subscribe", symbol);
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "okx spot depth ws disconnected");
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_text(&self, text: &str) {
        let Some(update) = parse_update(text) else {
            return;
        };
        match update.action {
            BookAction::Snapshot => {
                if let Some(cached) = snapshot_book(&update) {
                    self.books.insert(update.symbol, cached);
                }
            }
            BookAction::Update => self.apply_update(&update),
        }
    }

    fn apply_update(&self, update: &ParsedUpdate) {
        let symbol = update.symbol.clone();
        let outcome = {
            let Some(mut cached) = self.books.get_mut(&symbol) else {
                return;
            };
            apply_incremental(&mut cached, update)
        };
        if outcome == MergeOutcome::Gap {
            self.books.remove(&symbol);
            self.spawn_reset(symbol);
        }
    }

    fn spawn_reset(&self, symbol: String) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            let _ = manager
                .send(Message::Text(subscription_payload("unsubscribe", &symbol)))
                .await;
            let _ = manager
                .send(Message::Text(subscription_payload("subscribe", &symbol)))
                .await;
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
            self.spawn_subscription("unsubscribe", symbol);
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("spot-book").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("spot-book");
            }
        }
    }
}

async fn wait_until_connected(manager: &WsManager) {
    let deadline = std::time::Instant::now() + CONNECT_WAIT;
    while std::time::Instant::now() < deadline {
        if manager.is_connected().await {
            return;
        }
        tokio::time::sleep(CONNECT_POLL).await;
    }
}

fn mark_sent(subscriptions: &DashMap<String, SubscriptionState>, symbol: &str) {
    if let Some(mut state) = subscriptions.get_mut(symbol) {
        state.sent_on_current_connection = true;
    }
}

#[cfg(test)]
#[path = "okx_ws_spot_depth_tests.rs"]
mod tests;
