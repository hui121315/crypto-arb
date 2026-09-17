use super::bitget_config::BitgetConfig;
use super::bitget_uta_config::PROD_WS_PUBLIC;
use super::bitget_uta_ws_ticker_data::{
    channel_payload, parse_funding, parse_mark_index, parse_ticker, parse_ticker_update,
    stream_symbol, CachedTicker, EXCHANGE,
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

const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
    /// symbol -> funding interval（小时），由 REST instruments 回灌。
    intervals: Arc<DashMap<String, u32>>,
}

pub(crate) fn latest_ticker(config: &BitgetConfig, symbol: &str) -> Option<TickerInfo> {
    let stream = enabled_stream(config)?;
    stream.touch_many(&[symbol.to_owned()]);
    stream.latest_ticker(symbol)
}

/// 部分新鲜即服务：单个未上市/迟到的符号只缺席自己的行，
/// 不再把整个 venue 打回 REST（历史上的全有或全无门槛曾因此长期 0 覆盖）。
pub(crate) fn snapshot_tickers(
    config: &BitgetConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<TickerInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let rows = stream.snapshot_tickers(symbols);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn snapshot_mark_index(
    config: &BitgetConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<MarkIndexInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let rows = stream.snapshot_mark_index(symbols);
    (!rows.is_empty()).then_some(rows)
}

/// REST funding 路径回灌 interval（无 REST 数据前 WS funding 按符号缺席）。
pub(crate) fn seed_funding_intervals(
    config: &BitgetConfig,
    intervals: &std::collections::HashMap<String, u32>,
) {
    if config.base_url_override.is_some() || intervals.is_empty() {
        return;
    }
    let Some(stream) = enabled_stream(config) else {
        return;
    };
    for (symbol, interval) in intervals {
        stream.intervals.insert(symbol.clone(), *interval);
    }
}

pub(crate) fn snapshot_funding(
    config: &BitgetConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<FundingRateData>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let now = now_ms();
    let rows: Vec<FundingRateData> = symbols
        .iter()
        .filter_map(|symbol| {
            let key = stream_symbol(symbol);
            let interval = stream.intervals.get(&key).map(|entry| *entry)?;
            let row = stream.rows.get(&key)?;
            is_fresh(row.cached_at_ms, now)
                .then(|| parse_funding(&row, interval))
                .flatten()
        })
        .collect();
    (!rows.is_empty()).then_some(rows)
}

fn enabled_stream(config: &BitgetConfig) -> Option<Arc<TickerStream>> {
    if config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(TickerStream::new)))
}

impl TickerStream {
    fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: PROD_WS_PUBLIC.into(),
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
            intervals: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "bitget uta ticker ws supervisor exited");
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

    fn latest_ticker(&self, symbol: &str) -> Option<TickerInfo> {
        let row = self.rows.get(&stream_symbol(symbol))?;
        is_fresh(row.cached_at_ms, now_ms())
            .then(|| parse_ticker(&row))
            .flatten()
    }

    fn snapshot_tickers(&self, symbols: &[String]) -> Vec<TickerInfo> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_ticker(symbol))
            .collect()
    }

    fn latest_mark_index(&self, symbol: &str) -> Option<MarkIndexInfo> {
        let row = self.rows.get(&stream_symbol(symbol))?;
        is_fresh(row.cached_at_ms, now_ms())
            .then(|| parse_mark_index(&row))
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
                    "bitget uta ticker ws not connected yet; subscribe resumes after reconnect"
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
                    "bitget uta ticker ws subscription send failed"
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
                warn!(missed, "bitget uta ticker ws broadcast receiver lagged");
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
                warn!("bitget uta ticker ws circuit opened");
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
        debug!(%reason, "bitget uta ticker ws disconnected");
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.rows.clear();
    }

    fn on_text(&self, text: &str) {
        for row in parse_ticker_update(text) {
            self.rows.insert(row.symbol.clone(), row);
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
                "bitget uta ticker ws idle prune"
            );
            self.spawn_idle_unsubscribes(pruned);
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("perp-market").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("perp-market");
            }
        }
    }

    fn spawn_idle_unsubscribes(&self, symbols: Vec<String>) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
                let payload = channel_payload("unsubscribe", batch);
                if let Err(error) = manager.send(Message::Text(payload)).await {
                    debug!(
                        count = symbols.len(),
                        error = %error,
                        "bitget uta ticker ws idle unsubscribe stopped after disconnect"
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

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

#[cfg(test)]
#[path = "bitget_uta_ws_ticker_tests.rs"]
mod tests;
