//! Bybit V5 public Spot WebSocket ticker subscriber.
//!
//! `tickers.<symbol>` for Spot does not expose best bid / ask. `SpotTick`
//! therefore combines two official public topics:
//! - `tickers.<symbol>` for last price and 24h turnover
//! - `orderbook.1.<symbol>` for best bid / ask and sizes
//!
//! Official docs:
//! - Public WS connection: <https://bybit-exchange.github.io/docs/v5/ws/connect>
//! - Ticker topic: <https://bybit-exchange.github.io/docs/v5/websocket/public/ticker>
//! - Orderbook topic: <https://bybit-exchange.github.io/docs/v5/websocket/public/orderbook>

use super::bybit_config::BybitConfig;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use shared_types::{OrderBookInfo, SpotTick};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "bybit";
const WS_URL: &str = "wss://stream.bybit.com/v5/public/spot";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_BATCH_SIZE: usize = 5;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const EVENT_BUFFER_CAPACITY: usize = 8_192;

static SHARED_STREAM: OnceLock<Arc<SpotTickerStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
    depth_requested: bool,
    depth_sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedTicker {
    item: SpotTickerUpdate,
    cached_at_ms: i64,
    data_timestamp_ms: i64,
}

#[derive(Debug, Clone)]
struct CachedBook {
    item: SpotBookUpdate,
    cached_at_ms: i64,
    data_timestamp_ms: i64,
    update_id: i64,
}

#[derive(Debug)]
pub(crate) struct SpotTickerStream {
    manager: Arc<WsManager>,
    tickers: Arc<DashMap<String, CachedTicker>>,
    books: Arc<DashMap<String, CachedBook>>,
    depth_books: Arc<DashMap<String, CachedBook>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

pub(crate) fn snapshot_spot_ticks(
    config: &BybitConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<SpotTick>> {
    let symbols = symbols.and_then(stream_symbols)?;
    let stream = enabled_stream(config)?;
    stream.touch_many(&symbols);
    let rows = stream.snapshot(&symbols);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn spot_connection_problem(config: &BybitConfig) -> Option<String> {
    enabled_stream(config)?.manager.connection_problem()
}

pub(crate) fn latest_spot_orderbook(
    config: &BybitConfig,
    symbol: &str,
    depth: u32,
) -> Option<OrderBookInfo> {
    let symbol = crate::spot::compact_pair_symbol(symbol)?;
    let stream = enabled_stream(config)?;
    stream.touch_depth(&symbol);
    stream.latest_depth(&symbol, depth)
}

fn enabled_stream(config: &BybitConfig) -> Option<Arc<SpotTickerStream>> {
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
                    exchange: EXCHANGE.into(),
                    heartbeat_interval: Duration::from_secs(20),
                    heartbeat: WsHeartbeat::Text(r#"{"op":"ping"}"#.into()),
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
            depth_books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "bybit spot ticker ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch_many(&self, symbols: &[String]) {
        self.manager.activate_scope("spot-market");
        let now = now_ms();
        let mut new_symbols = Vec::new();
        for symbol in symbols {
            let mut needs_subscribe = false;
            self.subscriptions
                .entry(symbol.clone())
                .and_modify(|state| state.last_touched_ms = now)
                .or_insert_with(|| {
                    needs_subscribe = true;
                    SubscriptionState {
                        last_touched_ms: now,
                        sent_on_current_connection: false,
                        depth_requested: false,
                        depth_sent_on_current_connection: false,
                    }
                });
            if needs_subscribe {
                new_symbols.push(symbol.clone());
            }
        }
        self.spawn_subscribe_batches(&new_symbols);
    }

    fn touch_depth(&self, symbol: &str) {
        self.manager.activate_scope("spot-market");
        let now = now_ms();
        let mut needs_subscribe = false;
        self.subscriptions
            .entry(symbol.to_owned())
            .and_modify(|state| {
                state.last_touched_ms = now;
                if !state.depth_requested {
                    state.depth_requested = true;
                    needs_subscribe = true;
                }
            })
            .or_insert_with(|| {
                needs_subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: false,
                    depth_requested: true,
                    depth_sent_on_current_connection: false,
                }
            });
        if needs_subscribe {
            self.spawn_depth_subscription("subscribe", vec![symbol.to_owned()]);
        }
    }

    fn latest_depth(&self, symbol: &str, depth: u32) -> Option<OrderBookInfo> {
        let cached = self.depth_books.get(symbol)?;
        if !is_fresh(cached.cached_at_ms, now_ms()) {
            return None;
        }
        orderbook_from_cached(&cached, depth)
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
        let book = self.books.get(symbol)?;
        if !is_fresh(ticker.cached_at_ms, now) || !is_fresh(book.cached_at_ms, now) {
            return None;
        }
        parse_spot_tick(&ticker, &book)
    }

    fn spawn_subscribe_batches(&self, symbols: &[String]) {
        for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
            self.spawn_subscription("subscribe", batch.to_vec());
        }
    }

    fn spawn_subscription(&self, op: &'static str, symbols: Vec<String>) {
        if symbols.is_empty() {
            return;
        }
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                debug!(
                    count = symbols.len(),
                    "bybit spot ticker ws not connected yet; subscribe resumes after reconnect"
                );
                return;
            }
            let payload = channel_payload(op, &symbols);
            match manager.send(Message::Text(payload)).await {
                Ok(()) if op == "subscribe" => mark_sent(&subscriptions, &symbols),
                Ok(()) => {}
                Err(error) => warn!(
                    op,
                    count = symbols.len(),
                    error = %error,
                    "bybit spot ticker ws subscription send failed"
                ),
            }
        });
    }

    fn spawn_depth_subscription(&self, op: &'static str, symbols: Vec<String>) {
        if symbols.is_empty() {
            return;
        }
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                return;
            }
            match manager
                .send(Message::Text(depth_channel_payload(op, &symbols)))
                .await
            {
                Ok(()) if op == "subscribe" => mark_depth_sent(&subscriptions, &symbols),
                Ok(()) => {}
                Err(error) => warn!(
                    op,
                    count = symbols.len(),
                    error = %error,
                    "bybit spot depth ws subscription send failed"
                ),
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
                warn!(missed, "bybit spot ticker ws broadcast receiver lagged");
                self.clear_market_cache();
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
                self.clear_market_cache();
                warn!("bybit spot ticker ws circuit opened");
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
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
            entry.value_mut().depth_sent_on_current_connection = false;
        }
        self.spawn_subscribe_batches(&symbols);
        let depth_symbols = self
            .subscriptions
            .iter()
            .filter(|entry| entry.depth_requested)
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for batch in depth_symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
            self.spawn_depth_subscription("subscribe", batch.to_vec());
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "bybit spot ticker ws disconnected");
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
            entry.value_mut().depth_sent_on_current_connection = false;
        }
        self.clear_market_cache();
    }

    fn on_text(&self, text: &str) {
        if let Some(row) = parse_ticker_update(text) {
            let symbol = row.item.symbol.clone();
            if self.subscriptions.contains_key(&symbol) {
                self.tickers.insert(symbol, row);
            }
        }
        if let Some(row) = parse_book_update(text) {
            let symbol = row.item.symbol.clone();
            if self.subscriptions.contains_key(&symbol) {
                if row.depth > 1 {
                    ingest_book_update(&self.depth_books, row);
                } else {
                    ingest_book_update(&self.books, row);
                }
            }
        }
    }

    fn clear_market_cache(&self) {
        self.tickers.clear();
        self.books.clear();
        self.depth_books.clear();
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
                pruned.push((symbol.clone(), state.depth_requested));
                false
            } else {
                true
            }
        });
        for (symbol, _) in &pruned {
            self.tickers.remove(symbol);
            self.books.remove(symbol);
            self.depth_books.remove(symbol);
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "bybit spot ticker ws idle prune"
            );
            self.spawn_unsubscribe_batches(pruned);
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("spot-market").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("spot-market");
            }
        }
    }

    fn spawn_unsubscribe_batches(&self, pruned: Vec<(String, bool)>) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            let symbols: Vec<String> = pruned.iter().map(|(symbol, _)| symbol.clone()).collect();
            for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
                if let Err(error) = manager
                    .send(Message::Text(channel_payload("unsubscribe", batch)))
                    .await
                {
                    debug!(
                        remaining = symbols.len(),
                        error = %error,
                        "bybit spot ticker ws idle unsubscribe stopped after disconnect"
                    );
                    return;
                }
            }
            let depth_symbols: Vec<String> = pruned
                .into_iter()
                .filter_map(|(symbol, requested)| requested.then_some(symbol))
                .collect();
            for batch in depth_symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
                if let Err(error) = manager
                    .send(Message::Text(depth_channel_payload("unsubscribe", batch)))
                    .await
                {
                    debug!(
                        remaining = depth_symbols.len(),
                        error = %error,
                        "bybit spot depth ws idle unsubscribe stopped after disconnect"
                    );
                    return;
                }
            }
        });
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

fn mark_sent(subscriptions: &DashMap<String, SubscriptionState>, symbols: &[String]) {
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = true;
        }
    }
}

fn mark_depth_sent(subscriptions: &DashMap<String, SubscriptionState>, symbols: &[String]) {
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.depth_sent_on_current_connection = true;
        }
    }
}

fn stream_symbols(symbols: &[String]) -> Option<Vec<String>> {
    if symbols.is_empty() {
        return None;
    }
    // filter_map：单个不可映射符号（如无现货市场的股票代币）只缺席自己，
    // 不再把整个 venue 的现货 WS 触达打回 None。
    let mut normalized = symbols
        .iter()
        .filter_map(|symbol| crate::spot::compact_pair_symbol(symbol))
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    (!normalized.is_empty()).then_some(normalized)
}

fn channel_payload(op: &str, symbols: &[String]) -> String {
    let args: Vec<String> = symbols
        .iter()
        .flat_map(|symbol| [format!("tickers.{symbol}"), format!("orderbook.1.{symbol}")])
        .collect();
    json!({ "op": op, "args": args }).to_string()
}

fn depth_channel_payload(op: &str, symbols: &[String]) -> String {
    let args = symbols
        .iter()
        .map(|symbol| format!("orderbook.50.{symbol}"))
        .collect::<Vec<_>>();
    json!({ "op": op, "args": args }).to_string()
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

fn parse_ticker_update(text: &str) -> Option<CachedTicker> {
    let envelope: TickerEnvelope = serde_json::from_str(text).ok()?;
    if !envelope.topic.starts_with("tickers.") {
        return None;
    }
    let now = now_ms();
    let item = envelope.data.into_first()?;
    let data_timestamp_ms = envelope.ts.unwrap_or(now);
    (!item.symbol.is_empty() && !item.last_price.is_empty() && !item.turnover_24h.is_empty())
        .then_some(CachedTicker {
            item,
            cached_at_ms: now,
            data_timestamp_ms,
        })
}

fn parse_book_update(text: &str) -> Option<ParsedBookUpdate> {
    let envelope: BookEnvelope = serde_json::from_str(text).ok()?;
    let depth = book_topic_depth(&envelope.topic)?;
    if depth != 1 && depth != 50 {
        return None;
    }
    let now = now_ms();
    let item = SpotBookUpdate::from_data(envelope.data)?;
    let update_id = item.update_id;
    (update_id > 0).then_some(ParsedBookUpdate {
        item,
        cached_at_ms: now,
        data_timestamp_ms: envelope.ts.unwrap_or(now),
        depth,
        update_id,
        snapshot: envelope.kind == "snapshot" || update_id == 1,
    })
}

fn ingest_book_update(cache: &DashMap<String, CachedBook>, update: ParsedBookUpdate) {
    let symbol = update.item.symbol.clone();
    if update.snapshot {
        if update.item.bids.is_empty() || update.item.asks.is_empty() {
            cache.remove(&symbol);
            return;
        }
        cache.insert(symbol, update.into_cached());
        return;
    }

    let Some(mut cached) = cache.get_mut(&symbol) else {
        return;
    };
    if update.update_id <= cached.update_id {
        return;
    }
    apply_book_delta(&mut cached.item, &update.item);
    cached.cached_at_ms = update.cached_at_ms;
    cached.data_timestamp_ms = update.data_timestamp_ms;
    cached.update_id = update.update_id;
}

fn apply_book_delta(book: &mut SpotBookUpdate, delta: &SpotBookUpdate) {
    update_string_side(&mut book.bids, &delta.bids, true);
    update_string_side(&mut book.asks, &delta.asks, false);
}

fn update_string_side(side: &mut Vec<[String; 2]>, updates: &[[String; 2]], descending: bool) {
    for [price, size] in updates {
        let Ok(price_value) = price.parse::<f64>() else {
            continue;
        };
        let Ok(size_value) = size.parse::<f64>() else {
            continue;
        };
        if let Some(index) = side.iter().position(|level| {
            level[0]
                .parse::<f64>()
                .is_ok_and(|value| (value - price_value).abs() < f64::EPSILON)
        }) {
            if size_value == 0.0 {
                side.remove(index);
            } else if size_value > 0.0 {
                side[index] = [price.clone(), size.clone()];
            }
        } else if size_value > 0.0 {
            side.push([price.clone(), size.clone()]);
        }
    }
    side.sort_by(|left, right| {
        let left = left[0].parse::<f64>().unwrap_or_default();
        let right = right[0].parse::<f64>().unwrap_or_default();
        if descending {
            right.total_cmp(&left)
        } else {
            left.total_cmp(&right)
        }
    });
    side.truncate(50);
}

fn book_topic_depth(topic: &str) -> Option<u32> {
    let mut parts = topic.split('.');
    if parts.next()? != "orderbook" {
        return None;
    }
    parts.next()?.parse().ok()
}

fn parse_spot_tick(ticker: &CachedTicker, book: &CachedBook) -> Option<SpotTick> {
    let (base, quote) = crate::spot::suffix_pair(&ticker.item.symbol)?;
    let timestamp = ticker.data_timestamp_ms.max(book.data_timestamp_ms);
    let [bid_price, bid_size] = book.item.bids.first()?;
    let [ask_price, ask_size] = book.item.asks.first()?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: EXCHANGE,
        base: &base,
        quote: &quote,
        bid: bid_price,
        ask: ask_price,
        last: &ticker.item.last_price,
        bid_size: Some(bid_size),
        ask_size: Some(ask_size),
        volume_24h: &ticker.item.turnover_24h,
        exchange_ts_ms: Some(timestamp),
    })
}

fn orderbook_from_cached(cached: &CachedBook, depth: u32) -> Option<OrderBookInfo> {
    let cap = usize::try_from(depth.max(1)).ok()?;
    let bids = parse_levels(&cached.item.bids, cap);
    let asks = parse_levels(&cached.item.asks, cap);
    if bids.is_empty() || asks.is_empty() {
        return None;
    }
    Some(OrderBookInfo {
        symbol: crate::spot::native_pair_symbol(&cached.item.symbol, '/')?,
        exchange: EXCHANGE.into(),
        bids,
        asks,
        timestamp: cached.data_timestamp_ms,
    })
}

fn parse_levels(levels: &[[String; 2]], cap: usize) -> Vec<[f64; 2]> {
    levels
        .iter()
        .take(cap)
        .filter_map(|[price, size]| Some([price.parse().ok()?, size.parse().ok()?]))
        .collect()
}

#[derive(Debug, Deserialize)]
struct TickerEnvelope {
    topic: String,
    data: TickerPayload,
    #[serde(default, deserialize_with = "de_opt_i64")]
    ts: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum TickerPayload {
    One(Box<SpotTickerUpdate>),
    Many(Vec<SpotTickerUpdate>),
}

impl TickerPayload {
    fn into_first(self) -> Option<SpotTickerUpdate> {
        match self {
            Self::One(item) => Some(*item),
            Self::Many(items) => items.into_iter().next(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct SpotTickerUpdate {
    symbol: String,
    #[serde(default, rename = "lastPrice")]
    last_price: String,
    #[serde(default, rename = "turnover24h")]
    turnover_24h: String,
}

#[derive(Debug, Deserialize)]
struct BookEnvelope {
    topic: String,
    #[serde(default, rename = "type")]
    kind: String,
    data: BookData,
    #[serde(default, deserialize_with = "de_opt_i64")]
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

#[derive(Debug)]
struct ParsedBookUpdate {
    item: SpotBookUpdate,
    cached_at_ms: i64,
    data_timestamp_ms: i64,
    depth: u32,
    update_id: i64,
    snapshot: bool,
}

impl ParsedBookUpdate {
    fn into_cached(self) -> CachedBook {
        CachedBook {
            item: self.item,
            cached_at_ms: self.cached_at_ms,
            data_timestamp_ms: self.data_timestamp_ms,
            update_id: self.update_id,
        }
    }
}

#[derive(Debug, Clone)]
struct SpotBookUpdate {
    symbol: String,
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
    update_id: i64,
}

impl SpotBookUpdate {
    fn from_data(data: BookData) -> Option<Self> {
        if data.symbol.is_empty() || (data.bids.is_empty() && data.asks.is_empty()) {
            return None;
        }
        Some(Self {
            symbol: data.symbol,
            bids: data.bids,
            asks: data.asks,
            update_id: data.update_id,
        })
    }
}

fn de_opt_i64<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de;
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Number(number) => number
            .as_i64()
            .ok_or_else(|| de::Error::custom("invalid integer"))
            .map(Some),
        serde_json::Value::String(text) => text.parse::<i64>().map(Some).map_err(de::Error::custom),
        _ => Err(de::Error::custom("expected number, string, or null")),
    }
}

#[cfg(test)]
#[path = "bybit_ws_spot_ticker_tests.rs"]
mod tests;
