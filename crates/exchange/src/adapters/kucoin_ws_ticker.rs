//! KuCoin Futures public ticker WebSocket subscriber.
//!
//! Combines two KuCoin public channels into a single watchlist-driven cache:
//!
//! - `/contractMarket/snapshot:<sym>` (subject `snapshot.24h`) – 24h
//!   `lastPrice / volume / turnover / highPrice / lowPrice`.
//! - `/contractMarket/tickerV2:<sym>` (subject `tickerV2`) – best bid/ask price
//!   + size.
//!
//! Both streams arrive on the same `wss://...?token=<token>` connection
//! issued by the shared bullet-public handshake (`kucoin_ws_session`). The
//! adapter combines them into a [`TickerInfo`] only when both halves are
//! fresh (`CACHE_MAX_AGE_MS`), mirroring the Gate `gate_ws_ticker.rs` design.
//!
//! Funding, settlement timing and mark/index data are sourced by the sibling
//! Pro public WS stream; this classic connection remains only because its 24h
//! snapshot supplies quote turnover that the Pro ticker does not publish.
//!
//! Official docs:
//! - 24h snapshot: <https://www.kucoin.com/docs-new/3470089w0>
//! - tickerV2: <https://www.kucoin.com/docs-new/3470080w0>

use super::kucoin_config::KucoinConfig;
use super::kucoin_ws_market::{kucoin_to_normalized, normalized_to_kucoin};
use super::kucoin_ws_session::{
    fetch_public_session, spawn_public_session_refresh, PublicSession, PublicSessionKind,
};
use super::kucoin_ws_ticker_data::{
    parse_snapshot, parse_ticker_v2, CachedBookTicker, CachedSnapshot, SNAPSHOT_TOPIC_PREFIX,
    TICKER_TOPIC_PREFIX,
};
use crate::http::HttpClient;
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde_json::json;
use shared_types::TickerInfo;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::OnceCell;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "kucoin";
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
    inner: OnceCell<Arc<TickerStreamInner>>,
}

#[derive(Debug)]
struct TickerStreamInner {
    manager: Arc<WsManager>,
    snapshots: Arc<DashMap<String, CachedSnapshot>>,
    books: Arc<DashMap<String, CachedBookTicker>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

pub(crate) fn latest_ticker(
    config: &KucoinConfig,
    http: &HttpClient,
    symbol: &str,
) -> Option<TickerInfo> {
    let stream = enabled_stream(config, http)?;
    stream.touch_many(&[symbol.to_owned()]);
    stream.latest_ticker(symbol)
}

pub(crate) fn snapshot_tickers(
    config: &KucoinConfig,
    http: &HttpClient,
    symbols: Option<&[String]>,
) -> Option<Vec<TickerInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config, http)?;
    stream.touch_many(symbols);
    // 部分新鲜即服务：单个未上市/迟到符号不再把整个 venue 打回 REST。
    let rows = stream.snapshot_tickers(symbols);
    (!rows.is_empty()).then_some(rows)
}

fn enabled_stream(config: &KucoinConfig, http: &HttpClient) -> Option<Arc<TickerStream>> {
    if config.base_url_override.is_some() {
        return None;
    }
    let stream = SHARED_STREAM.get_or_init(|| {
        let stream = Arc::new(TickerStream {
            inner: OnceCell::new(),
        });
        let bootstrap = Arc::clone(&stream);
        let http = http.clone();
        tokio::spawn(async move {
            if let Err(error) = bootstrap.init(http).await {
                warn!(error = %error, "kucoin ticker ws bootstrap failed");
            }
        });
        stream
    });
    Some(Arc::clone(stream))
}

impl TickerStream {
    async fn init(&self, http: HttpClient) -> crate::error::ExchangeResult<()> {
        let session = fetch_public_session(&http).await?;
        let inner = TickerStreamInner::new(session, http);
        let _ = self.inner.set(Arc::clone(&inner));
        Ok(())
    }

    fn touch_many(&self, symbols: &[String]) {
        if let Some(inner) = self.inner.get() {
            inner.touch_many(symbols);
        }
    }

    fn latest_ticker(&self, symbol: &str) -> Option<TickerInfo> {
        self.inner.get()?.latest_ticker(symbol)
    }

    fn snapshot_tickers(&self, symbols: &[String]) -> Vec<TickerInfo> {
        self.inner
            .get()
            .map(|inner| inner.snapshot_tickers(symbols))
            .unwrap_or_default()
    }
}

impl TickerStreamInner {
    fn new(session: PublicSession, http: HttpClient) -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: session.ws_url,
                exchange: EXCHANGE.into(),
                heartbeat_interval: Duration::from_millis(session.ping_interval_ms),
                heartbeat: WsHeartbeat::Text(r#"{"id":"ping","type":"ping"}"#.into()),
                inbound_codec: WsInboundCodec::Plain,
                server_ping: WsServerPing::None,
                initial_reconnect_delay: Duration::from_secs(1),
                max_reconnect_delay: Duration::from_secs(30),
                circuit_breaker_threshold: 10,
            })
            .with_demand_control(),
        );
        spawn_public_session_refresh(http, Arc::clone(&manager), PublicSessionKind::Futures);
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            snapshots: Arc::new(DashMap::new()),
            books: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "kucoin ticker ws supervisor exited");
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
        self.manager.activate_scope("perp-ticker");
        let now = now_ms();
        let mut new_kucoin_symbols = Vec::new();
        for symbol in symbols {
            let kucoin = normalized_to_kucoin(symbol);
            let mut needs_subscribe = false;
            self.subscriptions
                .entry(kucoin.clone())
                .and_modify(|state| state.last_touched_ms = now)
                .or_insert_with(|| {
                    needs_subscribe = true;
                    SubscriptionState {
                        last_touched_ms: now,
                        sent_on_current_connection: false,
                    }
                });
            if needs_subscribe {
                new_kucoin_symbols.push(kucoin);
            }
        }
        self.spawn_subscribe_batches(&new_kucoin_symbols);
    }

    fn latest_ticker(&self, symbol: &str) -> Option<TickerInfo> {
        let kucoin = normalized_to_kucoin(symbol);
        let snapshot = self.snapshots.get(&kucoin)?;
        let book = self.books.get(&kucoin)?;
        let now = now_ms();
        if !is_fresh(snapshot.cached_at_ms, now) || !is_fresh(book.cached_at_ms, now) {
            return None;
        }
        Some(TickerInfo {
            symbol: kucoin_to_normalized(&kucoin),
            exchange: EXCHANGE.into(),
            bid: book.bid,
            ask: book.ask,
            last: snapshot.last_price,
            volume_24h: snapshot.volume_24h_quote,
            timestamp: snapshot.data_timestamp_ms.max(book.data_timestamp_ms),
        })
    }

    fn snapshot_tickers(&self, symbols: &[String]) -> Vec<TickerInfo> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_ticker(symbol))
            .collect()
    }

    fn spawn_subscribe_batches(&self, kucoin_symbols: &[String]) {
        for batch in kucoin_symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
            for kucoin in batch {
                self.spawn_subscription("subscribe", kucoin.clone());
            }
        }
    }

    fn spawn_subscription(&self, op: &'static str, kucoin_symbol: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                debug!(
                    op,
                    symbol = %kucoin_symbol,
                    "kucoin ticker ws not connected yet; subscribe will resume after reconnect"
                );
                return;
            }
            for topic_prefix in [SNAPSHOT_TOPIC_PREFIX, TICKER_TOPIC_PREFIX] {
                let payload = command_payload(op, topic_prefix, &kucoin_symbol);
                if let Err(error) = manager.send(Message::Text(payload)).await {
                    warn!(
                        op,
                        topic_prefix,
                        symbol = %kucoin_symbol,
                        error = %error,
                        "kucoin ticker ws subscription send failed"
                    );
                    return;
                }
            }
            if op == "subscribe" {
                if let Some(mut state) = subscriptions.get_mut(&kucoin_symbol) {
                    state.sent_on_current_connection = true;
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
                warn!(missed, "kucoin ticker ws broadcast receiver lagged");
                self.snapshots.clear();
                self.books.clear();
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
                self.snapshots.clear();
                self.books.clear();
                warn!("kucoin ticker ws circuit opened");
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
        for symbol in symbols {
            self.spawn_subscription("subscribe", symbol);
        }
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "kucoin ticker ws disconnected");
        self.snapshots.clear();
        self.books.clear();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn on_text(&self, text: &str) {
        if let Some(snap) = parse_snapshot(text) {
            self.snapshots.insert(snap.kucoin_symbol, snap.cached);
            return;
        }
        if let Some(ticker) = parse_ticker_v2(text) {
            self.books.insert(ticker.kucoin_symbol, ticker.cached);
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
            self.snapshots.remove(symbol);
            self.books.remove(symbol);
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "kucoin ticker ws idle prune"
            );
            self.spawn_idle_unsubscribes(pruned);
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("perp-ticker").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("perp-ticker");
            }
        }
    }

    fn spawn_idle_unsubscribes(&self, symbols: Vec<String>) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            for symbol in &symbols {
                for topic_prefix in [SNAPSHOT_TOPIC_PREFIX, TICKER_TOPIC_PREFIX] {
                    let payload = command_payload("unsubscribe", topic_prefix, symbol);
                    if let Err(error) = manager.send(Message::Text(payload)).await {
                        debug!(
                            topic_prefix,
                            count = symbols.len(),
                            error = %error,
                            "kucoin ticker ws idle unsubscribe stopped after disconnect"
                        );
                        return;
                    }
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

fn command_payload(kind: &str, topic_prefix: &str, kucoin_symbol: &str) -> String {
    json!({
        "id": format!("ticker-{kucoin_symbol}-{}", now_ms()),
        "type": kind,
        "topic": format!("{topic_prefix}:{kucoin_symbol}"),
        "privateChannel": false,
        "response": true,
    })
    .to_string()
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

#[cfg(test)]
#[path = "kucoin_ws_ticker_tests.rs"]
mod tests;
