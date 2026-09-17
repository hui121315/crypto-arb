//! Binance USD-M Futures WebSocket market data subscriber.
//!
//! Official docs:
//! - Market streams: <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams>
//! - Live subscribe/unsubscribe: <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Live-Subscribing-Unsubscribing-to-streams>

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
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "binance";
// 2026-03 起盘口类流的规范路径（changelog 2026-03-05）；旧 /ws 已在停用序列。
const WS_URL: &str = "wss://fstream.binance.com/public/ws";
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
/// 深度快照新鲜度上限：depth20@100ms 正常每 100ms 一条，超过该阈值即视为
/// 冻结数据（半开/静默连接期间禁止继续供给旧盘口）。
const BOOK_STALE_AFTER_MS: i64 = 10_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIPTION_BATCH_DELAY: Duration = Duration::from_millis(250);
const SUBSCRIPTION_WAKE_CAPACITY: usize = 1;

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedBook {
    book: OrderBookInfo,
    /// 本地接收时刻（不用 venue event time，避免时钟偏移干扰新鲜度判定）。
    observed_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct MarketStream {
    manager: Arc<WsManager>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    subscription_wakeup: mpsc::Sender<()>,
}

impl MarketStream {
    pub(crate) fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: WS_URL.into(),
                exchange: EXCHANGE.into(),
                heartbeat_interval: Duration::from_secs(180),
                heartbeat: WsHeartbeat::PingFrame,
                inbound_codec: WsInboundCodec::Plain,
                server_ping: WsServerPing::None,
                initial_reconnect_delay: Duration::from_secs(1),
                max_reconnect_delay: Duration::from_secs(30),
                circuit_breaker_threshold: 10,
            })
            .with_demand_control(),
        );
        let (subscription_wakeup, subscription_events) = mpsc::channel(SUBSCRIPTION_WAKE_CAPACITY);
        let subscriptions = Arc::new(DashMap::new());
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::clone(&subscriptions),
            subscription_wakeup,
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "binance market ws supervisor exited");
            }
        });
        tokio::spawn(run_subscription_worker(
            Arc::clone(&manager),
            subscriptions,
            subscription_events,
        ));
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());

        stream
    }

    pub(crate) fn latest(&self, symbol: &str) -> Option<OrderBookInfo> {
        self.books
            .get(&stream_symbol(symbol))
            .filter(|entry| book_is_fresh(entry.observed_at_ms, now_ms()))
            .map(|entry| entry.book.clone())
    }

    pub(crate) fn touch(&self, symbol: &str) {
        self.manager.activate_scope("perp-book");
        let symbol = stream_symbol(symbol);
        let now = now_ms();
        let mut needs_subscribe = false;
        self.subscriptions
            .entry(symbol)
            .and_modify(|state| state.last_touched_ms = now)
            .or_insert_with(|| {
                needs_subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: false,
                }
            });
        if needs_subscribe {
            self.schedule_subscription_flush();
        }
    }

    fn schedule_subscription_flush(&self) {
        let _ = self.subscription_wakeup.try_send(());
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
                self.books.clear();
                warn!(missed, "binance market ws broadcast receiver lagged");
                true
            }
            Err(RecvError::Closed) => {
                self.books.clear();
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
                self.books.clear();
                warn!("binance market ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
        self.schedule_subscription_flush();
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "binance market ws disconnected");
        // 断线立即清掉盘口缓存：重连间隙不得把冻结深度当作可交易证据。
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_text(&self, text: &str) {
        let Some(parsed) = parse_depth_snapshot(text) else {
            return;
        };
        self.books.insert(
            parsed.stream_symbol.clone(),
            CachedBook {
                book: parsed.book,
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
        if symbols.is_empty() {
            return;
        }
        if let Err(error) = self
            .manager
            .send(Message::Text(unsubscribe_payload(symbols)))
            .await
        {
            debug!(
                symbols = symbols.len(),
                error = %error,
                "binance market ws unsubscribe batch failed"
            );
        }
    }

    fn log_pruned(&self, pruned: &[String]) {
        if pruned.is_empty() {
            return;
        }
        info!(
            pruned = pruned.len(),
            remaining = self.subscriptions.len(),
            "binance market ws idle prune"
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

async fn run_subscription_worker(
    manager: Arc<WsManager>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    mut wakeups: mpsc::Receiver<()>,
) {
    while wakeups.recv().await.is_some() {
        tokio::time::sleep(SUBSCRIPTION_BATCH_DELAY).await;
        while wakeups.try_recv().is_ok() {}
        flush_pending_subscriptions(&manager, &subscriptions).await;
    }
}

async fn flush_pending_subscriptions(
    manager: &WsManager,
    subscriptions: &DashMap<String, SubscriptionState>,
) {
    if !manager.is_connected().await {
        return;
    }
    let symbols = pending_subscription_symbols(subscriptions);
    if symbols.is_empty() {
        return;
    }
    match manager
        .send(Message::Text(subscribe_payload(&symbols)))
        .await
    {
        Ok(()) => mark_subscriptions_sent(subscriptions, &symbols),
        Err(error) => warn!(
            symbols = symbols.len(),
            error = %error,
            "binance market ws subscribe batch failed"
        ),
    }
}

fn pending_subscription_symbols(subscriptions: &DashMap<String, SubscriptionState>) -> Vec<String> {
    let mut symbols = subscriptions
        .iter()
        .filter(|entry| !entry.value().sent_on_current_connection)
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    symbols.sort_unstable();
    symbols
}

fn mark_subscriptions_sent(subscriptions: &DashMap<String, SubscriptionState>, symbols: &[String]) {
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = true;
        }
    }
}

fn stream_symbol(symbol: &str) -> String {
    super::binance_format::usdm_stream_symbol(symbol)
}

fn subscribe_payload(symbols: &[String]) -> String {
    subscription_payload("SUBSCRIBE", symbols)
}

fn unsubscribe_payload(symbols: &[String]) -> String {
    subscription_payload("UNSUBSCRIBE", symbols)
}

fn subscription_payload(method: &str, symbols: &[String]) -> String {
    json!({
        "method": method,
        "params": symbols.iter().map(|symbol| depth_stream(symbol)).collect::<Vec<_>>(),
        "id": 1
    })
    .to_string()
}

fn depth_stream(symbol: &str) -> String {
    format!("{}@depth20@100ms", stream_symbol(symbol))
}

#[derive(Debug)]
struct ParsedDepthSnapshot {
    stream_symbol: String,
    book: OrderBookInfo,
}

fn book_is_fresh(observed_at_ms: i64, now_ms: i64) -> bool {
    now_ms.saturating_sub(observed_at_ms) <= BOOK_STALE_AFTER_MS
}

fn parse_depth_snapshot(text: &str) -> Option<ParsedDepthSnapshot> {
    // 本连接是 raw `/ws` 端点 + SUBSCRIBE：消息没有 combined `{"stream":..}`
    // 信封。用零成本前缀探测选择解析器——此前先试 combined 再 fallback，
    // serde 必须扫完整个 JSON 才能报 missing field，100% 的深度消息
    //（每 symbol 100ms 一条）都付双倍解析成本。
    if text.starts_with("{\"stream\"") {
        parse_combined_depth(text).or_else(|| parse_raw_depth(text))
    } else {
        parse_raw_depth(text).or_else(|| parse_combined_depth(text))
    }
}

fn parse_combined_depth(text: &str) -> Option<ParsedDepthSnapshot> {
    let envelope: CombinedEnvelope = serde_json::from_str(text).ok()?;
    parse_depth_payload(&envelope.data)
}

fn parse_raw_depth(text: &str) -> Option<ParsedDepthSnapshot> {
    let payload: DepthPayload = serde_json::from_str(text).ok()?;
    parse_depth_payload(&payload)
}

fn parse_depth_payload(payload: &DepthPayload) -> Option<ParsedDepthSnapshot> {
    let bids = parse_levels(&payload.bids);
    let asks = parse_levels(&payload.asks);
    if bids.is_empty() && asks.is_empty() {
        return None;
    }
    let stream_symbol = payload.symbol.to_ascii_lowercase();
    let book = OrderBookInfo {
        symbol: strip_common_suffixes(&payload.symbol),
        exchange: EXCHANGE.into(),
        bids,
        asks,
        timestamp: payload
            .event_time
            .or(payload.transaction_time)
            .unwrap_or_else(now_ms),
    };
    Some(ParsedDepthSnapshot {
        stream_symbol,
        book,
    })
}

fn parse_levels(levels: &[Vec<String>]) -> Vec<[f64; 2]> {
    levels
        .iter()
        .filter_map(|level| {
            let price = level.first()?.parse::<f64>().ok()?;
            let size = level.get(1)?.parse::<f64>().ok()?;
            Some([price, size])
        })
        .collect()
}

#[derive(Debug, Deserialize)]
struct CombinedEnvelope {
    data: DepthPayload,
}

#[derive(Debug, Deserialize)]
struct DepthPayload {
    #[serde(rename = "E")]
    event_time: Option<i64>,
    #[serde(rename = "T")]
    transaction_time: Option<i64>,
    #[serde(rename = "s")]
    symbol: String,
    #[serde(rename = "b")]
    bids: Vec<Vec<String>>,
    #[serde(rename = "a")]
    asks: Vec<Vec<String>>,
}

#[cfg(test)]
#[path = "binance_ws_market_tests.rs"]
mod tests;
