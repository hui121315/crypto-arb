//! Bitget V3 / UTA Spot public WebSocket ticker subscriber.
//!
//! This stays separate from `bitget_uta_ws_ticker.rs` so futures and spot rows
//! with the same `BTCUSDT` symbol cannot overwrite each other.
//!
//! Official docs:
//! - WS public ticker: <https://www.bitget.com/api-doc/uta/websocket/public/Tickers-Channel>

use super::bitget_config::BitgetConfig;
use super::bitget_uta_config::{BitgetUtaCategory, BitgetUtaWsArgs, PROD_WS_PUBLIC};
use super::bitget_uta_ws_market::MarketStream;
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

const EXCHANGE: &str = "bitget";
const TOPIC: &str = "ticker";
const SPOT_INST_TYPE: &str = "spot";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const EVENT_BUFFER_CAPACITY: usize = 8_192;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

static SHARED_STREAM: OnceLock<Arc<SpotTickerStream>> = OnceLock::new();
static SHARED_BOOK_STREAM: OnceLock<Arc<MarketStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedTicker {
    item: SpotTickerItem,
    cached_at_ms: i64,
    data_timestamp_ms: i64,
}

#[derive(Debug)]
pub(crate) struct SpotTickerStream {
    manager: Arc<WsManager>,
    rows: Arc<DashMap<String, CachedTicker>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

pub(crate) fn snapshot_spot_ticks(
    config: &BitgetConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<SpotTick>> {
    let symbols = symbols.and_then(stream_symbols)?;
    let stream = enabled_stream(config)?;
    stream.touch_many(&symbols);
    let rows = stream.snapshot(&symbols);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn spot_connection_problem(config: &BitgetConfig) -> Option<String> {
    enabled_stream(config)?.manager.connection_problem()
}

pub(crate) fn latest_spot_orderbook(
    config: &BitgetConfig,
    symbol: &str,
    depth: u32,
) -> Option<OrderBookInfo> {
    if config.base_url_override.is_some() {
        return None;
    }
    let stream = SHARED_BOOK_STREAM.get_or_init(MarketStream::new_spot);
    stream.touch(symbol);
    stream.latest(symbol, depth)
}

fn enabled_stream(config: &BitgetConfig) -> Option<Arc<SpotTickerStream>> {
    if config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(SpotTickerStream::new)))
}

impl SpotTickerStream {
    fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new_with_event_capacity(
                WsConfig {
                    url: PROD_WS_PUBLIC.into(),
                    exchange: EXCHANGE.into(),
                    heartbeat_interval: Duration::from_secs(20),
                    heartbeat: WsHeartbeat::Text("ping".into()),
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
            rows: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "bitget uta spot ticker ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch_many(&self, symbols: &[String]) {
        self.manager.activate_scope("spot-ticker");
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
                    }
                });
            if needs_subscribe {
                new_symbols.push(symbol.clone());
            }
        }
        self.spawn_subscribe_batches(&new_symbols);
    }

    fn snapshot(&self, symbols: &[String]) -> Vec<SpotTick> {
        let now = now_ms();
        symbols
            .iter()
            .filter_map(|symbol| self.latest_at(symbol, now))
            .collect()
    }

    fn latest_at(&self, symbol: &str, now: i64) -> Option<SpotTick> {
        let row = self.rows.get(symbol)?;
        is_fresh(row.cached_at_ms, now).then(|| parse_spot_tick(&row))?
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
                    "bitget uta spot ticker ws not connected yet; subscribe resumes after reconnect"
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
                    "bitget uta spot ticker ws subscription send failed"
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
                // Every Bitget ticker push is a complete per-symbol snapshot.
                // Retaining still-fresh rows is correct and avoids a feedback
                // loop where cache clearing and WARN logging create more lag.
                debug!(
                    missed,
                    "bitget uta spot ticker ws skipped intermediate snapshots"
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
                warn!("bitget uta spot ticker ws circuit opened");
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
        }
        self.spawn_subscribe_batches(&symbols);
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "bitget uta spot ticker ws disconnected");
        self.clear_cache();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.rows.clear();
    }

    fn on_text(&self, text: &str) {
        for row in parse_ticker_update(text) {
            let symbol = row.item.symbol.clone();
            if self.subscriptions.contains_key(&symbol) {
                self.rows.insert(symbol, row);
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
            self.rows.remove(symbol);
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "bitget uta spot ticker ws idle prune"
            );
            self.spawn_unsubscribe_batches(pruned);
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("spot-ticker").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("spot-ticker");
            }
        }
    }

    fn spawn_unsubscribe_batches(&self, symbols: Vec<String>) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
                if let Err(error) = manager
                    .send(Message::Text(channel_payload("unsubscribe", batch)))
                    .await
                {
                    debug!(
                        count = symbols.len(),
                        error = %error,
                        "bitget uta spot ticker ws idle unsubscribe stopped after disconnect"
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

fn stream_symbols(symbols: &[String]) -> Option<Vec<String>> {
    if symbols.is_empty() {
        return None;
    }
    let normalized: Vec<String> = symbols
        .iter()
        .map(|symbol| crate::spot::compact_pair_symbol(symbol))
        .collect::<Option<Vec<_>>>()?;
    (!normalized.is_empty()).then_some(normalized)
}

fn channel_payload(op: &str, symbols: &[String]) -> String {
    let args: Vec<_> = symbols
        .iter()
        .map(|symbol| BitgetUtaWsArgs::new(BitgetUtaCategory::Spot, TOPIC, symbol))
        .collect();
    json!({ "op": op, "args": args }).to_string()
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

fn parse_ticker_update(text: &str) -> Vec<CachedTicker> {
    let Ok(envelope) = serde_json::from_str::<TickerEnvelope>(text) else {
        return Vec::new();
    };
    let Some(arg) = envelope.arg else {
        return Vec::new();
    };
    if arg.topic != TOPIC || arg.inst_type != SPOT_INST_TYPE {
        return Vec::new();
    }

    let now = now_ms();
    let data_timestamp_ms = envelope.ts.unwrap_or(now);
    envelope
        .data
        .into_iter()
        .filter_map(|mut item| {
            item.symbol = symbol_from_row(&item, &arg)?;
            Some(CachedTicker {
                data_timestamp_ms: item.ts.parse().unwrap_or(data_timestamp_ms),
                cached_at_ms: now,
                item,
            })
        })
        .collect()
}

fn symbol_from_row(item: &SpotTickerItem, arg: &TickerArg) -> Option<String> {
    let symbol = if item.symbol.trim().is_empty() {
        &arg.symbol
    } else {
        &item.symbol
    };
    (!symbol.trim().is_empty()).then(|| symbol.to_ascii_uppercase())
}

fn parse_spot_tick(row: &CachedTicker) -> Option<SpotTick> {
    let item = &row.item;
    let (base, quote) = crate::spot::suffix_pair(&item.symbol)?;
    crate::spot::spot_tick(crate::spot::SpotFields {
        venue: EXCHANGE,
        base: &base,
        quote: &quote,
        bid: &item.bid1_price,
        ask: &item.ask1_price,
        last: &item.last_price,
        bid_size: Some(&item.bid1_size),
        ask_size: Some(&item.ask1_size),
        volume_24h: quote_volume(item),
        exchange_ts_ms: (row.data_timestamp_ms > 0).then_some(row.data_timestamp_ms),
    })
}

fn quote_volume(item: &SpotTickerItem) -> &str {
    if item.turnover_24h.trim().is_empty() {
        &item.volume_24h
    } else {
        &item.turnover_24h
    }
}

#[derive(Debug, Deserialize)]
struct TickerEnvelope {
    #[serde(default)]
    arg: Option<TickerArg>,
    #[serde(default)]
    data: Vec<SpotTickerItem>,
    #[serde(default)]
    ts: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TickerArg {
    #[serde(default, rename = "instType")]
    inst_type: String,
    #[serde(default)]
    topic: String,
    #[serde(default)]
    symbol: String,
}

#[derive(Debug, Clone, Deserialize)]
struct SpotTickerItem {
    #[serde(default)]
    symbol: String,
    #[serde(default, rename = "lastPrice")]
    last_price: String,
    #[serde(default, rename = "bid1Price")]
    bid1_price: String,
    #[serde(default, rename = "ask1Price")]
    ask1_price: String,
    #[serde(default, rename = "bid1Size")]
    bid1_size: String,
    #[serde(default, rename = "ask1Size")]
    ask1_size: String,
    #[serde(default, rename = "volume24h")]
    volume_24h: String,
    #[serde(default, rename = "turnover24h")]
    turnover_24h: String,
    #[serde(default)]
    ts: String,
}

#[cfg(test)]
#[path = "bitget_uta_ws_spot_ticker_tests.rs"]
mod tests;
