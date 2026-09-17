//! KuCoin Spot public ticker WebSocket subscriber.
//!
//! Classic Spot `All Tickers` carries full-market BBO changes every 100ms on a
//! single subscription. Small candidate sets additionally subscribe to Symbol
//! Snapshot so 24h quote volume remains available without multiplying the
//! discovery stream by every listed pair.
//!
//! Official docs:
//! - Spot token: <https://www.kucoin.com/docs-new/websocket-api/base-info/get-public-token-spot-margin>
//! - All Tickers: <https://www.kucoin.com/docs-new/3470064w0>
//! - Symbol Snapshot: <https://www.kucoin.com/docs-new/3470065w0>

use super::kucoin_config::KucoinConfig;
use super::kucoin_ws_session::{
    fetch_spot_public_session, spawn_public_session_refresh, PublicSession, PublicSessionKind,
};
use super::kucoin_ws_spot_ticker_data::{
    all_tickers_command_payload, command_payload, merge_cached_spot_ticker, parse_spot_ticker,
    spot_tick_from_cached, CachedSpotTicker, EXCHANGE,
};
use crate::http::HttpClient;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use shared_types::SpotTick;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::OnceCell;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const SUBSCRIBE_RETRY_MS: i64 = 3_000;
const MAX_SUBSCRIBE_ATTEMPTS: u8 = 3;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DETAIL_SUBSCRIPTION_MAX_SYMBOLS: usize = 64;
const EVENT_BUFFER_CAPACITY: usize = 8_192;

static SHARED_STREAM: OnceLock<Arc<SpotTickerStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    detail_requested: bool,
    subscribe_attempts: u8,
    last_subscribe_attempt_ms: i64,
}

#[derive(Debug)]
pub(crate) struct SpotTickerStream {
    inner: OnceCell<Arc<SpotTickerStreamInner>>,
}

#[derive(Debug)]
struct SpotTickerStreamInner {
    manager: Arc<WsManager>,
    rows: Arc<DashMap<String, CachedSpotTicker>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    all_market_sent: Arc<AtomicBool>,
}

pub(crate) fn snapshot_spot_ticks(
    config: &KucoinConfig,
    http: &HttpClient,
    symbols: Option<&[String]>,
) -> Option<Vec<SpotTick>> {
    let symbols = symbols.and_then(stream_symbols)?;
    let stream = enabled_stream(config, http)?;
    stream.touch_many(&symbols);
    let rows = stream.snapshot(&symbols);
    (!rows.is_empty()).then_some(rows)
}

fn enabled_stream(config: &KucoinConfig, http: &HttpClient) -> Option<Arc<SpotTickerStream>> {
    if config.base_url_override.is_some() {
        return None;
    }
    let stream = SHARED_STREAM.get_or_init(|| {
        let stream = Arc::new(SpotTickerStream {
            inner: OnceCell::new(),
        });
        let bootstrap = Arc::clone(&stream);
        let http = http.clone();
        tokio::spawn(async move {
            if let Err(error) = bootstrap.init(http).await {
                warn!(error = %error, "kucoin spot ticker ws bootstrap failed");
            }
        });
        stream
    });
    Some(Arc::clone(stream))
}

pub(crate) fn shared_manager(config: &KucoinConfig, http: &HttpClient) -> Option<Arc<WsManager>> {
    enabled_stream(config, http)?.manager()
}

pub(crate) fn spot_connection_problem(config: &KucoinConfig, http: &HttpClient) -> Option<String> {
    shared_manager(config, http)?.connection_problem()
}

impl SpotTickerStream {
    async fn init(&self, http: HttpClient) -> crate::error::ExchangeResult<()> {
        let session = fetch_spot_public_session(&http).await?;
        let inner = SpotTickerStreamInner::new(session, http);
        let _ = self.inner.set(Arc::clone(&inner));
        Ok(())
    }

    fn touch_many(&self, symbols: &[String]) {
        if let Some(inner) = self.inner.get() {
            inner.touch_many(symbols);
        }
    }

    fn manager(&self) -> Option<Arc<WsManager>> {
        self.inner.get().map(|inner| Arc::clone(&inner.manager))
    }

    fn snapshot(&self, symbols: &[String]) -> Vec<SpotTick> {
        self.inner
            .get()
            .map(|inner| inner.snapshot(symbols))
            .unwrap_or_default()
    }
}

impl SpotTickerStreamInner {
    fn new(session: PublicSession, http: HttpClient) -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new_with_event_capacity(
                WsConfig {
                    url: session.ws_url,
                    exchange: EXCHANGE.into(),
                    heartbeat_interval: Duration::from_millis(session.ping_interval_ms),
                    heartbeat: WsHeartbeat::Text(r#"{"id":"ping","type":"ping"}"#.into()),
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
        spawn_public_session_refresh(http, Arc::clone(&manager), PublicSessionKind::Spot);
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            rows: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
            all_market_sent: Arc::new(AtomicBool::new(false)),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "kucoin spot ticker ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch_many(&self, symbols: &[String]) {
        self.manager.activate_scope("spot-ticker");
        let now = now_ms();
        let request_detail = symbols.len() <= DETAIL_SUBSCRIPTION_MAX_SYMBOLS;
        let mut due_symbols = Vec::new();
        for symbol in symbols {
            let has_fresh_detail = self
                .rows
                .get(symbol)
                .is_some_and(|row| is_fresh(row.cached_at_ms, now) && row.volume_24h.is_some());
            let mut subscribe = false;
            self.subscriptions
                .entry(symbol.clone())
                .and_modify(|state| {
                    state.last_touched_ms = now;
                    state.detail_requested |= request_detail;
                    if request_detail && subscription_due(state, has_fresh_detail, now) {
                        record_subscription_attempt(state, now);
                        subscribe = true;
                    }
                })
                .or_insert_with(|| {
                    subscribe = request_detail;
                    SubscriptionState {
                        last_touched_ms: now,
                        detail_requested: request_detail,
                        subscribe_attempts: u8::from(request_detail),
                        last_subscribe_attempt_ms: if request_detail { now } else { 0 },
                    }
                });
            if subscribe {
                due_symbols.push(symbol.clone());
            }
        }
        for symbol in due_symbols {
            self.spawn_subscription("subscribe", symbol);
        }
        self.ensure_all_market_subscription();
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
        if !is_fresh(row.cached_at_ms, now) {
            return None;
        }
        spot_tick_from_cached(symbol, &row)
    }

    fn spawn_subscription(&self, kind: &'static str, symbol: String) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                debug!(
                    kind,
                    symbol = %symbol,
                    "kucoin spot ticker ws not connected yet; subscribe resumes on reconnect"
                );
                return;
            }
            if let Err(error) = manager
                .send(Message::Text(command_payload(kind, &symbol)))
                .await
            {
                warn!(
                    kind,
                    symbol = %symbol,
                    error = %error,
                    "kucoin spot ticker ws subscription send failed"
                );
            }
        });
    }

    fn ensure_all_market_subscription(&self) {
        if self.all_market_sent.swap(true, Ordering::AcqRel) {
            return;
        }
        let manager = Arc::clone(&self.manager);
        let sent = Arc::clone(&self.all_market_sent);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await
                || manager
                    .send(Message::Text(all_tickers_command_payload("subscribe")))
                    .await
                    .is_err()
            {
                sent.store(false, Ordering::Release);
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
                debug!(
                    missed,
                    "kucoin spot ticker ws skipped intermediate complete snapshots"
                );
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
                self.rows.clear();
                warn!("kucoin spot ticker ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let now = now_ms();
        let symbols: Vec<String> = self
            .subscriptions
            .iter()
            .filter(|entry| entry.value().detail_requested)
            .map(|entry| entry.key().clone())
            .collect();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().subscribe_attempts = 1;
            entry.value_mut().last_subscribe_attempt_ms = now;
        }
        for symbol in symbols {
            self.spawn_subscription("subscribe", symbol);
        }
        self.all_market_sent.store(false, Ordering::Release);
        if !self.subscriptions.is_empty() {
            self.ensure_all_market_subscription();
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "kucoin spot ticker ws disconnected");
        self.rows.clear();
        self.all_market_sent.store(false, Ordering::Release);
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().subscribe_attempts = 0;
            entry.value_mut().last_subscribe_attempt_ms = 0;
        }
    }

    fn on_text(&self, text: &str) {
        let Some(parsed) = parse_spot_ticker(text) else {
            return;
        };
        if !self.subscriptions.contains_key(&parsed.symbol) {
            return;
        }
        if let Some(mut state) = self.subscriptions.get_mut(&parsed.symbol) {
            state.subscribe_attempts = 1;
        }
        self.rows
            .entry(parsed.symbol)
            .and_modify(|current| merge_cached_spot_ticker(current, parsed.cached.clone()))
            .or_insert(parsed.cached);
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
        let pruned = self.prune_idle_subscriptions();
        self.remove_cached_rows(&pruned);
        self.unsubscribe_symbols(&pruned).await;
        self.log_prune(&pruned);
        if self.subscriptions.is_empty() {
            self.unsubscribe_all_market().await;
            self.manager.suspend_scope("spot-ticker").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("spot-ticker");
            }
        }
    }

    fn prune_idle_subscriptions(&self) -> Vec<String> {
        let cutoff = now_ms().saturating_sub(SUBSCRIPTION_IDLE_TTL_MS);
        let mut pruned = Vec::new();
        self.subscriptions.retain(|symbol, state| {
            let keep = state.last_touched_ms >= cutoff;
            if !keep {
                pruned.push(symbol.clone());
            }
            keep
        });
        pruned
    }

    fn remove_cached_rows(&self, symbols: &[String]) {
        for symbol in symbols {
            self.rows.remove(symbol);
        }
    }

    async fn unsubscribe_symbols(&self, symbols: &[String]) {
        for symbol in symbols {
            self.unsubscribe_symbol(symbol).await;
        }
    }

    async fn unsubscribe_symbol(&self, symbol: &str) {
        if let Err(error) = self
            .manager
            .send(Message::Text(command_payload("unsubscribe", symbol)))
            .await
        {
            debug!(
                symbol = %symbol,
                error = %error,
                "kucoin spot ticker ws unsubscribe failed"
            );
        }
    }

    async fn unsubscribe_all_market(&self) {
        if !self.all_market_sent.swap(false, Ordering::AcqRel) {
            return;
        }
        self.rows.clear();
        if let Err(error) = self
            .manager
            .send(Message::Text(all_tickers_command_payload("unsubscribe")))
            .await
        {
            debug!(error = %error, "kucoin all-market spot ticker unsubscribe failed");
        }
        if !self.subscriptions.is_empty() {
            self.ensure_all_market_subscription();
        }
    }

    fn log_prune(&self, pruned: &[String]) {
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "kucoin spot ticker ws idle prune"
            );
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

fn subscription_due(state: &SubscriptionState, has_fresh_row: bool, now_ms: i64) -> bool {
    if has_fresh_row || state.subscribe_attempts >= MAX_SUBSCRIBE_ATTEMPTS {
        return false;
    }
    state.subscribe_attempts == 0
        || now_ms.saturating_sub(state.last_subscribe_attempt_ms) >= SUBSCRIBE_RETRY_MS
}

fn record_subscription_attempt(state: &mut SubscriptionState, now_ms: i64) {
    state.subscribe_attempts = state.subscribe_attempts.saturating_add(1);
    state.last_subscribe_attempt_ms = now_ms;
}

fn stream_symbols(symbols: &[String]) -> Option<Vec<String>> {
    if symbols.is_empty() {
        return None;
    }
    let normalized: Vec<String> = symbols
        .iter()
        .filter_map(|symbol| crate::spot::native_pair_symbol(symbol, '-'))
        .collect();
    (!normalized.is_empty()).then_some(normalized)
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

#[cfg(test)]
#[path = "kucoin_ws_spot_ticker_tests.rs"]
mod tests;
