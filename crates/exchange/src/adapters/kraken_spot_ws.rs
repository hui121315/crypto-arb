//! Shared Kraken Spot WebSocket v2 market stream.

use super::kraken_config::{KrakenConfig, SPOT_PUBLIC_WS_URL};
use super::kraken_spot_book::{MergeOutcome, SpotBookState};
use crate::ws::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use crate::ExchangeError;
use arc_swap::ArcSwap;
use common::time::now_ms;
use dashmap::DashMap;
use serde_json::{json, Value};
use shared_types::{OrderBookInfo, SpotTick, VenueInstrument};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::{broadcast::error::RecvError, Mutex};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

const VENUE: &str = "kraken";
const APPLICATION_PING: &str = r#"{"method":"ping"}"#;
const TICKER_STALE_MS: i64 = 30_000;
const BOOK_STALE_MS: i64 = 10_000;
// Kraken emits a text heartbeat roughly once per second while a subscription
// is quiet. A short control-ping interval lets the shared idle watchdog replace
// a half-open proxy path before the 30-second ticker freshness window expires.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
// Kraken v2 emits an automatic heartbeat about once per second while an
// active subscription has no market updates. Keep a small jitter allowance,
// but fail closed well before the generic socket idle watchdog.
const WS_LIVENESS_MAX_AGE_MS: i64 = 3_500;
const IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const CONNECT_POLL: Duration = Duration::from_millis(100);
const TICKER_ACK_TIMEOUT_MS: i64 = 3_000;
const BOOK_ACK_TIMEOUT_MS: i64 = 3_000;
const BOOK_SUBSCRIPTION_RETRY_MS: i64 = 5_000;
const TICKER_BATCH: usize = 50;
const TICKER_BATCH_DELAY: Duration = Duration::from_millis(50);
const EVENT_CAPACITY: usize = 16_384;

mod dispatch;

static SHARED: OnceLock<Arc<KrakenSpotPublicStream>> = OnceLock::new();
static EXACT_SHARED: OnceLock<Arc<KrakenSpotPublicStream>> = OnceLock::new();
static REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent: bool,
    trigger: TickerTrigger,
}

#[derive(Debug, Clone)]
struct PendingTickerRequest {
    symbols: Vec<String>,
    subscribe: bool,
    sent_at_ms: i64,
}

#[derive(Debug)]
struct TickerCommand {
    req_id: u64,
    payload: String,
    request: PendingTickerRequest,
    trigger: TickerTrigger,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TickerTrigger {
    Trades,
    Bbo,
}

impl TickerTrigger {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Trades => "trades",
            Self::Bbo => "bbo",
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct BookSubscriptionState {
    last_touched_ms: i64,
    sent: bool,
    depth: usize,
    retry_after_ms: i64,
}

#[derive(Debug, Clone)]
struct PendingBookRequest {
    symbol: String,
    depth: usize,
    subscribe: bool,
    sent_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MethodAck {
    req_id: u64,
    success: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct CachedTick {
    tick: SpotTick,
    observed_at_ms: i64,
}

#[derive(Debug)]
pub(super) struct KrakenSpotPublicStream {
    manager: Arc<WsManager>,
    subscribe_instruments: bool,
    ticks: Arc<DashMap<String, CachedTick>>,
    books: Arc<DashMap<String, SpotBookState>>,
    instruments: Arc<ArcSwap<Vec<VenueInstrument>>>,
    ticker_subscriptions: Arc<DashMap<String, SubscriptionState>>,
    ticker_requests: Arc<DashMap<u64, PendingTickerRequest>>,
    ticker_problems: Arc<DashMap<String, String>>,
    book_subscriptions: Arc<DashMap<String, BookSubscriptionState>>,
    book_requests: Arc<DashMap<u64, PendingBookRequest>>,
    book_problems: Arc<DashMap<String, String>>,
    ticker_trade_command_lock: Arc<Mutex<()>>,
    ticker_bbo_command_lock: Arc<Mutex<()>>,
    connected: Arc<AtomicBool>,
    connection_generation: Arc<AtomicU64>,
    last_inbound_ms: AtomicI64,
}

impl KrakenSpotPublicStream {
    pub(super) fn shared(config: &KrakenConfig) -> Arc<Self> {
        if let Some(url) = &config.spot_public_ws_url_override {
            return Self::new(url.clone(), "spot");
        }
        Arc::clone(SHARED.get_or_init(|| Self::new(SPOT_PUBLIC_WS_URL.to_owned(), "spot")))
    }

    pub(super) fn exact(config: &KrakenConfig) -> Arc<Self> {
        if let Some(url) = &config.spot_public_ws_url_override {
            return Self::new(url.clone(), "spot-exact");
        }
        Arc::clone(
            EXACT_SHARED.get_or_init(|| Self::new(SPOT_PUBLIC_WS_URL.to_owned(), "spot-exact")),
        )
    }

    fn new(url: String, stream_name: &str) -> Arc<Self> {
        let manager = WsManager::new_with_event_capacity(
            WsConfig {
                url,
                exchange: format!("{VENUE}:{stream_name}"),
                heartbeat_interval: HEARTBEAT_INTERVAL,
                // Kraken documents an application-level ping/pong method.
                // Text frames also survive the local proxy path more
                // reliably than protocol-level control frames.
                heartbeat: WsHeartbeat::Text(APPLICATION_PING.to_owned()),
                inbound_codec: WsInboundCodec::Plain,
                server_ping: WsServerPing::None,
                initial_reconnect_delay: Duration::from_secs(1),
                max_reconnect_delay: Duration::from_secs(30),
                circuit_breaker_threshold: 10,
            },
            EVENT_CAPACITY,
        )
        .with_demand_control();
        let manager = if stream_name == "spot-exact" {
            manager.with_priority_connect()
        } else {
            manager
        };
        let manager = Arc::new(manager);
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            // The exact BBO socket serves a bounded symbol set. Instrument is
            // an all-pairs feed and belongs only on the shared registry socket.
            subscribe_instruments: stream_name != "spot-exact",
            ticks: Arc::new(DashMap::new()),
            books: Arc::new(DashMap::new()),
            instruments: Arc::new(ArcSwap::from_pointee(Vec::new())),
            ticker_subscriptions: Arc::new(DashMap::new()),
            ticker_requests: Arc::new(DashMap::new()),
            ticker_problems: Arc::new(DashMap::new()),
            book_subscriptions: Arc::new(DashMap::new()),
            book_requests: Arc::new(DashMap::new()),
            book_problems: Arc::new(DashMap::new()),
            ticker_trade_command_lock: Arc::new(Mutex::new(())),
            ticker_bbo_command_lock: Arc::new(Mutex::new(())),
            connected: Arc::new(AtomicBool::new(false)),
            connection_generation: Arc::new(AtomicU64::new(0)),
            last_inbound_ms: AtomicI64::new(0),
        });
        let receiver = manager.subscribe();
        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "kraken spot ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch(receiver));
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    pub(super) fn touch_tickers(&self, symbols: &[String]) {
        self.touch_tickers_with_trigger(symbols, TickerTrigger::Bbo);
    }

    pub(super) fn touch_discovery_tickers(&self, symbols: &[String]) {
        self.touch_tickers_with_trigger(symbols, TickerTrigger::Trades);
    }

    fn touch_tickers_with_trigger(&self, symbols: &[String], trigger: TickerTrigger) {
        self.manager.activate_scope("spot-market");
        let now = now_ms();
        let connected = self.connected.load(Ordering::Acquire);
        let mut added_trades = Vec::new();
        let mut added_bbo = Vec::new();
        let mut upgrades = Vec::new();
        let mut exact_subscription_changed = false;
        for symbol in symbols {
            let mut is_new = false;
            let mut upgraded_to_bbo = false;
            self.ticker_subscriptions
                .entry(symbol.clone())
                .and_modify(|state| {
                    state.last_touched_ms = now;
                    if trigger == TickerTrigger::Bbo && state.trigger == TickerTrigger::Trades {
                        state.trigger = TickerTrigger::Bbo;
                        upgraded_to_bbo = true;
                        if connected {
                            if state.sent {
                                upgrades.push(symbol.clone());
                            } else {
                                added_bbo.push(symbol.clone());
                            }
                            state.sent = true;
                        }
                    } else if connected && !state.sent {
                        match state.trigger {
                            TickerTrigger::Trades => added_trades.push(symbol.clone()),
                            TickerTrigger::Bbo => added_bbo.push(symbol.clone()),
                        }
                        state.sent = true;
                    }
                })
                .or_insert_with(|| {
                    is_new = true;
                    SubscriptionState {
                        last_touched_ms: now,
                        sent: connected,
                        trigger,
                    }
                });
            exact_subscription_changed |=
                trigger == TickerTrigger::Bbo && (is_new || upgraded_to_bbo);
            if is_new && connected {
                match trigger {
                    TickerTrigger::Trades => added_trades.push(symbol.clone()),
                    TickerTrigger::Bbo => added_bbo.push(symbol.clone()),
                }
            }
        }
        if !connected {
            // A newly selected exact pair may interrupt an old backoff once.
            // Repeated 100ms projection reads must not keep bypassing the
            // circuit breaker when the network path is unavailable.
            if should_prioritize_exact_reconnect(connected, trigger, exact_subscription_changed) {
                self.manager.prioritize_reconnect();
            }
            return;
        }
        self.spawn_ticker_batches("subscribe", &added_trades, TickerTrigger::Trades);
        self.spawn_ticker_batches("subscribe", &added_bbo, TickerTrigger::Bbo);
        self.spawn_ticker_upgrades(&upgrades);
    }

    pub(super) fn ticker_snapshot(&self, symbols: &[String]) -> Vec<SpotTick> {
        let now = now_ms();
        let last_inbound_ms = self.last_inbound_ms.load(Ordering::Acquire);
        symbols
            .iter()
            .filter_map(|symbol| {
                let row = self.ticks.get(symbol)?;
                let effective_timestamp = effective_market_timestamp(
                    row.observed_at_ms,
                    last_inbound_ms,
                    now,
                    TICKER_STALE_MS,
                )?;
                let mut tick = row.tick.clone();
                tick.received_at_ms = tick.received_at_ms.max(effective_timestamp);
                Some(tick)
            })
            .collect()
    }

    pub(super) fn ticker_problem(&self, symbol: &str) -> Option<String> {
        let now = now_ms();
        let last_inbound_ms = self.last_inbound_ms.load(Ordering::Acquire);
        if self.ticks.get(symbol).is_some_and(|row| {
            effective_market_timestamp(row.observed_at_ms, last_inbound_ms, now, TICKER_STALE_MS)
                .is_some()
        }) {
            return None;
        }
        if !self.connected.load(Ordering::Acquire) {
            if let Some(problem) = self.ticker_problems.get(symbol) {
                return Some(problem.clone());
            }
            return Some(format!("Kraken {symbol} 行情 WS 正在连接"));
        }
        if last_inbound_ms > 0 && now.saturating_sub(last_inbound_ms) > WS_LIVENESS_MAX_AGE_MS {
            return Some(format!(
                "Kraken {symbol} 行情 WS 入站心跳已过期，正在恢复精确交易对订阅"
            ));
        }
        if let Some(problem) = self.ticker_problems.get(symbol) {
            return Some(problem.clone());
        }
        let status = if last_inbound_ms > 0 {
            "WS 已连接，等待精确交易对首个最优买卖价"
        } else {
            "WS 已连接，等待订阅确认"
        };
        Some(format!("Kraken {symbol} {status}"))
    }

    pub(super) fn connection_problem(&self) -> Option<String> {
        self.manager.connection_problem()
    }

    pub(super) fn instruments(&self) -> Arc<Vec<VenueInstrument>> {
        self.instruments.load_full()
    }

    pub(super) fn activate_instrument_stream(&self) {
        self.manager.activate_scope("spot-instrument");
    }

    pub(super) fn touch_book(&self, symbol: &str, requested_depth: usize) {
        self.manager.activate_scope("spot-market");
        let now = now_ms();
        let depth = supported_depth(requested_depth);
        let connected = self.connected.load(Ordering::Acquire);
        let mut subscribe = false;
        let mut reset_from = None;
        self.book_subscriptions
            .entry(symbol.to_owned())
            .and_modify(|state| {
                state.last_touched_ms = now;
                if depth > state.depth {
                    if connected && state.sent {
                        reset_from = Some(state.depth);
                    } else if connected {
                        subscribe = true;
                    }
                    state.depth = depth;
                    state.sent = connected;
                    state.retry_after_ms = 0;
                } else if connected && !state.sent && now >= state.retry_after_ms {
                    state.sent = true;
                    subscribe = true;
                }
            })
            .or_insert_with(|| {
                subscribe = connected;
                BookSubscriptionState {
                    last_touched_ms: now,
                    sent: connected,
                    depth,
                    retry_after_ms: 0,
                }
            });
        if let Some(previous_depth) = reset_from {
            self.books.remove(symbol);
            self.spawn_book_reset(symbol.to_owned(), previous_depth, depth);
        } else if subscribe {
            self.spawn_book_subscription("subscribe", symbol, depth);
        }
    }

    pub(super) fn latest_book(&self, symbol: &str, depth: usize) -> Option<OrderBookInfo> {
        let state = self.books.get(symbol)?;
        let now = now_ms();
        let effective_timestamp = effective_book_timestamp(
            state.timestamp_ms(),
            self.last_inbound_ms.load(Ordering::Acquire),
            now,
        )?;
        let mut snapshot = state.snapshot(depth.max(1));
        snapshot.timestamp = effective_timestamp;
        Some(snapshot)
    }

    pub(super) fn book_problem(&self, symbol: &str) -> Option<String> {
        self.book_problems
            .get(symbol)
            .map(|problem| problem.clone())
    }

    fn spawn_ticker_batches(
        &self,
        operation: &'static str,
        symbols: &[String],
        trigger: TickerTrigger,
    ) {
        if symbols.is_empty() {
            return;
        }
        let commands = ticker_commands(operation, symbols, trigger);
        self.spawn_ticker_commands(commands);
    }

    fn spawn_ticker_upgrades(&self, symbols: &[String]) {
        if symbols.is_empty() {
            return;
        }
        let mut commands = ticker_commands("unsubscribe", symbols, TickerTrigger::Trades);
        commands.extend(ticker_commands("subscribe", symbols, TickerTrigger::Bbo));
        self.spawn_ticker_commands(commands);
    }

    fn spawn_ticker_commands(&self, commands: Vec<TickerCommand>) {
        if commands.is_empty() {
            return;
        }
        let manager = Arc::clone(&self.manager);
        let command_lock = Arc::clone(match commands.first().map(|command| command.trigger) {
            Some(TickerTrigger::Bbo) => &self.ticker_bbo_command_lock,
            _ => &self.ticker_trade_command_lock,
        });
        let subscriptions = Arc::clone(&self.ticker_subscriptions);
        let requests = Arc::clone(&self.ticker_requests);
        let problems = Arc::clone(&self.ticker_problems);
        let connected = Arc::clone(&self.connected);
        let connection_generation = Arc::clone(&self.connection_generation);
        let generation = connection_generation.load(Ordering::Acquire);
        tokio::spawn(async move {
            let _guard = command_lock.lock().await;
            if !connection_is_current(&connected, &connection_generation, generation) {
                return;
            }
            for (index, command) in commands.iter().enumerate() {
                let mut request = command.request.clone();
                request.sent_at_ms = now_ms();
                requests.insert(command.req_id, request);
                if command.request.subscribe {
                    mark_ticker_problem(
                        &problems,
                        &command.request.symbols,
                        "订阅已发送，等待交易所确认",
                    );
                }
                if let Err(error) = manager.send_text(command.payload.clone()).await {
                    requests.remove(&command.req_id);
                    subscriptions
                        .iter_mut()
                        .for_each(|mut row| row.sent = false);
                    if !connection_is_current(&connected, &connection_generation, generation) {
                        debug!(error = %error, "kraken spot ticker subscription interrupted by reconnect");
                        return;
                    }
                    mark_ticker_problem(
                        &problems,
                        &command.request.symbols,
                        &format!("订阅发送失败：{error}"),
                    );
                    if matches!(error, ExchangeError::WsClosed(_)) {
                        debug!(error = %error, "kraken spot ticker subscription interrupted by closed socket");
                    } else {
                        warn!(error = %error, "kraken spot ticker subscription send failed");
                    }
                    return;
                }
                if index + 1 < commands.len() {
                    tokio::time::sleep(TICKER_BATCH_DELAY).await;
                }
            }
        });
    }

    fn spawn_book_subscription(&self, operation: &'static str, symbol: &str, depth: usize) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.book_subscriptions);
        let requests = Arc::clone(&self.book_requests);
        let problems = Arc::clone(&self.book_problems);
        let connected = Arc::clone(&self.connected);
        let connection_generation = Arc::clone(&self.connection_generation);
        let generation = connection_generation.load(Ordering::Acquire);
        let symbol = symbol.to_owned();
        if operation == "subscribe" {
            mark_book_subscription_sent(&subscriptions, &symbol, depth, true);
        }
        tokio::spawn(async move {
            if !connection_is_current(&connected, &connection_generation, generation) {
                return;
            }
            if let Err(error) =
                send_book_command(&manager, &requests, operation, &symbol, depth).await
            {
                if operation == "subscribe" {
                    mark_book_subscription_sent(&subscriptions, &symbol, depth, false);
                    problems.insert(
                        symbol.clone(),
                        format!("Kraken {symbol} 盘口订阅发送失败：{error}"),
                    );
                }
            }
        });
    }

    fn spawn_book_reset(&self, symbol: String, old_depth: usize, new_depth: usize) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.book_subscriptions);
        let requests = Arc::clone(&self.book_requests);
        let problems = Arc::clone(&self.book_problems);
        let connected = Arc::clone(&self.connected);
        let connection_generation = Arc::clone(&self.connection_generation);
        let generation = connection_generation.load(Ordering::Acquire);
        tokio::spawn(async move {
            if !connection_is_current(&connected, &connection_generation, generation) {
                return;
            }
            let _ = send_book_command(&manager, &requests, "unsubscribe", &symbol, old_depth).await;
            problems.insert(symbol.clone(), format!("Kraken {symbol} 盘口正在重新同步"));
            if let Err(error) =
                send_book_command(&manager, &requests, "subscribe", &symbol, new_depth).await
            {
                mark_book_subscription_sent(&subscriptions, &symbol, new_depth, false);
                problems.insert(
                    symbol.clone(),
                    format!("Kraken {symbol} 盘口重订阅发送失败：{error}"),
                );
            }
        });
    }

    async fn run_dispatch(
        self: Arc<Self>,
        mut receiver: tokio::sync::broadcast::Receiver<WsEvent>,
    ) {
        loop {
            match receiver.recv().await {
                Ok(event) => self.handle_event(event),
                Err(RecvError::Lagged(missed)) => {
                    self.clear_market_cache();
                    warn!(missed, "kraken spot ws receiver lagged");
                }
                Err(RecvError::Closed) => return,
            }
        }
    }

    fn handle_event(&self, event: WsEvent) {
        match event {
            WsEvent::Text(text) => {
                self.last_inbound_ms.store(now_ms(), Ordering::Release);
                self.on_text(&text);
            }
            WsEvent::Connected => {
                self.connection_generation.fetch_add(1, Ordering::AcqRel);
                self.connected.store(true, Ordering::Release);
                self.last_inbound_ms.store(0, Ordering::Release);
                self.on_connected();
            }
            WsEvent::Disconnected(reason) => {
                self.connected.store(false, Ordering::Release);
                self.connection_generation.fetch_add(1, Ordering::AcqRel);
                self.last_inbound_ms.store(0, Ordering::Release);
                debug!(%reason, "kraken spot ws disconnected");
                self.clear_market_cache();
                self.mark_all_unsent(Some(&reason));
            }
            WsEvent::CircuitOpened => {
                self.connected.store(false, Ordering::Release);
                self.connection_generation.fetch_add(1, Ordering::AcqRel);
                self.last_inbound_ms.store(0, Ordering::Release);
                self.clear_market_cache();
                self.mark_all_unsent(Some("连续重连失败，已进入退避"));
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        self.mark_all_unsent(None);
        if self.subscribe_instruments {
            spawn_send(Arc::clone(&self.manager), instrument_payload());
        }
        let mut trade_tickers = Vec::new();
        let mut bbo_tickers = Vec::new();
        for mut row in self.ticker_subscriptions.iter_mut() {
            match row.trigger {
                TickerTrigger::Trades => trade_tickers.push(row.key().clone()),
                TickerTrigger::Bbo => bbo_tickers.push(row.key().clone()),
            }
            row.sent = true;
        }
        self.spawn_ticker_batches("subscribe", &trade_tickers, TickerTrigger::Trades);
        self.spawn_ticker_batches("subscribe", &bbo_tickers, TickerTrigger::Bbo);
        let books = self
            .book_subscriptions
            .iter()
            .map(|row| (row.key().clone(), row.depth))
            .collect::<Vec<_>>();
        for (symbol, depth) in books {
            self.spawn_book_subscription("subscribe", &symbol, depth);
        }
    }

    fn apply_book(&self, update: &super::kraken_spot_data::SpotBookUpdate) {
        let depth = self
            .book_subscriptions
            .get(&update.symbol)
            .map_or(10, |row| row.depth);
        if update.snapshot {
            if let Some(state) = SpotBookState::from_snapshot(update, depth) {
                self.books.insert(update.symbol.clone(), state);
                self.book_problems.remove(&update.symbol);
            } else {
                self.reset_book(&update.symbol, depth);
            }
            return;
        }
        let outcome = self
            .books
            .get_mut(&update.symbol)
            .map(|mut state| state.apply(update));
        if !matches!(outcome, Some(MergeOutcome::Applied)) {
            self.reset_book(&update.symbol, depth);
        }
    }

    fn reset_book(&self, symbol: &str, depth: usize) {
        self.books.remove(symbol);
        self.spawn_book_reset(symbol.to_owned(), depth, depth);
    }

    fn mark_all_unsent(&self, problem: Option<&str>) {
        self.ticker_subscriptions
            .iter_mut()
            .for_each(|mut row| row.sent = false);
        self.book_subscriptions
            .iter_mut()
            .for_each(|mut row| row.sent = false);
        self.ticker_requests.clear();
        self.book_requests.clear();
        for row in self.ticker_subscriptions.iter() {
            let symbol = row.key();
            let status = problem.map_or_else(
                || format!("Kraken {symbol} 行情 WS 已重连，等待重新订阅"),
                |problem| format!("Kraken {symbol} 行情 WS 已断开，正在重连：{problem}"),
            );
            self.ticker_problems.insert(symbol.clone(), status);
        }
        if let Some(problem) = problem {
            for row in self.book_subscriptions.iter() {
                let symbol = row.key();
                self.book_problems.insert(
                    symbol.clone(),
                    format!("Kraken {symbol} 盘口 WS 已断开：{problem}"),
                );
            }
        }
    }

    fn clear_market_cache(&self) {
        self.ticks.clear();
        self.books.clear();
    }

    async fn run_cleaner(self: Arc<Self>) {
        let mut interval = tokio::time::interval(CLEAN_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            self.expire_ticker_requests(now_ms());
            self.expire_book_requests(now_ms());
            self.prune_idle().await;
        }
    }

    fn expire_ticker_requests(&self, now: i64) {
        let mut expired = Vec::new();
        self.ticker_requests.retain(|_, request| {
            let keep = now.saturating_sub(request.sent_at_ms) <= TICKER_ACK_TIMEOUT_MS;
            if !keep {
                expired.push(request.clone());
            }
            keep
        });
        for request in expired.into_iter().filter(|request| request.subscribe) {
            for symbol in &request.symbols {
                if let Some(mut state) = self.ticker_subscriptions.get_mut(symbol) {
                    state.sent = false;
                }
            }
            mark_ticker_problem(
                &self.ticker_problems,
                &request.symbols,
                "订阅确认超时，正在重试",
            );
        }
    }

    fn expire_book_requests(&self, now: i64) {
        let mut expired = Vec::new();
        self.book_requests.retain(|_, request| {
            let keep = now.saturating_sub(request.sent_at_ms) <= BOOK_ACK_TIMEOUT_MS;
            if !keep {
                expired.push(request.clone());
            }
            keep
        });
        for request in expired.into_iter().filter(|request| request.subscribe) {
            mark_book_subscription_sent(
                &self.book_subscriptions,
                &request.symbol,
                request.depth,
                false,
            );
            self.book_problems.insert(
                request.symbol.clone(),
                format!(
                    "Kraken {} 盘口订阅未在 {}ms 内确认",
                    request.symbol, BOOK_ACK_TIMEOUT_MS
                ),
            );
        }
    }

    async fn prune_idle(&self) {
        let cutoff = now_ms().saturating_sub(IDLE_TTL_MS);
        let mut trade_tickers = Vec::new();
        let mut bbo_tickers = Vec::new();
        self.ticker_subscriptions.retain(|symbol, state| {
            let keep = state.last_touched_ms >= cutoff;
            if !keep {
                match state.trigger {
                    TickerTrigger::Trades => trade_tickers.push(symbol.clone()),
                    TickerTrigger::Bbo => bbo_tickers.push(symbol.clone()),
                }
                self.ticks.remove(symbol);
                self.ticker_problems.remove(symbol);
            }
            keep
        });
        self.spawn_ticker_batches("unsubscribe", &trade_tickers, TickerTrigger::Trades);
        self.spawn_ticker_batches("unsubscribe", &bbo_tickers, TickerTrigger::Bbo);
        let mut books = Vec::new();
        self.book_subscriptions.retain(|symbol, state| {
            let keep = state.last_touched_ms >= cutoff;
            if !keep {
                books.push((symbol.clone(), state.depth));
                self.books.remove(symbol);
                self.book_problems.remove(symbol);
            }
            keep
        });
        for (symbol, depth) in books {
            self.spawn_book_subscription("unsubscribe", &symbol, depth);
        }
        if self.ticker_subscriptions.is_empty() && self.book_subscriptions.is_empty() {
            self.manager.suspend_scope("spot-market").await;
            if !self.ticker_subscriptions.is_empty() || !self.book_subscriptions.is_empty() {
                self.manager.activate_scope("spot-market");
            }
        }
    }
}

fn should_prioritize_exact_reconnect(
    connected: bool,
    trigger: TickerTrigger,
    subscription_changed: bool,
) -> bool {
    !connected && trigger == TickerTrigger::Bbo && subscription_changed
}

fn effective_book_timestamp(
    book_observed_at_ms: i64,
    last_inbound_ms: i64,
    now_ms: i64,
) -> Option<i64> {
    effective_market_timestamp(book_observed_at_ms, last_inbound_ms, now_ms, BOOK_STALE_MS)
}

fn effective_market_timestamp(
    observed_at_ms: i64,
    last_inbound_ms: i64,
    now_ms: i64,
    stale_ms: i64,
) -> Option<i64> {
    let sample_is_fresh = now_ms.saturating_sub(observed_at_ms) <= stale_ms;
    let stream_is_live =
        last_inbound_ms > 0 && now_ms.saturating_sub(last_inbound_ms) <= WS_LIVENESS_MAX_AGE_MS;
    if stream_is_live {
        return Some(observed_at_ms.max(last_inbound_ms));
    }
    sample_is_fresh.then_some(observed_at_ms)
}

fn mark_book_subscription_sent(
    subscriptions: &DashMap<String, BookSubscriptionState>,
    symbol: &str,
    depth: usize,
    sent: bool,
) {
    if let Some(mut state) = subscriptions.get_mut(symbol) {
        if state.depth == depth {
            state.sent = sent;
            state.retry_after_ms = if sent {
                0
            } else {
                now_ms().saturating_add(BOOK_SUBSCRIPTION_RETRY_MS)
            };
        }
    }
}

async fn send_book_command(
    manager: &WsManager,
    requests: &DashMap<u64, PendingBookRequest>,
    operation: &'static str,
    symbol: &str,
    depth: usize,
) -> Result<(), String> {
    if !manager.is_connected().await {
        return Err("WebSocket 尚未连接".to_owned());
    }
    let req_id = next_request_id();
    requests.insert(
        req_id,
        PendingBookRequest {
            symbol: symbol.to_owned(),
            depth,
            subscribe: operation == "subscribe",
            sent_at_ms: now_ms(),
        },
    );
    if let Err(error) = manager
        .send_text(book_payload(operation, symbol, depth, req_id))
        .await
    {
        requests.remove(&req_id);
        return Err(error.to_string());
    }
    Ok(())
}

fn spawn_send(manager: Arc<WsManager>, payload: String) {
    tokio::spawn(async move {
        wait_until_connected(&manager).await;
        if manager.is_connected().await {
            let _ = manager.send(Message::Text(payload)).await;
        }
    });
}

fn connection_is_current(
    connected: &AtomicBool,
    connection_generation: &AtomicU64,
    expected_generation: u64,
) -> bool {
    connected.load(Ordering::Acquire)
        && connection_generation.load(Ordering::Acquire) == expected_generation
}

async fn wait_until_connected(manager: &WsManager) {
    let deadline = std::time::Instant::now() + CONNECT_WAIT;
    while std::time::Instant::now() < deadline && !manager.is_connected().await {
        tokio::time::sleep(CONNECT_POLL).await;
    }
}

fn next_request_id() -> u64 {
    REQUEST_ID.fetch_add(1, Ordering::Relaxed)
}

fn mark_ticker_problem(problems: &DashMap<String, String>, symbols: &[String], status: &str) {
    for symbol in symbols {
        problems.insert(symbol.clone(), format!("Kraken {symbol} 行情 {status}"));
    }
}

fn instrument_payload() -> String {
    json!({"method":"subscribe","params":{"channel":"instrument","snapshot":true,"include_tokenized_assets":true},"req_id":next_request_id()}).to_string()
}

#[cfg(test)]
fn ticker_payload(operation: &str, symbols: &[String], trigger: TickerTrigger) -> String {
    ticker_payload_with_req_id(operation, symbols, trigger, next_request_id())
}

fn ticker_payload_with_req_id(
    operation: &str,
    symbols: &[String],
    trigger: TickerTrigger,
    req_id: u64,
) -> String {
    let mut params = json!({"channel":"ticker","symbol":symbols,"event_trigger":trigger.as_str()});
    if operation == "subscribe" {
        params["snapshot"] = Value::Bool(true);
    }
    json!({"method":operation,"params":params,"req_id":req_id}).to_string()
}

fn ticker_commands(
    operation: &'static str,
    symbols: &[String],
    trigger: TickerTrigger,
) -> Vec<TickerCommand> {
    symbols
        .chunks(TICKER_BATCH)
        .map(|batch| {
            let req_id = next_request_id();
            TickerCommand {
                req_id,
                payload: ticker_payload_with_req_id(operation, batch, trigger, req_id),
                request: PendingTickerRequest {
                    symbols: batch.to_vec(),
                    subscribe: operation == "subscribe",
                    sent_at_ms: now_ms(),
                },
                trigger,
            }
        })
        .collect()
}

fn book_payload(operation: &str, symbol: &str, depth: usize, req_id: u64) -> String {
    let mut params = json!({"channel":"book","symbol":[symbol],"depth":depth});
    if operation == "subscribe" {
        params["snapshot"] = Value::Bool(true);
    }
    json!({"method":operation,"params":params,"req_id":req_id}).to_string()
}

fn method_ack(root: &Value) -> Option<MethodAck> {
    root.get("method")?.as_str()?;
    let req_id = root.get("req_id")?.as_u64()?;
    let success = root.get("success")?.as_bool()?;
    let error = root.get("error").and_then(Value::as_str).map(str::to_owned);
    Some(MethodAck {
        req_id,
        success,
        error,
    })
}

fn subscription_ack_accepted(ack: &MethodAck) -> bool {
    ack.success
}

fn subscription_already_exists(ack: &MethodAck) -> bool {
    ack.error
        .as_deref()
        .is_some_and(|error| error.eq_ignore_ascii_case("Already subscribed"))
}

fn supported_depth(requested: usize) -> usize {
    match requested.max(1) {
        1..=10 => 10,
        11..=25 => 25,
        26..=100 => 100,
        101..=500 => 500,
        _ => 1_000,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payloads_use_spot_v2_schema_and_explicit_triggers() {
        let symbols = ["BTC/USD".to_owned()];
        let bbo: Value =
            serde_json::from_str(&ticker_payload("subscribe", &symbols, TickerTrigger::Bbo))
                .unwrap();
        assert_eq!(bbo["params"]["event_trigger"], "bbo");
        assert_eq!(bbo["params"]["snapshot"], true);
        let discovery: Value = serde_json::from_str(&ticker_payload(
            "subscribe",
            &symbols,
            TickerTrigger::Trades,
        ))
        .unwrap();
        assert_eq!(discovery["params"]["event_trigger"], "trades");
        let book: Value =
            serde_json::from_str(&book_payload("subscribe", "BTC/USD", 100, 41)).unwrap();
        assert_eq!(book["params"]["depth"], 100);
        assert_eq!(book["params"]["symbol"][0], "BTC/USD");
        assert_eq!(book["req_id"], 41);
        let instrument: Value = serde_json::from_str(&instrument_payload()).unwrap();
        assert_eq!(instrument["params"]["include_tokenized_assets"], true);
    }

    #[test]
    fn ticker_reconfiguration_is_batched_instead_of_sent_per_symbol() {
        let symbols = (0..101)
            .map(|index| format!("ASSET{index}/USD"))
            .collect::<Vec<_>>();
        let commands = ticker_commands("subscribe", &symbols, TickerTrigger::Bbo);

        assert_eq!(commands.len(), 3);
        let sizes = commands
            .iter()
            .map(|command| {
                serde_json::from_str::<Value>(&command.payload).unwrap()["params"]["symbol"]
                    .as_array()
                    .unwrap()
                    .len()
            })
            .collect::<Vec<_>>();
        assert_eq!(sizes, [50, 50, 1]);
        assert_eq!(commands[0].request.symbols.len(), 50);
        assert!(commands.iter().all(|command| command.request.subscribe));
    }

    #[test]
    fn rounds_depth_up_to_supported_kraken_values() {
        assert_eq!(supported_depth(1), 10);
        assert_eq!(supported_depth(11), 25);
        assert_eq!(supported_depth(26), 100);
        assert_eq!(supported_depth(501), 1_000);
    }

    #[test]
    fn failed_book_send_becomes_retryable_without_overwriting_a_new_depth() {
        let subscriptions = DashMap::new();
        subscriptions.insert(
            "PUPS/USD".to_owned(),
            BookSubscriptionState {
                last_touched_ms: 1,
                sent: true,
                depth: 25,
                retry_after_ms: 0,
            },
        );

        mark_book_subscription_sent(&subscriptions, "PUPS/USD", 25, false);
        assert!(!subscriptions.get("PUPS/USD").unwrap().sent);
        assert!(subscriptions.get("PUPS/USD").unwrap().retry_after_ms > now_ms());
        subscriptions.get_mut("PUPS/USD").unwrap().depth = 100;
        subscriptions.get_mut("PUPS/USD").unwrap().sent = true;
        mark_book_subscription_sent(&subscriptions, "PUPS/USD", 25, false);
        assert!(subscriptions.get("PUPS/USD").unwrap().sent);
    }

    #[test]
    fn parses_official_method_ack_success_and_error() {
        let success: Value = serde_json::from_str(
            r#"{"method":"subscribe","result":{"channel":"book","symbol":"PUPS/USD","depth":10},"success":true,"req_id":41}"#,
        )
        .unwrap();
        assert_eq!(
            method_ack(&success),
            Some(MethodAck {
                req_id: 41,
                success: true,
                error: None,
            })
        );

        let failure: Value = serde_json::from_str(
            r#"{"method":"subscribe","success":false,"error":"Currency pair not supported PUPS/USD","req_id":42}"#,
        )
        .unwrap();
        assert_eq!(
            method_ack(&failure),
            Some(MethodAck {
                req_id: 42,
                success: false,
                error: Some("Currency pair not supported PUPS/USD".to_owned()),
            })
        );
        let already_subscribed = MethodAck {
            req_id: 43,
            success: false,
            error: Some("Already subscribed".to_owned()),
        };
        assert!(!subscription_ack_accepted(&already_subscribed));
        assert!(subscription_already_exists(&already_subscribed));
    }

    #[test]
    fn static_book_remains_live_only_while_kraken_heartbeat_is_recent() {
        let now = 200_000;
        assert_eq!(
            effective_book_timestamp(now - 120_000, now - 900, now),
            Some(now - 900)
        );
        assert_eq!(
            effective_book_timestamp(now - 120_000, now - 4_000, now),
            None
        );
        assert_eq!(
            effective_book_timestamp(now - 9_000, 0, now),
            Some(now - 9_000)
        );
        assert_eq!(effective_book_timestamp(now - 11_000, 0, now), None);
        assert_eq!(
            effective_market_timestamp(now - 90_000, now - 800, now, TICKER_STALE_MS),
            Some(now - 800)
        );
        assert_eq!(
            effective_market_timestamp(now - 90_000, now - 4_000, now, TICKER_STALE_MS),
            None
        );
    }

    #[test]
    fn repeated_exact_reads_do_not_bypass_reconnect_backoff() {
        assert!(should_prioritize_exact_reconnect(
            false,
            TickerTrigger::Bbo,
            true
        ));
        assert!(!should_prioritize_exact_reconnect(
            false,
            TickerTrigger::Bbo,
            false
        ));
        assert!(!should_prioritize_exact_reconnect(
            false,
            TickerTrigger::Trades,
            true
        ));
        assert!(!should_prioritize_exact_reconnect(
            true,
            TickerTrigger::Bbo,
            true
        ));
    }
}
