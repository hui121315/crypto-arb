//! OKX mark/index/open-interest public WebSocket cache.
//!
//! Official docs:
//! - Mark price: <https://www.okx.com/docs-v5/en/#public-data-websocket-mark-price-channel>
//! - Index tickers: <https://www.okx.com/docs-v5/en/#public-data-websocket-index-tickers-channel>
//! - Open interest: <https://www.okx.com/docs-v5/en/#public-data-websocket-open-interest-channel>

use crate::adapters::okx_mark_index_data::{
    index_id_for_swap, parse_mark_index, parse_millis, swap_id_for_index, OkxIndexTickerItem,
    OkxMarkPriceItem, OkxOpenInterestItem,
};
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::MarkIndexInfo;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "okx";
const WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const CHANNEL_MARK: &str = "mark-price";
const CHANNEL_INDEX: &str = "index-tickers";
const CHANNEL_OI: &str = "open-interest";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const SUBSCRIBE_BATCH_SIZE: usize = 30;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone, Default)]
struct CachedParts {
    mark: Option<TimedValue>,
    index: Option<TimedValue>,
    oi: Option<TimedValue>,
    oi_usd: Option<TimedValue>,
}

#[derive(Debug, Clone, Copy)]
struct TimedValue {
    value: f64,
    timestamp_ms: i64,
    cached_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct MarkIndexStream {
    manager: Arc<WsManager>,
    rows: Arc<DashMap<String, CachedParts>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl MarkIndexStream {
    pub(crate) fn new() -> Arc<Self> {
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
            rows: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "okx mark/index ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    pub(crate) fn touch_many(&self, inst_ids: &[String]) {
        if inst_ids.is_empty() {
            return;
        }
        self.manager.activate_scope("perp-mark-index");
        let now = now_ms();
        let mut new_symbols = Vec::new();
        for inst_id in inst_ids {
            let symbol = stream_symbol(inst_id);
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

    pub(crate) fn snapshot(&self, inst_ids: &[String]) -> Vec<MarkIndexInfo> {
        inst_ids
            .iter()
            .filter_map(|inst_id| self.latest(inst_id))
            .collect()
    }

    fn latest(&self, inst_id: &str) -> Option<MarkIndexInfo> {
        let key = stream_symbol(inst_id);
        let row = self.rows.get(&key)?;
        let now = now_ms();
        let mark = row.mark.filter(|value| is_fresh(value.cached_at_ms, now))?;
        parse_mark_index(
            &key,
            &mark.value.to_string(),
            row.index
                .filter(|value| is_fresh(value.cached_at_ms, now))
                .map(|value| value.value.to_string())
                .as_deref(),
            row.oi
                .filter(|value| is_fresh(value.cached_at_ms, now))
                .map(|value| value.value.to_string())
                .as_deref(),
            row.oi_usd
                .filter(|value| is_fresh(value.cached_at_ms, now))
                .map(|value| value.value.to_string())
                .as_deref(),
            mark.timestamp_ms,
        )
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
                    "okx mark/index ws not connected yet; subscribe resumes after reconnect"
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
                    "okx mark/index ws subscription send failed"
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
                warn!(missed, "okx mark/index ws broadcast receiver lagged");
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
                warn!("okx mark/index ws circuit opened");
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
        debug!(%reason, "okx mark/index ws disconnected");
        self.clear_cache();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.rows.clear();
    }

    fn on_text(&self, text: &str) {
        let Some(update) = parse_update(text) else {
            return;
        };
        let now = now_ms();
        let key = update.stream_symbol.clone();
        self.rows
            .entry(key)
            .and_modify(|parts| apply_update(parts, &update, now))
            .or_insert_with(|| {
                let mut parts = CachedParts::default();
                apply_update(&mut parts, &update, now);
                parts
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
        for chunk in pruned.chunks(SUBSCRIBE_BATCH_SIZE) {
            for key in chunk {
                self.rows.remove(key);
            }
            self.spawn_subscription("unsubscribe", chunk.to_vec());
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "okx mark/index ws idle prune"
            );
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("perp-mark-index").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("perp-mark-index");
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

fn stream_symbol(symbol: &str) -> String {
    let upper = crate::adapter::strip_common_suffixes(symbol);
    format!("{upper}-USDT-SWAP")
}

fn channel_payload(op: &str, symbols: &[String]) -> String {
    let args: Vec<_> = symbols
        .iter()
        .flat_map(|symbol| {
            let symbol = stream_symbol(symbol);
            let index_id = index_id_for_swap(&symbol);
            [
                json!({"channel": CHANNEL_MARK, "instId": symbol}),
                json!({"channel": CHANNEL_OI, "instId": symbol}),
                json!({"channel": CHANNEL_INDEX, "instId": index_id}),
            ]
        })
        .collect();
    json!({ "op": op, "args": args }).to_string()
}

fn parse_update(text: &str) -> Option<ParsedUpdate> {
    let envelope: Envelope = serde_json::from_str(text).ok()?;
    let value = envelope.data.into_iter().next()?;
    match envelope.arg.channel.as_str() {
        CHANNEL_MARK => parse_mark_update(&envelope.arg.inst_id, value),
        CHANNEL_INDEX => parse_index_update(&envelope.arg.inst_id, value),
        CHANNEL_OI => parse_open_interest_update(&envelope.arg.inst_id, value),
        _ => None,
    }
}

fn parse_mark_update(inst_id: &str, value: Value) -> Option<ParsedUpdate> {
    let item: OkxMarkPriceItem = serde_json::from_value(value).ok()?;
    let mark = parse_positive(&item.mark_px)?;
    Some(ParsedUpdate {
        stream_symbol: stream_symbol(inst_id),
        kind: UpdateKind::Mark(TimedValue {
            value: mark,
            timestamp_ms: parse_millis(&item.ts),
            cached_at_ms: 0,
        }),
    })
}

fn parse_index_update(inst_id: &str, value: Value) -> Option<ParsedUpdate> {
    let item: OkxIndexTickerItem = serde_json::from_value(value).ok()?;
    let index = parse_positive(&item.idx_px)?;
    Some(ParsedUpdate {
        stream_symbol: swap_id_for_index(inst_id),
        kind: UpdateKind::Index(index),
    })
}

fn parse_open_interest_update(inst_id: &str, value: Value) -> Option<ParsedUpdate> {
    let item: OkxOpenInterestItem = serde_json::from_value(value).ok()?;
    Some(ParsedUpdate {
        stream_symbol: stream_symbol(inst_id),
        kind: UpdateKind::OpenInterest {
            oi: parse_positive(&item.oi),
            oi_usd: parse_positive(&item.oi_usd),
        },
    })
}

fn apply_update(parts: &mut CachedParts, update: &ParsedUpdate, now: i64) {
    match update.kind {
        UpdateKind::Mark(mut value) => {
            value.cached_at_ms = now;
            parts.mark = Some(value);
        }
        UpdateKind::Index(value) => parts.index = Some(timed(value, now)),
        UpdateKind::OpenInterest { oi, oi_usd } => {
            parts.oi = oi.map(|value| timed(value, now));
            parts.oi_usd = oi_usd.map(|value| timed(value, now));
        }
    }
}

fn timed(value: f64, now: i64) -> TimedValue {
    TimedValue {
        value,
        timestamp_ms: now,
        cached_at_ms: now,
    }
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

fn parse_positive(raw: &str) -> Option<f64> {
    raw.parse::<f64>()
        .ok()
        .filter(|value| value.is_finite() && *value > f64::EPSILON)
}

#[derive(Debug)]
struct ParsedUpdate {
    stream_symbol: String,
    kind: UpdateKind,
}

#[derive(Debug, Clone, Copy)]
enum UpdateKind {
    Mark(TimedValue),
    Index(f64),
    OpenInterest {
        oi: Option<f64>,
        oi_usd: Option<f64>,
    },
}

#[derive(Debug, Deserialize)]
struct Envelope {
    arg: ChannelArg,
    #[serde(default)]
    data: Vec<Value>,
}

#[derive(Debug, Deserialize)]
struct ChannelArg {
    channel: String,
    #[serde(rename = "instId")]
    inst_id: String,
}

#[cfg(test)]
#[path = "okx_ws_mark_index_tests.rs"]
mod tests;
