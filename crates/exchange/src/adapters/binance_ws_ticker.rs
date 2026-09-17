//! Binance USD-M Futures public ticker WebSocket subscriber.
//!
//! ## 2026-03 路径拆分（changelog 2026-03-05；旧 `/ws` 只剩盘口类流）
//!
//! 盘口类（bookTicker）在 `/public/ws`，统计类（24hrTicker）在 `/market/ws`，
//! 本模块双连接各订各的：
//! - `<symbol>@ticker` 提供稳定的 bid / ask / last / `volume_24h` 基线；
//! - 当前有界计划的 `<symbol>@bookTicker` 覆盖为更低延迟的 bid / ask / timestamp。
//!
//! 逐符号实时盘口由构建/执行路径按需订阅；扫描常驻路径不再为每个候选消费
//! 高频 `<symbol>@bookTicker`；不会订阅或接收全市场 `!bookTicker`。
//! （0 量在下游流动性过滤里天然保守）。
//!
//! Official docs:
//! - Book ticker: <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Individual-Symbol-Book-Ticker-Streams>
//! - 24h ticker: <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Individual-Symbol-Ticker-Streams>
//! - Live subscribe/unsubscribe: <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Live-Subscribing-Unsubscribing-to-streams>

use super::binance_config::BinanceConfig;
use crate::adapter::strip_common_suffixes;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use shared_types::TickerInfo;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "binance";
// 2026-03 起币安把市场流按类拆路径：盘口类（bookTicker/depth）在 /public，
// 统计类（24hrTicker/markPrice 等）在 /market；旧 /ws 只剩盘口类在服务。
const WS_PUBLIC_URL: &str = "wss://fstream.binance.com/public/ws";
const WS_MARKET_URL: &str = "wss://fstream.binance.com/market/ws";
const EVENT_24H_TICKER: &str = "24hrTicker";
const EVENT_BOOK_TICKER: &str = "bookTicker";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LAG_REPORT_INTERVAL_MS: i64 = 5_000;
const EVENT_BUFFER_CAPACITY: usize = 8_192;

static SHARED_STREAM: OnceLock<Arc<TickerStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
}

#[derive(Debug, Clone)]
struct CachedTicker {
    item: Ticker24hUpdate,
    cached_at_ms: i64,
}

#[derive(Debug, Clone)]
struct CachedBook {
    item: BookTickerUpdate,
    cached_at_ms: i64,
}

#[derive(Debug, Clone, Copy)]
enum TickerSocket {
    Book,
    Stats,
}

#[derive(Debug, Default)]
struct LagReportState {
    pending_missed: AtomicU64,
    last_reported_at_ms: AtomicI64,
}

impl LagReportState {
    fn observe(&self, missed: u64, observed_at_ms: i64) -> Option<u64> {
        self.pending_missed.fetch_add(missed, Ordering::Relaxed);
        let previous = self.last_reported_at_ms.load(Ordering::Acquire);
        if observed_at_ms.saturating_sub(previous) < LAG_REPORT_INTERVAL_MS
            || self
                .last_reported_at_ms
                .compare_exchange(
                    previous,
                    observed_at_ms,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_err()
        {
            return None;
        }
        Some(self.pending_missed.swap(0, Ordering::AcqRel))
    }
}

#[derive(Debug)]
pub(crate) struct TickerStream {
    /// /public/ws：当前有界计划的实时 bookTicker（发现态 BBO）。
    book_manager: Arc<WsManager>,
    /// /market/ws：24hrTicker（last / 24h 量统计）。
    stats_manager: Arc<WsManager>,
    tickers: Arc<DashMap<String, CachedTicker>>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    next_request_id: AtomicU64,
    lag_report: LagReportState,
}

pub(crate) fn latest_ticker(config: &BinanceConfig, symbol: &str) -> Option<TickerInfo> {
    let stream = enabled_stream(config)?;
    stream.touch_many(&[symbol.to_owned()]);
    stream.latest(symbol)
}

/// 部分新鲜即服务：venue 不上市/迟到的符号只缺席自己的行，
/// 不再把整个 venue 打回 REST。
pub(crate) fn snapshot_tickers(
    config: &BinanceConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<TickerInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let rows = stream.snapshot(symbols);
    (!rows.is_empty()).then_some(rows)
}

fn enabled_stream(config: &BinanceConfig) -> Option<Arc<TickerStream>> {
    if config.testnet || config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(TickerStream::new)))
}

impl TickerStream {
    fn make_manager(url: &str) -> Arc<WsManager> {
        Arc::new(
            WsManager::new_with_event_capacity(
                WsConfig {
                    url: url.into(),
                    exchange: EXCHANGE.into(),
                    heartbeat_interval: Duration::from_secs(30),
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
        )
    }

    fn new() -> Arc<Self> {
        let book_manager = Self::make_manager(WS_PUBLIC_URL);
        let stats_manager = Self::make_manager(WS_MARKET_URL);
        let stream = Arc::new(Self {
            book_manager: Arc::clone(&book_manager),
            stats_manager: Arc::clone(&stats_manager),
            tickers: Arc::new(DashMap::new()),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            next_request_id: AtomicU64::new(1),
            lag_report: LagReportState::default(),
        });

        for (manager, socket) in [
            (&book_manager, TickerSocket::Book),
            (&stats_manager, TickerSocket::Stats),
        ] {
            let supervisor = Arc::clone(manager);
            tokio::spawn(async move {
                if let Err(error) = supervisor.run().await {
                    warn!(error = %error, "binance ticker ws supervisor exited");
                }
            });
            tokio::spawn(Arc::clone(&stream).run_dispatch(manager.subscribe(), socket));
        }
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch_many(&self, symbols: &[String]) {
        if symbols.is_empty() {
            return;
        }
        self.book_manager.activate_scope("perp-ticker");
        self.stats_manager.activate_scope("perp-ticker");
        let now = now_ms();
        let mut new_symbols = Vec::new();
        for symbol in symbols {
            let symbol = stream_symbol(symbol);
            let mut needs_subscribe = false;
            self.subscriptions
                .entry(symbol.clone())
                .and_modify(|state| state.last_touched_ms = now)
                .or_insert_with(|| {
                    needs_subscribe = true;
                    SubscriptionState {
                        last_touched_ms: now,
                    }
                });
            if needs_subscribe {
                new_symbols.push(symbol);
            }
        }
        self.spawn_subscribe_batches(TickerSocket::Book, &new_symbols);
        self.spawn_subscribe_batches(TickerSocket::Stats, &new_symbols);
    }

    fn latest(&self, symbol: &str) -> Option<TickerInfo> {
        self.latest_at(symbol, now_ms())
    }

    fn snapshot(&self, symbols: &[String]) -> Vec<TickerInfo> {
        let now = now_ms();
        symbols
            .iter()
            .filter_map(|symbol| self.latest_at(symbol, now))
            .collect()
    }

    /// 24hrTicker 是完整基线；新鲜 bookTicker 只覆盖为更低延迟的 BBO。
    fn latest_at(&self, symbol: &str, now: i64) -> Option<TickerInfo> {
        let key = stream_symbol(symbol);
        let book = self
            .books
            .get(&key)
            .filter(|row| is_fresh(row.cached_at_ms, now));
        if let Some(ticker) = self
            .tickers
            .get(&key)
            .filter(|row| is_fresh(row.cached_at_ms, now))
        {
            return parse_ticker(&ticker.item, book.as_ref().map(|row| &row.item));
        }
        assemble_from_book(&book?.item)
    }

    fn spawn_subscribe_batches(&self, socket: TickerSocket, symbols: &[String]) {
        for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
            self.spawn_subscription(socket, "SUBSCRIBE", batch.to_vec());
        }
    }

    fn spawn_subscription(&self, socket: TickerSocket, method: &'static str, symbols: Vec<String>) {
        if symbols.is_empty() {
            return;
        }
        let manager = match socket {
            TickerSocket::Book => Arc::clone(&self.book_manager),
            TickerSocket::Stats => Arc::clone(&self.stats_manager),
        };
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            if method != "UNSUBSCRIBE" {
                wait_until_connected(&manager).await;
            }
            if !manager.is_connected().await {
                debug!(
                    method,
                    ?socket,
                    count = symbols.len(),
                    "binance ticker ws not connected yet; subscribe resumes after reconnect"
                );
                return;
            }
            let payload = match socket {
                TickerSocket::Book => book_subscription_payload(method, &symbols, request_id),
                TickerSocket::Stats => stats_subscription_payload(method, &symbols, request_id),
            };
            if let Err(error) = manager.send(Message::Text(payload)).await {
                if method == "UNSUBSCRIBE" {
                    debug!(
                        ?socket,
                        count = symbols.len(),
                        error = %error,
                        "binance ticker ws disconnected before idle unsubscribe"
                    );
                } else {
                    warn!(
                        method,
                        ?socket,
                        count = symbols.len(),
                        error = %error,
                        "binance ticker ws subscription send failed"
                    );
                }
            }
        });
    }

    async fn run_dispatch(
        self: Arc<Self>,
        mut rx: tokio::sync::broadcast::Receiver<WsEvent>,
        socket: TickerSocket,
    ) {
        loop {
            if !self.handle_dispatch_result(rx.recv().await, socket) {
                return;
            }
        }
    }

    fn handle_dispatch_result(
        &self,
        event: Result<WsEvent, RecvError>,
        socket: TickerSocket,
    ) -> bool {
        match event {
            Ok(event) => {
                self.handle_ws_event(event, socket);
                true
            }
            Err(RecvError::Lagged(missed)) => {
                self.clear_socket_cache(socket);
                if let Some(window_missed) = self.lag_report.observe(missed, now_ms()) {
                    warn!(
                        missed = window_missed,
                        window_ms = LAG_REPORT_INTERVAL_MS,
                        "binance ticker ws broadcast receiver lagged"
                    );
                }
                true
            }
            Err(RecvError::Closed) => {
                self.clear_socket_cache(socket);
                false
            }
        }
    }

    fn handle_ws_event(&self, event: WsEvent, socket: TickerSocket) {
        match event {
            WsEvent::Text(text) => self.on_text(&text),
            WsEvent::Connected => self.on_connected(socket),
            WsEvent::Disconnected(reason) => self.on_disconnected(&reason, socket),
            WsEvent::CircuitOpened => {
                self.clear_socket_cache(socket);
                warn!("binance ticker ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self, socket: TickerSocket) {
        let symbols: Vec<String> = self
            .subscriptions
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        self.spawn_subscribe_batches(socket, &symbols);
    }

    fn on_disconnected(&self, reason: &str, socket: TickerSocket) {
        debug!(%reason, "binance ticker ws disconnected");
        self.clear_socket_cache(socket);
    }

    fn clear_socket_cache(&self, socket: TickerSocket) {
        match socket {
            TickerSocket::Book => self.books.clear(),
            TickerSocket::Stats => self.tickers.clear(),
        }
    }

    fn on_text(&self, text: &str) {
        match parse_ticker_update(text) {
            Some(BinanceTickerUpdate::Ticker(item)) => {
                self.tickers.insert(
                    item.symbol.to_ascii_lowercase(),
                    CachedTicker {
                        item,
                        cached_at_ms: now_ms(),
                    },
                );
            }
            Some(BinanceTickerUpdate::Book(item)) => {
                let key = item.symbol.to_ascii_lowercase();
                if self.subscriptions.contains_key(&key) {
                    self.books.insert(
                        key,
                        CachedBook {
                            item,
                            cached_at_ms: now_ms(),
                        },
                    );
                }
            }
            None => {}
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
        for chunk in pruned.chunks(SUBSCRIBE_BATCH_SIZE) {
            for symbol in chunk {
                self.tickers.remove(symbol);
                self.books.remove(symbol);
            }
            self.spawn_subscription(TickerSocket::Book, "UNSUBSCRIBE", chunk.to_vec());
            self.spawn_subscription(TickerSocket::Stats, "UNSUBSCRIBE", chunk.to_vec());
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "binance ticker ws idle prune"
            );
        }
        if self.subscriptions.is_empty() {
            self.book_manager.suspend_scope("perp-ticker").await;
            self.stats_manager.suspend_scope("perp-ticker").await;
            if !self.subscriptions.is_empty() {
                self.book_manager.activate_scope("perp-ticker");
                self.stats_manager.activate_scope("perp-ticker");
            }
        }
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

fn stream_symbol(symbol: &str) -> String {
    super::binance_format::usdm_stream_symbol(symbol)
}

fn book_subscription_payload(method: &str, symbols: &[String], request_id: u64) -> String {
    let params: Vec<String> = symbols
        .iter()
        .map(|symbol| format!("{symbol}@bookTicker"))
        .collect();
    json!({
        "method": method,
        "params": params,
        "id": request_id,
    })
    .to_string()
}

fn stats_subscription_payload(method: &str, symbols: &[String], request_id: u64) -> String {
    let params: Vec<String> = symbols
        .iter()
        .map(|symbol| format!("{symbol}@ticker"))
        .collect();
    json!({
        "method": method,
        "params": params,
        "id": request_id,
    })
    .to_string()
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

fn parse_ticker_update(text: &str) -> Option<BinanceTickerUpdate> {
    let kind: EventKind = serde_json::from_str(text).ok()?;
    match kind.event_type.as_deref()? {
        EVENT_24H_TICKER => serde_json::from_str(text)
            .ok()
            .map(BinanceTickerUpdate::Ticker),
        EVENT_BOOK_TICKER => serde_json::from_str(text)
            .ok()
            .map(BinanceTickerUpdate::Book),
        _ => None,
    }
}

/// 24hrTicker 首帧前只提供实时 BBO；0 量会让流动性门槛保持保守。
fn assemble_from_book(book: &BookTickerUpdate) -> Option<TickerInfo> {
    let bid = parse_positive(&book.bid_price)?;
    let ask = parse_positive(&book.ask_price)?;
    let timestamp = [book.event_time, book.transaction_time]
        .into_iter()
        .max()
        .unwrap_or_default();
    Some(TickerInfo {
        symbol: strip_common_suffixes(&book.symbol),
        exchange: EXCHANGE.into(),
        bid,
        ask,
        last: (bid + ask) / 2.0,
        volume_24h: 0.0,
        timestamp: if timestamp == 0 { now_ms() } else { timestamp },
    })
}

fn parse_ticker(ticker: &Ticker24hUpdate, book: Option<&BookTickerUpdate>) -> Option<TickerInfo> {
    let timestamp = [
        ticker.event_time,
        ticker.close_time,
        book.map_or(0, |row| row.event_time),
        book.map_or(0, |row| row.transaction_time),
    ]
    .into_iter()
    .max()
    .unwrap_or_default();
    let book_bbo = book.and_then(|row| {
        Some((
            parse_positive(&row.bid_price)?,
            parse_positive(&row.ask_price)?,
        ))
    });
    let (bid, ask) = match book_bbo {
        Some(bbo) => bbo,
        None => (
            parse_positive(&ticker.bid_price)?,
            parse_positive(&ticker.ask_price)?,
        ),
    };
    let last = parse_positive(&ticker.last_price)?;
    Some(TickerInfo {
        symbol: strip_common_suffixes(&ticker.symbol),
        exchange: EXCHANGE.into(),
        bid,
        ask,
        last,
        volume_24h: parse_f64(&ticker.quote_volume),
        timestamp: if timestamp == 0 { now_ms() } else { timestamp },
    })
}

fn parse_f64(value: &str) -> f64 {
    value.parse().unwrap_or(0.0)
}

fn parse_positive(value: &str) -> Option<f64> {
    value
        .parse()
        .ok()
        .filter(|value: &f64| value.is_finite() && *value > f64::EPSILON)
}

#[derive(Debug)]
enum BinanceTickerUpdate {
    Ticker(Ticker24hUpdate),
    Book(BookTickerUpdate),
}

#[derive(Debug, Deserialize)]
struct EventKind {
    #[serde(default, rename = "e")]
    event_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Ticker24hUpdate {
    #[serde(default, rename = "E")]
    event_time: i64,
    #[serde(default, rename = "s")]
    symbol: String,
    #[serde(default, rename = "c")]
    last_price: String,
    #[serde(default, rename = "q")]
    quote_volume: String,
    #[serde(default, rename = "b")]
    bid_price: String,
    #[serde(default, rename = "a")]
    ask_price: String,
    #[serde(default, rename = "C")]
    close_time: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct BookTickerUpdate {
    #[serde(default, rename = "E")]
    event_time: i64,
    #[serde(default, rename = "T")]
    transaction_time: i64,
    #[serde(default, rename = "s")]
    symbol: String,
    #[serde(default, rename = "b")]
    bid_price: String,
    #[serde(default, rename = "a")]
    ask_price: String,
}

#[cfg(test)]
#[path = "binance_ws_ticker_tests.rs"]
mod tests;
