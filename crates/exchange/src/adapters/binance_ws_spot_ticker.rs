//! Binance Spot public ticker WebSocket subscriber.
//!
//! Exact watched markets use `<symbol>@ticker` plus realtime
//! `<symbol>@bookTicker`. Full-market discovery stays on Binance's native WS API
//! snapshot because current all-market push streams do not carry a complete BBO.
//! Execution depth remains deferred until preview/build.
//!
//! Official docs:
//! - WebSocket base / live subscribe: <https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams>
//! - 24h ticker: <https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#individual-symbol-ticker-streams>
//! - Realtime BBO: <https://developers.binance.com/docs/binance-spot-api-docs/web-socket-streams#individual-symbol-book-ticker-streams>

use super::binance_config::BinanceConfig;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::SpotTick;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const VENUE: &str = "binance";
const WS_NAME: &str = "binance:spot";
const WS_URL: &str = "wss://stream.binance.com:9443/stream";
const EVENT_24H_TICKER: &str = "24hrTicker";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const SUBSCRIPTION_ACK_WAIT_MS: i64 = 3_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const CONTROL_FLUSH_INTERVAL: Duration = Duration::from_millis(100);
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const EXPLICIT_STREAM_LIMIT: usize = 256;
const EVENT_BUFFER_CAPACITY: usize = 8_192;

static SHARED_STREAM: OnceLock<Arc<SpotTickerStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
    sent_at_ms: Option<i64>,
    acknowledged_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedTicker {
    item: SpotTicker24hUpdate,
    cached_at_ms: i64,
}

#[derive(Debug, Clone)]
struct CachedBook {
    item: SpotBookTickerUpdate,
    cached_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct SpotTickerStream {
    manager: Arc<WsManager>,
    tickers: Arc<DashMap<String, CachedTicker>>,
    books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    pending_subscriptions: Arc<DashMap<u64, Vec<String>>>,
    subscription_problems: Arc<DashMap<String, String>>,
    pending_unsubscribes: Arc<DashMap<String, ()>>,
    next_request_id: AtomicU64,
}

pub(crate) fn snapshot_spot_ticks(
    config: &BinanceConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<SpotTick>> {
    let symbols = symbols.and_then(stream_symbols)?;
    if symbols.len() >= EXPLICIT_STREAM_LIMIT {
        return None;
    }
    let stream = enabled_stream(config)?;
    stream.touch_many(&symbols);
    let rows = stream.snapshot(&symbols);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn spot_problem(config: &BinanceConfig, symbol: &str) -> Option<String> {
    let stream = enabled_stream(config)?;
    if let Some(problem) = stream.manager.connection_problem() {
        return Some(format!("连接失败：{problem}"));
    }
    stream.subscription_problem(symbol)
}

fn enabled_stream(config: &BinanceConfig) -> Option<Arc<SpotTickerStream>> {
    if config.testnet || config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(SpotTickerStream::new)))
}

impl SpotTickerStream {
    fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new_with_event_capacity(
                WsConfig {
                    url: WS_URL.into(),
                    exchange: WS_NAME.into(),
                    heartbeat_interval: Duration::from_secs(20),
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
        );
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            tickers: Arc::new(DashMap::new()),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            pending_subscriptions: Arc::new(DashMap::new()),
            subscription_problems: Arc::new(DashMap::new()),
            pending_unsubscribes: Arc::new(DashMap::new()),
            next_request_id: AtomicU64::new(1),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "binance spot ticker ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_subscription_control());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch_many(&self, symbols: &[String]) {
        self.manager.activate_scope("spot-ticker");
        let now = now_ms();
        for symbol in symbols {
            self.pending_unsubscribes.remove(symbol);
            self.subscriptions
                .entry(symbol.clone())
                .and_modify(|state| state.last_touched_ms = now)
                .or_insert(SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: false,
                    sent_at_ms: None,
                    acknowledged_on_current_connection: false,
                });
        }
    }

    fn subscription_problem(&self, symbol: &str) -> Option<String> {
        let symbol = stream_symbol(symbol)?;
        if let Some(problem) = self.subscription_problems.get(&symbol) {
            return Some(format!("订阅被交易所拒绝：{}", problem.value()));
        }
        let state = self.subscriptions.get(&symbol)?;
        if state.acknowledged_on_current_connection {
            return Some("订阅已确认，正在等待首个最优买卖价变化".to_owned());
        }
        let Some(sent_at_ms) = state.sent_at_ms else {
            return Some("正在建立连接并发送精确交易对订阅".to_owned());
        };
        let age_ms = now_ms().saturating_sub(sent_at_ms);
        Some(if age_ms >= SUBSCRIPTION_ACK_WAIT_MS {
            format!("订阅请求已发送，但 {age_ms}ms 内未收到交易所确认")
        } else {
            "订阅请求已发送，等待交易所确认".to_owned()
        })
    }

    fn snapshot(&self, symbols: &[String]) -> Vec<SpotTick> {
        let now = now_ms();
        symbols
            .iter()
            .filter_map(|symbol| self.latest_at(symbol, now))
            .collect()
    }

    fn latest_at(&self, symbol: &str, now: i64) -> Option<SpotTick> {
        let ticker = self.tickers.get(symbol)?;
        if !is_fresh(ticker.cached_at_ms, now) {
            return None;
        }
        let book = self
            .books
            .get(symbol)
            .filter(|book| is_fresh(book.cached_at_ms, now));
        let received_at_ms = book
            .as_ref()
            .map_or(ticker.cached_at_ms, |book| book.cached_at_ms);
        parse_spot_tick(
            &ticker.item,
            book.as_ref().map(|book| &book.item),
            received_at_ms,
        )
    }

    async fn run_subscription_control(self: Arc<Self>) {
        let mut tick = tokio::time::interval(CONTROL_FLUSH_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            if !self.manager.is_connected().await {
                continue;
            }
            self.flush_explicit_subscriptions().await;
            self.flush_unsubscribes().await;
        }
    }

    async fn flush_explicit_subscriptions(&self) {
        let symbols: Vec<String> = self
            .subscriptions
            .iter()
            .filter(|entry| !entry.value().sent_on_current_connection)
            .map(|entry| entry.key().clone())
            .take(SUBSCRIBE_BATCH_SIZE)
            .collect();
        if symbols.is_empty() {
            return;
        }
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        let payload = subscription_payload("SUBSCRIBE", &symbols, request_id);
        self.pending_subscriptions
            .insert(request_id, symbols.clone());
        mark_sent(&self.subscriptions, &symbols, now_ms());
        match self.manager.send(Message::Text(payload)).await {
            Ok(()) => {}
            Err(error) => {
                self.pending_subscriptions.remove(&request_id);
                mark_unsent(&self.subscriptions, &symbols);
                warn!(
                    count = symbols.len(),
                    error = %error,
                    "binance spot ticker ws subscription send failed"
                );
            }
        }
    }

    async fn flush_unsubscribes(&self) {
        let symbols: Vec<String> = self
            .pending_unsubscribes
            .iter()
            .map(|entry| entry.key().clone())
            .take(SUBSCRIBE_BATCH_SIZE)
            .collect();
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
                count = symbols.len(),
                error = %error,
                "binance spot ticker ws unsubscribe failed"
            ),
        }
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
                debug!(
                    missed,
                    "binance spot ticker ws skipped intermediate complete snapshots"
                );
                true
            }
            Err(RecvError::Closed) => {
                self.clear_cache();
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
                self.clear_cache();
                warn!("binance spot ticker ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        self.pending_subscriptions.clear();
        self.subscription_problems.clear();
        self.pending_unsubscribes.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
            entry.value_mut().sent_at_ms = None;
            entry.value_mut().acknowledged_on_current_connection = false;
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "binance spot ticker ws disconnected");
        self.clear_cache();
        self.pending_subscriptions.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
            entry.value_mut().sent_at_ms = None;
            entry.value_mut().acknowledged_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.tickers.clear();
        self.books.clear();
    }

    fn on_text(&self, text: &str) {
        if let Some(response) = parse_control_response(text) {
            self.on_control_response(&response);
            return;
        }
        for update in parse_ticker_updates(text) {
            match update {
                BinanceSpotTickerUpdate::Ticker(item) => {
                    let symbol = item.symbol.to_ascii_lowercase();
                    if !self.subscriptions.contains_key(&symbol) {
                        continue;
                    }
                    self.tickers.insert(
                        symbol,
                        CachedTicker {
                            item,
                            cached_at_ms: now_ms(),
                        },
                    );
                }
                BinanceSpotTickerUpdate::Book(item) => {
                    let symbol = item.symbol.to_ascii_lowercase();
                    if !self.subscriptions.contains_key(&symbol) {
                        continue;
                    }
                    self.books.insert(
                        symbol,
                        CachedBook {
                            item,
                            cached_at_ms: now_ms(),
                        },
                    );
                }
            }
        }
    }

    fn on_control_response(&self, response: &ControlResponse) {
        let Some((_, symbols)) = self.pending_subscriptions.remove(&response.id) else {
            return;
        };
        if let Some(problem) = response.problem() {
            for symbol in symbols {
                self.subscription_problems
                    .insert(symbol.clone(), problem.clone());
                if let Some(mut state) = self.subscriptions.get_mut(&symbol) {
                    state.acknowledged_on_current_connection = false;
                }
            }
            return;
        }
        for symbol in symbols {
            self.subscription_problems.remove(&symbol);
            if let Some(mut state) = self.subscriptions.get_mut(&symbol) {
                state.acknowledged_on_current_connection = true;
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
        for symbol in &pruned {
            self.tickers.remove(symbol);
            self.books.remove(symbol);
            self.subscription_problems.remove(symbol);
            self.pending_unsubscribes.insert(symbol.clone(), ());
        }
        self.pending_subscriptions.retain(|_, symbols| {
            symbols
                .iter()
                .any(|symbol| self.subscriptions.contains_key(symbol))
        });
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "binance spot ticker ws idle prune"
            );
        }
        if self.subscriptions.is_empty() {
            self.pending_unsubscribes.clear();
            self.manager.suspend_scope("spot-ticker").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("spot-ticker");
            }
        }
    }
}

fn mark_sent(
    subscriptions: &DashMap<String, SubscriptionState>,
    symbols: &[String],
    sent_at_ms: i64,
) {
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = true;
            state.sent_at_ms = Some(sent_at_ms);
            state.acknowledged_on_current_connection = false;
        }
    }
}

fn mark_unsent(subscriptions: &DashMap<String, SubscriptionState>, symbols: &[String]) {
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = false;
            state.sent_at_ms = None;
            state.acknowledged_on_current_connection = false;
        }
    }
}

fn stream_symbols(symbols: &[String]) -> Option<Vec<String>> {
    if symbols.is_empty() {
        return None;
    }
    let normalized: Vec<String> = symbols
        .iter()
        .map(|symbol| stream_symbol(symbol))
        .collect::<Option<Vec<_>>>()?;
    (!normalized.is_empty()).then_some(normalized)
}

fn stream_symbol(symbol: &str) -> Option<String> {
    crate::spot::compact_pair_symbol(symbol).map(|symbol| symbol.to_ascii_lowercase())
}

fn subscription_payload(method: &str, symbols: &[String], request_id: u64) -> String {
    let params: Vec<String> = symbols
        .iter()
        .flat_map(|symbol| [format!("{symbol}@ticker"), format!("{symbol}@bookTicker")])
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

#[cfg(test)]
fn parse_ticker_update(text: &str) -> Option<BinanceSpotTickerUpdate> {
    parse_ticker_updates(text).into_iter().next()
}

fn parse_ticker_updates(text: &str) -> Vec<BinanceSpotTickerUpdate> {
    let payload = serde_json::from_str::<CombinedTickerEnvelope>(text)
        .map(|envelope| envelope.data)
        .or_else(|_| serde_json::from_str::<TickerPayload>(text));
    match payload {
        Ok(TickerPayload::Many(rows)) => rows
            .into_iter()
            .filter(|row| row.event_type == EVENT_24H_TICKER)
            .map(BinanceSpotTickerUpdate::Ticker)
            .collect(),
        Ok(TickerPayload::One(value)) => parse_ticker_value(value).into_iter().collect(),
        Err(_) => Vec::new(),
    }
}

fn parse_control_response(text: &str) -> Option<ControlResponse> {
    let value: Value = serde_json::from_str(text).ok()?;
    let id = value.get("id")?.as_u64()?;
    Some(ControlResponse {
        id,
        code: value.get("code").and_then(Value::as_i64),
        message: value.get("msg").and_then(Value::as_str).map(str::to_owned),
    })
}

fn parse_ticker_value(value: Value) -> Option<BinanceSpotTickerUpdate> {
    let kind: EventKind = serde_json::from_value(value.clone()).ok()?;
    match kind.event_type.as_deref() {
        Some(EVENT_24H_TICKER) => serde_json::from_value(value)
            .ok()
            .map(BinanceSpotTickerUpdate::Ticker),
        _ if kind.update_id.is_some() => serde_json::from_value(value)
            .ok()
            .map(BinanceSpotTickerUpdate::Book),
        _ => None,
    }
}

fn parse_spot_tick(
    ticker: &SpotTicker24hUpdate,
    book: Option<&SpotBookTickerUpdate>,
    received_at_ms: i64,
) -> Option<SpotTick> {
    let (base, quote) = crate::spot::suffix_pair(&ticker.symbol)?;
    let exchange_ts_ms = [ticker.event_time, ticker.close_time]
        .into_iter()
        .max()
        .filter(|ms| *ms > 0);
    let (bid, ask, bid_size, ask_size) = book.map_or(
        (
            ticker.bid_price.as_str(),
            ticker.ask_price.as_str(),
            ticker.bid_qty.as_str(),
            ticker.ask_qty.as_str(),
        ),
        |book| {
            (
                book.bid_price.as_str(),
                book.ask_price.as_str(),
                book.bid_qty.as_str(),
                book.ask_qty.as_str(),
            )
        },
    );
    let mut tick = crate::spot::spot_tick(crate::spot::SpotFields {
        venue: VENUE,
        base: &base,
        quote: &quote,
        bid,
        ask,
        last: &ticker.last_price,
        bid_size: Some(bid_size),
        ask_size: Some(ask_size),
        volume_24h: &ticker.quote_volume,
        exchange_ts_ms,
    })?;
    tick.received_at_ms = received_at_ms;
    Some(tick)
}

#[derive(Debug)]
enum BinanceSpotTickerUpdate {
    Ticker(SpotTicker24hUpdate),
    Book(SpotBookTickerUpdate),
}

#[derive(Debug, PartialEq, Eq)]
struct ControlResponse {
    id: u64,
    code: Option<i64>,
    message: Option<String>,
}

impl ControlResponse {
    fn problem(&self) -> Option<String> {
        match (self.code, self.message.as_deref()) {
            (Some(code), Some(message)) => Some(format!("code {code} · {message}")),
            (Some(code), None) => Some(format!("code {code}")),
            (None, Some(message)) => Some(message.to_owned()),
            (None, None) => None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CombinedTickerEnvelope {
    data: TickerPayload,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TickerPayload {
    Many(Vec<SpotTicker24hUpdate>),
    One(Value),
}

#[derive(Debug, Deserialize)]
struct EventKind {
    #[serde(default, rename = "e")]
    event_type: Option<String>,
    #[serde(default, rename = "u")]
    update_id: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
struct SpotTicker24hUpdate {
    #[serde(default, rename = "e")]
    event_type: String,
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
    #[serde(default, rename = "B")]
    bid_qty: String,
    #[serde(default, rename = "a")]
    ask_price: String,
    #[serde(default, rename = "A")]
    ask_qty: String,
    #[serde(default, rename = "C")]
    close_time: i64,
}

#[derive(Debug, Clone, Deserialize)]
struct SpotBookTickerUpdate {
    #[serde(default, rename = "s")]
    symbol: String,
    #[serde(default, rename = "b")]
    bid_price: String,
    #[serde(default, rename = "B")]
    bid_qty: String,
    #[serde(default, rename = "a")]
    ask_price: String,
    #[serde(default, rename = "A")]
    ask_qty: String,
}

#[cfg(test)]
#[path = "binance_ws_spot_ticker_tests.rs"]
mod tests;
