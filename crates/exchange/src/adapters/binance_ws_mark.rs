//! Binance USD-M Futures mark price + funding WebSocket subscriber.
//!
//! Each watched symbol opens a `<sym>@markPrice@1s` stream on the shared
//! `wss://fstream.binance.com/market/ws` connection. The payload carries the
//! funding rate (`r`), next funding timestamp (`T`), mark price (`p`) and
//! index price (`i`); funding interval (8h / 4h / 1h) is **not** part of
//! the stream, so callers stitch the cached row with the value cached in
//! `binance_funding_info::FundingIntervalCache` to build `FundingRateData`.
//!
//! Official docs:
//! - Mark Price Stream: <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Mark-Price-Stream>
//! - Live subscribe/unsubscribe: <https://developers.binance.com/docs/derivatives/usds-margined-futures/websocket-market-streams/Live-Subscribing-Unsubscribing-to-streams>

use super::binance_config::BinanceConfig;
use crate::adapter::strip_common_suffixes;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use shared_types::{FundingRateData, MarkIndexInfo};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "binance";
// 2026-03 币安把统计类市场流迁到 /market 路径（changelog 2026-03-05，旧
// /ws 2026-04-23 起只剩盘口类流）；markPrice 在 /market/ws 上实测正常推送。
const WS_URL: &str = "wss://fstream.binance.com/market/ws";
const STREAM_SUFFIX: &str = "@markPrice@1s";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

static SHARED_STREAM: OnceLock<Arc<MarkPriceStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedMarkRow {
    item: MarkPriceItem,
    cached_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct MarkPriceStream {
    manager: Arc<WsManager>,
    rows: Arc<DashMap<String, CachedMarkRow>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    /// Monotonic request id for `SUBSCRIBE` / `UNSUBSCRIBE` frames (Binance
    /// echoes it back on the ack so we can correlate if we ever surface acks
    /// to the dispatcher).
    next_request_id: AtomicU64,
}

/// Look up the latest cached mark-price row for `symbol` and stitch it with
/// the supplied `interval_hours` to build a `FundingRateData`.
///
/// Returns `None` if the WS cache is empty / stale for the symbol or if the
/// stream is disabled by configuration (testnet / `base_url_override`).
pub(crate) fn latest_funding(
    config: &BinanceConfig,
    symbol: &str,
    interval_hours: u32,
) -> Option<FundingRateData> {
    let stream = enabled_stream(config)?;
    stream.touch_many(&[symbol.to_owned()]);
    stream.latest_funding(symbol, interval_hours)
}

/// Snapshot `FundingRateData` rows for `symbols` from the WS cache. Returns
/// `None` when **any** symbol is missing a fresh row, so the caller can fall
/// back to the REST aggregate atomically. `interval_for` is invoked per
/// symbol so callers can mix the per-symbol funding interval cache.
pub(crate) fn snapshot_funding<F>(
    config: &BinanceConfig,
    symbols: Option<&[String]>,
    interval_for: F,
) -> Option<Vec<FundingRateData>>
where
    F: Fn(&str) -> u32,
{
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    // 部分新鲜即服务；单个未到首帧的符号不会阻塞其余新鲜行。
    let rows = stream.snapshot_funding(symbols, interval_for);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn snapshot_mark_index(
    config: &BinanceConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<MarkIndexInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let rows = stream.snapshot_mark_index(symbols);
    (!rows.is_empty()).then_some(rows)
}

fn enabled_stream(config: &BinanceConfig) -> Option<Arc<MarkPriceStream>> {
    if config.testnet || config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(MarkPriceStream::new)))
}

impl MarkPriceStream {
    fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: WS_URL.into(),
                exchange: EXCHANGE.into(),
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
            manager: Arc::clone(&manager),
            rows: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            next_request_id: AtomicU64::new(1),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "binance mark price ws supervisor exited");
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
        self.manager.activate_scope("perp-mark-funding");
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
                        sent_on_current_connection: false,
                    }
                });
            if needs_subscribe {
                new_symbols.push(symbol);
            }
        }
        self.spawn_subscribe_batches(&new_symbols);
    }

    fn latest_funding(&self, symbol: &str, interval_hours: u32) -> Option<FundingRateData> {
        let row = self.rows.get(&stream_symbol(symbol))?;
        is_fresh(row.cached_at_ms, now_ms())
            .then(|| parse_funding(&row.item, interval_hours))
            .flatten()
    }

    fn snapshot_funding<F>(&self, symbols: &[String], interval_for: F) -> Vec<FundingRateData>
    where
        F: Fn(&str) -> u32,
    {
        symbols
            .iter()
            .filter_map(|symbol| {
                let exch = stream_symbol(symbol).to_ascii_uppercase();
                self.latest_funding(symbol, interval_for(&exch))
            })
            .collect()
    }

    fn latest_mark_index(&self, symbol: &str) -> Option<MarkIndexInfo> {
        let row = self.rows.get(&stream_symbol(symbol))?;
        is_fresh(row.cached_at_ms, now_ms())
            .then(|| parse_mark_index(&row.item))
            .flatten()
    }

    fn snapshot_mark_index(&self, symbols: &[String]) -> Vec<MarkIndexInfo> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_mark_index(symbol))
            .collect()
    }

    fn spawn_subscribe_batches(&self, symbols: &[String]) {
        for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
            self.spawn_subscription("SUBSCRIBE", batch.to_vec());
        }
    }

    fn spawn_subscription(&self, method: &'static str, symbols: Vec<String>) {
        if symbols.is_empty() {
            return;
        }
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        let request_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(async move {
            if method != "UNSUBSCRIBE" {
                wait_until_connected(&manager).await;
            }
            if !manager.is_connected().await {
                debug!(
                    method,
                    count = symbols.len(),
                    "binance mark price ws not connected yet; subscribe resumes after reconnect"
                );
                return;
            }
            let payload = subscription_payload(method, &symbols, request_id);
            match manager.send(Message::Text(payload)).await {
                Ok(()) if method == "SUBSCRIBE" => mark_sent(&subscriptions, &symbols),
                Ok(()) => {}
                Err(error) if method == "UNSUBSCRIBE" => debug!(
                    count = symbols.len(),
                    error = %error,
                    "binance mark price ws disconnected before idle unsubscribe"
                ),
                Err(error) => warn!(
                    method,
                    count = symbols.len(),
                    error = %error,
                    "binance mark price ws subscription send failed"
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
                self.clear_cache();
                warn!(missed, "binance mark price ws broadcast receiver lagged");
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
                warn!("binance mark price ws circuit opened");
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
        self.clear_cache();
        debug!(%reason, "binance mark price ws disconnected");
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.rows.clear();
    }

    fn on_text(&self, text: &str) {
        let Some(item) = parse_mark_price_update(text) else {
            return;
        };
        let key = item.symbol.to_ascii_lowercase();
        self.rows.insert(
            key,
            CachedMarkRow {
                item,
                cached_at_ms: now_ms(),
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
        for chunk in pruned.chunks(SUBSCRIBE_BATCH_SIZE) {
            for symbol in chunk {
                self.rows.remove(symbol);
            }
            self.spawn_subscription("UNSUBSCRIBE", chunk.to_vec());
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "binance mark price ws idle prune"
            );
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("perp-mark-funding").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("perp-mark-funding");
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

fn mark_sent(subscriptions: &DashMap<String, SubscriptionState>, symbols: &[String]) {
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = true;
        }
    }
}

/// Lower-cased exchange symbol with an explicit USDT/USDC quote preserved.
/// Binance's market streams are always lower-case.
fn stream_symbol(symbol: &str) -> String {
    super::binance_format::usdm_stream_symbol(symbol)
}

fn subscription_payload(method: &str, symbols: &[String], request_id: u64) -> String {
    let params: Vec<String> = symbols
        .iter()
        .map(|symbol| format!("{symbol}{STREAM_SUFFIX}"))
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

fn parse_mark_price_update(text: &str) -> Option<MarkPriceItem> {
    let item: MarkPriceItem = serde_json::from_str(text).ok()?;
    (item.event_type == EVENT_TYPE_MARK_PRICE).then_some(item)
}

const EVENT_TYPE_MARK_PRICE: &str = "markPriceUpdate";

fn parse_funding(item: &MarkPriceItem, interval_hours: u32) -> Option<FundingRateData> {
    let rate: f64 = item
        .funding_rate
        .parse()
        .ok()
        .filter(|v: &f64| v.is_finite())?;
    if rate == 0.0 && item.funding_rate.trim().is_empty() {
        return None;
    }
    if item.next_funding_time <= 0 {
        return None;
    }
    let interval = interval_hours.clamp(1, 24);
    Some(FundingRateData {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: EXCHANGE.into(),
        rate,
        rate_8h: rate * (8.0 / f64::from(interval)),
        predicted_rate: None,
        next_funding_time: item.next_funding_time,
        funding_interval: interval,
        volume_24h: 0.0,
        timestamp: item.event_time.max(0),
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

fn parse_mark_index(item: &MarkPriceItem) -> Option<MarkIndexInfo> {
    Some(MarkIndexInfo {
        symbol: strip_common_suffixes(&item.symbol),
        exchange: EXCHANGE.into(),
        mark_price: parse_positive(&item.mark_price)?,
        index_price: parse_positive(&item.index_price),
        open_interest: None,
        open_interest_value: None,
        timestamp: item.event_time.max(0),
    })
}

fn parse_positive(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > f64::EPSILON)
}

/// `<sym>@markPrice@1s` payload shape (see official docs above).
///
/// Only documented fields consumed by funding or mark/index parsing are kept.
#[derive(Debug, Clone, Deserialize)]
struct MarkPriceItem {
    #[serde(rename = "e")]
    event_type: String,
    #[serde(default, rename = "E")]
    event_time: i64,
    #[serde(default, rename = "s")]
    symbol: String,
    #[serde(default, rename = "p")]
    mark_price: String,
    #[serde(default, rename = "i")]
    index_price: String,
    #[serde(default, rename = "r")]
    funding_rate: String,
    #[serde(default, rename = "T")]
    next_funding_time: i64,
}

#[cfg(test)]
#[path = "binance_ws_mark_tests.rs"]
mod tests;
