//! KuCoin Classic Spot public WebSocket level-50 order books.
//!
//! Official docs:
//! - Public token: <https://www.kucoin.com/docs-new/api-3470294>
//! - Level 50: <https://www.kucoin.com/docs-new/3470070w0>

use super::kucoin_config::KucoinConfig;
use crate::http::HttpClient;
use crate::ws::manager::{WsEvent, WsManager};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use shared_types::OrderBookInfo;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

const EXCHANGE: &str = "kucoin";
const TOPIC_PREFIX: &str = "/spotMarket/level2Depth50";
const DEPTH_LEVEL: u32 = 50;
const BOOK_STALE_AFTER_MS: i64 = 30_000;
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
}

#[derive(Debug)]
struct SpotDepthStream {
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

pub(crate) fn latest_spot_orderbook(
    config: &KucoinConfig,
    http: &HttpClient,
    symbol: &str,
    depth: u32,
) -> Option<OrderBookInfo> {
    if config.base_url_override.is_some() {
        return None;
    }
    let symbol = crate::spot::native_pair_symbol(symbol, '-')?;
    let manager = super::kucoin_ws_spot_ticker::shared_manager(config, http)?;
    let stream = SHARED_STREAM.get_or_init(|| SpotDepthStream::new(manager));
    stream.touch(&symbol);
    stream.latest(&symbol, depth)
}

impl SpotDepthStream {
    fn new(manager: Arc<WsManager>) -> Arc<Self> {
        let stream = Arc::new(Self {
            manager,
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch(&self, symbol: &str) {
        self.manager.activate_scope("spot-book");
        let now = now_ms();
        let mut subscribe = false;
        self.subscriptions
            .entry(symbol.to_owned())
            .and_modify(|state| state.last_touched_ms = now)
            .or_insert_with(|| {
                subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: false,
                }
            });
        if subscribe {
            self.spawn_subscription("subscribe", symbol.to_owned());
        }
    }

    fn latest(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        let cached = self.books.get(symbol)?;
        if now_ms().saturating_sub(cached.observed_at_ms) > BOOK_STALE_AFTER_MS {
            return None;
        }
        let mut book = cached.book.clone();
        let cap = depth.clamp(1, DEPTH_LEVEL) as usize;
        book.bids.truncate(cap);
        book.asks.truncate(cap);
        Some(book)
    }

    fn spawn_subscription(&self, kind: &'static str, symbol: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                return;
            }
            match manager
                .send(Message::Text(command_payload(kind, &symbol)))
                .await
            {
                Ok(()) if kind == "subscribe" => {
                    if let Some(mut state) = subscriptions.get_mut(&symbol) {
                        state.sent_on_current_connection = true;
                    }
                }
                Ok(()) => {}
                Err(error) => warn!(
                    kind,
                    symbol = %symbol,
                    error = %error,
                    "kucoin classic spot depth subscription failed"
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
                    warn!(missed, "kucoin classic spot depth receiver lagged");
                    self.books.clear();
                }
                Err(RecvError::Closed) => return,
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
                warn!("kucoin classic spot depth ws circuit opened");
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
        debug!(%reason, "kucoin classic spot depth ws disconnected");
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
            let keep = state.last_touched_ms >= cutoff;
            if !keep {
                pruned.push(symbol.clone());
            }
            keep
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

fn command_payload(kind: &str, symbol: &str) -> String {
    json!({
        "id": format!("spot-depth-{symbol}-{}", now_ms()),
        "type": kind,
        "topic": format!("{TOPIC_PREFIX}:{symbol}"),
        "response": true,
    })
    .to_string()
}

fn parse_snapshot(text: &str) -> Option<(String, OrderBookInfo)> {
    let envelope: DepthEnvelope = serde_json::from_str(text).ok()?;
    if !envelope.topic.starts_with(TOPIC_PREFIX) {
        return None;
    }
    let symbol = envelope.topic.split_once(':')?.1.to_owned();
    let data = envelope.data?;
    let bids = parse_levels(&data.bids);
    let asks = parse_levels(&data.asks);
    if bids.is_empty() || asks.is_empty() {
        return None;
    }
    Some((
        symbol.clone(),
        OrderBookInfo {
            symbol: crate::spot::native_pair_symbol(&symbol, '/')?,
            exchange: EXCHANGE.into(),
            bids,
            asks,
            timestamp: data.timestamp.unwrap_or_else(now_ms),
        },
    ))
}

fn parse_levels(levels: &[[String; 2]]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|[price, size]| Some([price.parse().ok()?, size.parse().ok()?]))
        .filter(|[price, size]| *price > 0.0 && *size > 0.0)
        .collect()
}

#[derive(Debug, Deserialize)]
struct DepthEnvelope {
    #[serde(default)]
    topic: String,
    #[serde(default)]
    data: Option<DepthData>,
}

#[derive(Debug, Deserialize)]
struct DepthData {
    #[serde(default)]
    bids: Vec<[String; 2]>,
    #[serde(default)]
    asks: Vec<[String; 2]>,
    #[serde(default)]
    timestamp: Option<i64>,
}

#[cfg(test)]
#[path = "kucoin_ws_spot_depth_tests.rs"]
mod tests;
