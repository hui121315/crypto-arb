//! Gate.io futures public WebSocket ticker subscriber.
//!
//! Official docs:
//! - Futures WS and SBE API: <https://www.gate.com/docs/developers/futures/ws/en/>
//! - Production schema: <https://github.com/gate/gatews/blob/master/sbe/schemas/prod/gate_fex_ws_latest.xml>

use super::gate_config::GateConfig;
use super::gate_market_data::{parse_mark_index, TickerItem};
use super::gate_public_rest::{SIZE_DECIMAL_HEADER, SIZE_DECIMAL_HEADER_VALUE};
use super::gate_ws_ticker_data::{
    parse_book_update, parse_market_updates, stream_symbol, to_ticker, CachedBookTicker,
    CachedMarket, EXCHANGE,
};
use super::gate_ws_ticker_runtime::GateTickerRuntime;
use super::gate_ws_ticker_sbe::{parse_sbe_update, GateSbeUpdate};
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use shared_types::{MarkIndexInfo, TickerInfo};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tracing::warn;

const WS_URL: &str = "wss://fx-ws.gateio.ws/v4/ws/usdt/sbe?sbe_schema_id=1";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const EVENT_BUFFER_CAPACITY: usize = 8_192;

static SHARED_STREAM: OnceLock<Arc<TickerStream>> = OnceLock::new();
static SBE_DECODE_WARNING_EMITTED: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub(crate) struct TickerStream {
    manager: Arc<WsManager>,
    markets: Arc<DashMap<String, CachedMarket>>,
    books: Arc<DashMap<String, CachedBookTicker>>,
    runtime: GateTickerRuntime,
}

pub(crate) fn latest_ticker(config: &GateConfig, symbol: &str) -> Option<TickerInfo> {
    let stream = enabled_stream(config)?;
    stream.touch_many(&[symbol.to_owned()]);
    stream.latest_ticker(symbol)
}

pub(crate) fn snapshot_tickers(
    config: &GateConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<TickerInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    // 部分新鲜即服务：单个未上市/迟到符号不再把整个 venue 打回 REST。
    let rows = stream.snapshot_tickers(symbols);
    (!rows.is_empty()).then_some(rows)
}

/// Return the latest cached `futures.tickers` row for `symbol` if it is still
/// within `CACHE_MAX_AGE_MS` of arrival.
///
/// Funding interval is **not** part of the row – callers must combine the
/// returned row with the value cached in `GateContractCache::funding_interval_hours`
/// (or fall back to the venue default 8h) before constructing
/// `FundingRateData`.
pub(crate) fn latest_funding_row(config: &GateConfig, symbol: &str) -> Option<TickerItem> {
    let stream = enabled_stream(config)?;
    stream.touch_many(&[symbol.to_owned()]);
    stream.latest_market_row(symbol)
}

/// Snapshot of fresh `futures.tickers` rows for the requested `symbols`.
///
/// Serves the fresh subset (`None` only when nothing is fresh), mirroring
/// `snapshot_tickers`：单符号缺席只影响自己，不再拖垮整个 venue。
pub(crate) fn snapshot_funding_rows(
    config: &GateConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<TickerItem>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let rows = stream.snapshot_market_rows(symbols);
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn snapshot_mark_index(
    config: &GateConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<MarkIndexInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_many(symbols);
    let rows = stream.snapshot_mark_index(symbols);
    (!rows.is_empty()).then_some(rows)
}

fn enabled_stream(config: &GateConfig) -> Option<Arc<TickerStream>> {
    if config.testnet || config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(TickerStream::new)))
}

impl TickerStream {
    fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new_with_event_capacity(
                WsConfig {
                    url: WS_URL.into(),
                    exchange: EXCHANGE.into(),
                    heartbeat_interval: Duration::from_secs(20),
                    heartbeat: WsHeartbeat::PingFrame,
                    inbound_codec: WsInboundCodec::Plain,
                    server_ping: WsServerPing::None,
                    initial_reconnect_delay: Duration::from_secs(1),
                    max_reconnect_delay: Duration::from_secs(30),
                    circuit_breaker_threshold: 10,
                },
                EVENT_BUFFER_CAPACITY,
            )
            .with_connect_header(SIZE_DECIMAL_HEADER, SIZE_DECIMAL_HEADER_VALUE)
            .with_demand_control(),
        );
        let stream = Arc::new(Self {
            manager: Arc::clone(&manager),
            markets: Arc::new(DashMap::new()),
            books: Arc::new(DashMap::new()),
            runtime: GateTickerRuntime::new(Arc::clone(&manager)),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "gate ticker ws supervisor exited");
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
        self.runtime
            .touch_many(symbols, &self.markets, &self.books, CACHE_MAX_AGE_MS);
    }

    fn latest_ticker(&self, symbol: &str) -> Option<TickerInfo> {
        let symbol = stream_symbol(symbol);
        let market = self.markets.get(&symbol)?;
        let book = self.books.get(&symbol)?;
        let now = now_ms();
        if !is_fresh(market.cached_at_ms, now) || !is_fresh(book.cached_at_ms, now) {
            return None;
        }
        to_ticker(&market, &book)
    }

    fn snapshot_tickers(&self, symbols: &[String]) -> Vec<TickerInfo> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_ticker(symbol))
            .collect()
    }

    fn latest_market_row(&self, symbol: &str) -> Option<TickerItem> {
        let row = self.markets.get(&stream_symbol(symbol))?;
        is_fresh(row.cached_at_ms, now_ms()).then(|| row.item.clone())
    }

    fn snapshot_market_rows(&self, symbols: &[String]) -> Vec<TickerItem> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_market_row(symbol))
            .collect()
    }

    fn latest_mark_index(&self, symbol: &str) -> Option<MarkIndexInfo> {
        let row = self.markets.get(&stream_symbol(symbol))?;
        is_fresh(row.cached_at_ms, now_ms())
            .then(|| parse_mark_index(&row.item, row.data_timestamp_ms))
            .flatten()
    }

    fn snapshot_mark_index(&self, symbols: &[String]) -> Vec<MarkIndexInfo> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest_mark_index(symbol))
            .collect()
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
                warn!(missed, "gate ticker ws broadcast receiver lagged");
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
                warn!("gate ticker ws circuit opened");
            }
            WsEvent::Binary(payload) => self.on_binary(&payload),
        }
    }

    fn on_connected(&self) {
        self.runtime.on_connected();
    }

    fn on_disconnected(&self, reason: &str) {
        self.clear_cache();
        self.runtime.on_disconnected(reason);
    }

    fn clear_cache(&self) {
        self.markets.clear();
        self.books.clear();
    }

    fn on_text(&self, text: &str) {
        for row in parse_market_updates(text) {
            self.markets.insert(row.item.contract.clone(), row);
        }
        if let Some(row) = parse_book_update(text) {
            self.books.insert(row.0, row.1);
        }
    }

    fn on_binary(&self, payload: &[u8]) {
        match parse_sbe_update(payload) {
            Ok(Some(GateSbeUpdate::Book(symbol, row))) => {
                self.books.insert(symbol, row);
            }
            Ok(Some(GateSbeUpdate::BookSnapshot(symbol, row))) => {
                self.books.insert(symbol.clone(), row);
                self.runtime.unsubscribe_book_snapshot(symbol);
            }
            Ok(Some(GateSbeUpdate::Markets(rows))) => {
                for row in rows {
                    self.markets.insert(row.item.contract.clone(), row);
                }
            }
            Ok(None) => {}
            Err(error) => {
                if !SBE_DECODE_WARNING_EMITTED.swap(true, Ordering::Relaxed) {
                    warn!(
                        error,
                        "gate ticker SBE frame rejected; REST fallback remains active"
                    );
                }
            }
        }
    }

    async fn run_cleaner(self: Arc<Self>) {
        let mut tick = tokio::time::interval(CLEAN_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            self.runtime.prune_idle(&self.markets, &self.books).await;
        }
    }
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

#[cfg(test)]
#[path = "gate_ws_ticker_tests.rs"]
mod tests;
