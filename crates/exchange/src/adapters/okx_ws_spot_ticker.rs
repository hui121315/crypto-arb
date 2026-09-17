//! OKX public Spot WebSocket ticker subscriber.
//!
//! The OKX `tickers` channel is shared by instrument types. This module keeps
//! a spot-only cache keyed by `BASE-QUOTE` IDs and leaves the existing
//! `okx_ws_ticker` module responsible for `*-USDT-SWAP` contracts.
//!
//! Official docs:
//! - Public WS URL: <https://www.okx.com/docs-v5/en/#overview-production-trading-services>
//! - Tickers channel: <https://www.okx.com/docs-v5/en/#public-data-websocket-tickers-channel>

use crate::adapters::okx_market_data::{parse_spot_tick, TickerItem};
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::json;
use shared_types::SpotTick;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "okx";
const WS_URL: &str = "wss://ws.okx.com:8443/ws/v5/public";
const CHANNEL: &str = "tickers";
const INST_TYPE_SPOT: &str = "SPOT";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const EVENT_BUFFER_CAPACITY: usize = 8_192;

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug)]
struct CachedTicker {
    item: TickerItem,
    cached_at_ms: i64,
}

#[derive(Debug)]
pub(crate) struct SpotTickerStream {
    manager: Arc<WsManager>,
    rows: Arc<DashMap<String, CachedTicker>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl SpotTickerStream {
    pub(crate) fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new_with_event_capacity(
                WsConfig {
                    url: WS_URL.into(),
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
                warn!(error = %error, "okx spot ticker ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());

        stream
    }

    pub(crate) fn touch_many(&self, inst_ids: &[String]) {
        self.manager.activate_scope("spot-ticker");
        let now = now_ms();
        let mut new_symbols = Vec::new();
        for inst_id in inst_ids {
            let mut needs_subscribe = false;
            self.subscriptions
                .entry(inst_id.clone())
                .and_modify(|state| state.last_touched_ms = now)
                .or_insert_with(|| {
                    needs_subscribe = true;
                    SubscriptionState {
                        last_touched_ms: now,
                        sent_on_current_connection: false,
                    }
                });
            if needs_subscribe {
                new_symbols.push(inst_id.clone());
            }
        }
        self.spawn_subscribe_batches(&new_symbols);
    }

    pub(crate) fn snapshot(&self, inst_ids: &[String]) -> Vec<SpotTick> {
        let now = now_ms();
        inst_ids
            .iter()
            .filter_map(|inst_id| self.latest_at(inst_id, now))
            .collect()
    }

    pub(crate) fn connection_problem(&self) -> Option<String> {
        self.manager.connection_problem()
    }

    fn latest_at(&self, inst_id: &str, now: i64) -> Option<SpotTick> {
        let cached = self.rows.get(inst_id)?;
        if !is_fresh(cached.cached_at_ms, now) {
            return None;
        }
        parse_spot_tick(&cached.item)
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
                    "okx spot ticker ws not connected yet; subscribe resumes after reconnect"
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
                    "okx spot ticker ws subscription send failed"
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
                warn!(missed, "okx spot ticker ws broadcast receiver lagged");
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
                warn!("okx spot ticker ws circuit opened");
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
        debug!(%reason, "okx spot ticker ws disconnected");
        self.clear_cache();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.rows.clear();
    }

    fn on_text(&self, text: &str) {
        let Some(parsed) = parse_ticker_update(text) else {
            return;
        };
        if !self.subscriptions.contains_key(&parsed.stream_symbol) {
            return;
        }
        self.rows.insert(
            parsed.stream_symbol,
            CachedTicker {
                item: parsed.item,
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
        for symbol in &pruned {
            self.rows.remove(symbol);
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "okx spot ticker ws idle prune"
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
                        "okx spot ticker ws idle unsubscribe stopped after disconnect"
                    );
                    return;
                }
            }
        });
    }
}

pub(crate) fn spot_inst_ids(symbols: &[String]) -> Option<Vec<String>> {
    if symbols.is_empty() {
        return None;
    }
    let inst_ids: Vec<String> = symbols
        .iter()
        .filter_map(|symbol| crate::spot::native_pair_symbol(symbol, '-'))
        .collect();
    (!inst_ids.is_empty()).then_some(inst_ids)
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

fn channel_payload(op: &str, symbols: &[String]) -> String {
    let args: Vec<_> = symbols
        .iter()
        .map(|symbol| {
            json!({
                "channel": CHANNEL,
                "instType": INST_TYPE_SPOT,
                "instId": symbol
            })
        })
        .collect();
    json!({ "op": op, "args": args }).to_string()
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

#[derive(Debug)]
struct ParsedTickerUpdate {
    stream_symbol: String,
    item: TickerItem,
}

fn parse_ticker_update(text: &str) -> Option<ParsedTickerUpdate> {
    let envelope: TickerEnvelope = serde_json::from_str(text).ok()?;
    if envelope.arg.channel != CHANNEL {
        return None;
    }
    let item = envelope
        .data
        .into_iter()
        .find(|item| item.inst_type() == Some(INST_TYPE_SPOT))?;
    Some(ParsedTickerUpdate {
        stream_symbol: envelope.arg.inst_id,
        item,
    })
}

#[derive(Debug, Deserialize)]
struct TickerEnvelope {
    arg: TickerArg,
    #[serde(default)]
    data: Vec<TickerItem>,
}

#[derive(Debug, Deserialize)]
struct TickerArg {
    channel: String,
    #[serde(rename = "instId")]
    inst_id: String,
}

#[cfg(test)]
#[path = "okx_ws_spot_ticker_tests.rs"]
mod tests;
