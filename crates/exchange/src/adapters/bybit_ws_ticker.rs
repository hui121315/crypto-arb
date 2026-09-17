//! Bybit V5 public WebSocket ticker/funding subscriber.
//!
//! Official docs:
//! - Public WS connection: <https://bybit-exchange.github.io/docs/v5/ws/connect>
//! - Ticker topic: <https://bybit-exchange.github.io/docs/v5/websocket/public/ticker>

use super::bybit_config::BybitConfig;
use super::bybit_market_data::parse_mark_index;
use super::bybit_ws_ticker_data::{
    channel_payload, is_complete, merge_item, parse_ticker_update, stream_symbol, to_funding,
    to_ticker, CachedTicker, ParsedTickerUpdate,
};
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use shared_types::{FundingRateData, MarkIndexInfo, TickerInfo};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "bybit";
const WS_URL: &str = "wss://stream.bybit.com/v5/public/linear";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const EVENT_BUFFER_CAPACITY: usize = 8_192;

static SHARED_STREAM: OnceLock<Arc<TickerStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug)]
pub(crate) struct TickerStream {
    manager: Arc<WsManager>,
    rows: Arc<DashMap<String, CachedTicker>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

pub(crate) fn latest_ticker(config: &BybitConfig, symbol: &str) -> Option<TickerInfo> {
    let stream = enabled_stream(config)?;
    stream.touch_many(&[symbol.to_owned()]);
    stream.latest_ticker(symbol)
}

pub(crate) fn latest_funding(config: &BybitConfig, symbol: &str) -> Option<FundingRateData> {
    let stream = enabled_stream(config)?;
    stream.touch_many(&[symbol.to_owned()]);
    stream.latest_funding(symbol)
}

pub(crate) fn snapshot_tickers(
    config: &BybitConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<TickerInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    // 部分新鲜即服务：单个未上市/迟到符号不再把整个 venue 打回 REST。
    let rows = stream.snapshot_tickers(symbols);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn snapshot_funding(
    config: &BybitConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<FundingRateData>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let rows = stream.snapshot_funding(symbols);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn snapshot_mark_index(
    config: &BybitConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<MarkIndexInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let rows = stream.snapshot_mark_index(symbols);
    (!rows.is_empty()).then_some(rows)
}

fn enabled_stream(config: &BybitConfig) -> Option<Arc<TickerStream>> {
    if config.testnet || config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(TickerStream::new)))
}

pub(crate) fn shared_manager(config: &BybitConfig) -> Option<Arc<WsManager>> {
    enabled_stream(config).map(|stream| Arc::clone(&stream.manager))
}

impl TickerStream {
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
            rows: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "bybit ticker ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch_many(&self, symbols: &[String]) {
        if symbols.is_empty() {
            return;
        }
        self.manager.activate_scope("perp-market");
        let now = now_ms();
        let mut new_symbols = Vec::new();
        for symbol in symbols {
            let symbol = stream_symbol(symbol);
            let mut needs_subscribe = false;
            self.subscriptions
                .entry(symbol.clone())
                .and_modify(|state| {
                    state.last_touched_ms = now;
                    needs_subscribe = claim_subscription(state);
                })
                .or_insert_with(|| {
                    needs_subscribe = true;
                    SubscriptionState {
                        last_touched_ms: now,
                        sent_on_current_connection: true,
                    }
                });
            if needs_subscribe {
                new_symbols.push(symbol);
            }
        }
        self.spawn_subscribe_batches(&new_symbols);
    }

    fn latest_ticker(&self, symbol: &str) -> Option<TickerInfo> {
        let cached = self.fresh_row(symbol)?;
        to_ticker(&cached)
    }

    fn latest_funding(&self, symbol: &str) -> Option<FundingRateData> {
        let cached = self.fresh_row(symbol)?;
        to_funding(&cached)
    }

    fn snapshot_tickers(&self, symbols: &[String]) -> Vec<TickerInfo> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_ticker(symbol))
            .collect()
    }

    fn snapshot_funding(&self, symbols: &[String]) -> Vec<FundingRateData> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_funding(symbol))
            .collect()
    }

    fn latest_mark_index(&self, symbol: &str) -> Option<MarkIndexInfo> {
        let cached = self.fresh_row(symbol)?;
        parse_mark_index(&cached.item, cached.data_timestamp_ms)
    }

    fn snapshot_mark_index(&self, symbols: &[String]) -> Vec<MarkIndexInfo> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_mark_index(symbol))
            .collect()
    }

    fn fresh_row(&self, symbol: &str) -> Option<CachedTicker> {
        let cached = self.rows.get(&stream_symbol(symbol))?;
        is_fresh(cached.cached_at_ms, now_ms()).then(|| cached.clone())
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
            if op != "unsubscribe" {
                wait_until_connected(&manager).await;
            }
            if !manager.is_connected().await {
                release_subscriptions(&subscriptions, &symbols, op);
                debug!(
                    count = symbols.len(),
                    "bybit ticker ws not connected yet; subscribe resumes after reconnect"
                );
                return;
            }
            let payload = channel_payload(op, &symbols);
            if let Err(error) = manager.send(Message::Text(payload)).await {
                release_subscriptions(&subscriptions, &symbols, op);
                if op == "unsubscribe" {
                    debug!(
                        count = symbols.len(),
                        error = %error,
                        "bybit ticker ws disconnected before idle unsubscribe"
                    );
                } else {
                    warn!(
                        op,
                        count = symbols.len(),
                        error = %error,
                        "bybit ticker ws subscription send failed"
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
                self.clear_cache();
                warn!(missed, "bybit ticker ws broadcast receiver lagged");
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
                warn!("bybit ticker ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let mut symbols = Vec::with_capacity(self.subscriptions.len());
        for mut entry in self.subscriptions.iter_mut() {
            if claim_subscription(entry.value_mut()) {
                symbols.push(entry.key().clone());
            }
        }
        self.spawn_subscribe_batches(&symbols);
    }

    fn on_disconnected(&self, reason: &str) {
        self.clear_cache();
        debug!(%reason, "bybit ticker ws disconnected");
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.rows.clear();
    }

    fn on_text(&self, text: &str) {
        let Some(update) = parse_ticker_update(text) else {
            return;
        };
        let now = now_ms();
        if update.is_delta {
            self.merge_delta(update, now);
            return;
        }
        self.insert_snapshot(update, now);
    }

    fn insert_snapshot(&self, update: ParsedTickerUpdate, cached_at_ms: i64) {
        if !is_complete(&update.item) {
            return;
        }
        self.rows.insert(
            update.stream_symbol,
            CachedTicker {
                item: update.item,
                cached_at_ms,
                data_timestamp_ms: update.timestamp_ms,
            },
        );
    }

    fn merge_delta(&self, update: ParsedTickerUpdate, cached_at_ms: i64) {
        if let Some(mut cached) = self.rows.get_mut(&update.stream_symbol) {
            merge_item(&mut cached.item, update.item);
            cached.cached_at_ms = cached_at_ms;
            cached.data_timestamp_ms = update.timestamp_ms;
        } else {
            self.insert_snapshot(update, cached_at_ms);
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
                self.rows.remove(symbol);
            }
            self.spawn_subscription("unsubscribe", chunk.to_vec());
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "bybit ticker ws idle prune"
            );
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("perp-market").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("perp-market");
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

fn claim_subscription(state: &mut SubscriptionState) -> bool {
    if state.sent_on_current_connection {
        return false;
    }
    state.sent_on_current_connection = true;
    true
}

fn release_subscriptions(
    subscriptions: &DashMap<String, SubscriptionState>,
    symbols: &[String],
    op: &str,
) {
    if op != "subscribe" {
        return;
    }
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = false;
        }
    }
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

#[cfg(test)]
#[path = "bybit_ws_ticker_tests.rs"]
mod tests;
