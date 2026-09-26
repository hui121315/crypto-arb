//! One shared, bounded Gate `CrossEx` public WebSocket connection.

use super::gate_crossex_book::{CrossExBookState, MergeOutcome};
use super::gate_crossex_config::{GateCrossExConfig, CROSSEX_PUBLIC_WS_URL};
use super::gate_crossex_data::{
    reference_row, CrossExBookSnapshot, CrossExFundingUpdate, CrossExReferenceUpdate,
    CrossExTickerUpdate,
};
use super::gate_crossex_symbols::CrossExRoute;
use crate::ws::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use rust_decimal::Decimal;
use serde_json::json;
use shared_types::{
    FundingRateData, GateCrossExProduct, GateCrossExRouteQuote, MarkIndexInfo, OrderBookInfo,
    SpotTick, TickerInfo,
};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

const VENUE: &str = "gate_crossex";
const MARKET_STALE_MS: i64 = shared_types::gate_crossex::GATE_CROSSEX_MARKET_MAX_AGE_MS;
const BOOK_STALE_MS: i64 = 10_000;
const MARKET_IDLE_TTL_MS: i64 = 20_000;
const BOOK_IDLE_TTL_MS: i64 = 10_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const CONNECT_POLL: Duration = Duration::from_millis(100);
const SUBSCRIBE_BATCH: usize = 50;

mod dispatch;

static SHARED: OnceLock<Arc<GateCrossExPublicStream>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SubscriptionKey {
    channel: String,
    symbol: String,
}

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
}

#[derive(Debug, Clone)]
struct Timed<T> {
    value: T,
    observed_at_ms: i64,
}

#[derive(Debug, Clone)]
struct ReferenceState {
    route: CrossExRoute,
    mark: Option<(f64, i64)>,
    index: Option<(f64, i64)>,
    open_interest: Option<(f64, Option<f64>, i64)>,
}

#[derive(Debug)]
pub(super) struct GateCrossExPublicStream {
    manager: Arc<WsManager>,
    subscriptions: Arc<DashMap<SubscriptionKey, SubscriptionState>>,
    tickers: Arc<DashMap<String, Timed<CrossExTickerUpdate>>>,
    funding: Arc<DashMap<String, Timed<CrossExFundingUpdate>>>,
    references: Arc<DashMap<String, ReferenceState>>,
    full_books: Arc<DashMap<String, Timed<CrossExBookSnapshot>>>,
    incremental_books: Arc<DashMap<String, CrossExBookState>>,
}

impl GateCrossExPublicStream {
    pub(super) fn shared(config: &GateCrossExConfig) -> Arc<Self> {
        if let Some(url) = &config.public_ws_url_override {
            return Self::new(url.clone());
        }
        Arc::clone(SHARED.get_or_init(|| Self::new(CROSSEX_PUBLIC_WS_URL.to_owned())))
    }

    fn new(url: String) -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url,
                exchange: VENUE.to_owned(),
                heartbeat_interval: Duration::from_secs(20),
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
            subscriptions: Arc::new(DashMap::new()),
            tickers: Arc::new(DashMap::new()),
            funding: Arc::new(DashMap::new()),
            references: Arc::new(DashMap::new()),
            full_books: Arc::new(DashMap::new()),
            incremental_books: Arc::new(DashMap::new()),
        });
        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "Gate CrossEx public WS supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    pub(super) fn touch_tickers(&self, routes: &[String]) {
        self.touch("ticker", routes);
    }

    pub(super) fn touch_funding(&self, routes: &[String]) {
        self.touch("funding_rate", routes);
    }

    pub(super) fn touch_references(&self, routes: &[String]) {
        self.touch("mark_price", routes);
        self.touch("open_interest", routes);
    }

    pub(super) fn touch_book(&self, route: &CrossExRoute, depth: usize) -> bool {
        let Some(channel) = book_channel(route, depth) else {
            return false;
        };
        self.touch(&channel, std::slice::from_ref(&route.native_symbol));
        true
    }

    pub(super) fn ticker_snapshot(&self, routes: &[String]) -> Vec<TickerInfo> {
        routes
            .iter()
            .filter_map(|route| self.fresh_ticker(route))
            .filter(|row| {
                row.route.business == super::gate_crossex_symbols::CrossExBusiness::Future
            })
            .map(|row| row.ticker)
            .collect()
    }

    pub(super) fn spot_snapshot(&self, routes: &[String]) -> Vec<SpotTick> {
        routes
            .iter()
            .filter_map(|route| self.fresh_ticker(route))
            .filter_map(|row| row.spot)
            .collect()
    }

    pub(super) fn route_quote_snapshot(&self, routes: &[String]) -> Vec<GateCrossExRouteQuote> {
        routes
            .iter()
            .filter_map(|route| self.fresh_ticker(route))
            .filter_map(|row| {
                let product = match row.route.business {
                    super::gate_crossex_symbols::CrossExBusiness::Spot => GateCrossExProduct::Spot,
                    super::gate_crossex_symbols::CrossExBusiness::Future => {
                        GateCrossExProduct::Future
                    }
                    super::gate_crossex_symbols::CrossExBusiness::Margin => return None,
                };
                Some(GateCrossExRouteQuote {
                    native_symbol: row.route.native_symbol,
                    underlying_venue: row.route.underlying_venue.to_ascii_lowercase(),
                    product,
                    base_asset: row.route.base,
                    quote_asset: row.route.quote,
                    bid: row.ticker.bid,
                    ask: row.ticker.ask,
                    last: row.ticker.last,
                    observed_at_ms: row.ticker.timestamp,
                })
            })
            .collect()
    }

    pub(super) fn funding_snapshot(
        &self,
        routes: &[String],
        intervals: &HashMap<String, u32>,
    ) -> Vec<FundingRateData> {
        let now = now_ms();
        routes
            .iter()
            .filter_map(|route| {
                let row = self.funding.get(route)?;
                if now.saturating_sub(row.observed_at_ms) > MARKET_STALE_MS {
                    return None;
                }
                let interval = *intervals.get(route)?;
                let volume = self
                    .fresh_ticker(route)
                    .map_or(0.0, |ticker| ticker.ticker.volume_24h);
                Some(row.value.clone().into_row(interval, volume))
            })
            .collect()
    }

    pub(super) fn reference_snapshot(&self, routes: &[String]) -> Vec<MarkIndexInfo> {
        let now = now_ms();
        routes
            .iter()
            .filter_map(|route| {
                let state = self.references.get(route)?;
                let (mark, mark_ts) = state.mark?;
                if now.saturating_sub(mark_ts) > MARKET_STALE_MS {
                    return None;
                }
                let index = state
                    .index
                    .filter(|(_, ts)| now.saturating_sub(*ts) <= MARKET_STALE_MS);
                let oi = state
                    .open_interest
                    .filter(|(_, _, ts)| now.saturating_sub(*ts) <= MARKET_STALE_MS);
                Some(reference_row(
                    &state.route,
                    mark,
                    index.map(|(value, _)| value),
                    oi.map(|(quantity, _, _)| quantity),
                    oi.and_then(|(_, value, _)| value),
                    [
                        mark_ts,
                        index.map_or(0, |(_, ts)| ts),
                        oi.map_or(0, |(_, _, ts)| ts),
                    ]
                    .into_iter()
                    .max()
                    .unwrap_or(mark_ts),
                ))
            })
            .collect()
    }

    pub(super) fn latest_book(
        &self,
        route: &CrossExRoute,
        depth: usize,
        quantity_multiplier: Decimal,
    ) -> Option<OrderBookInfo> {
        let now = now_ms();
        if let Some(row) = self.full_books.get(&route.native_symbol) {
            if now.saturating_sub(row.observed_at_ms) <= BOOK_STALE_MS {
                return Some(row.value.to_orderbook(depth, quantity_multiplier));
            }
        }
        let row = self.incremental_books.get(&route.native_symbol)?;
        (now.saturating_sub(row.timestamp()) <= BOOK_STALE_MS)
            .then(|| row.snapshot(depth, quantity_multiplier))
    }

    pub(super) fn subscribed_symbols(&self, channel: &str) -> Vec<String> {
        self.subscriptions
            .iter()
            .filter(|row| row.key().channel == channel)
            .map(|row| row.key().symbol.clone())
            .collect()
    }

    fn fresh_ticker(&self, route: &str) -> Option<CrossExTickerUpdate> {
        let row = self.tickers.get(route)?;
        (now_ms().saturating_sub(row.observed_at_ms) <= MARKET_STALE_MS).then(|| row.value.clone())
    }

    fn touch(&self, channel: &str, symbols: &[String]) {
        if symbols.is_empty() {
            return;
        }
        self.manager.activate_scope("public-market");
        let now = now_ms();
        let mut added = Vec::new();
        for symbol in symbols {
            let key = SubscriptionKey {
                channel: channel.to_owned(),
                symbol: symbol.clone(),
            };
            let mut is_new = false;
            self.subscriptions
                .entry(key)
                .and_modify(|state| state.last_touched_ms = now)
                .or_insert_with(|| {
                    is_new = true;
                    SubscriptionState {
                        last_touched_ms: now,
                    }
                });
            if is_new {
                added.push(symbol.clone());
            }
        }
        self.spawn_subscription("subscribe", channel, &added);
    }

    fn spawn_subscription(&self, event: &'static str, channel: &str, symbols: &[String]) {
        for batch in symbols.chunks(SUBSCRIBE_BATCH) {
            if batch.is_empty() {
                continue;
            }
            spawn_send(
                Arc::clone(&self.manager),
                subscription_payload(event, channel, batch),
            );
        }
    }

    async fn run_dispatch(self: Arc<Self>) {
        let mut receiver = self.manager.subscribe();
        loop {
            match receiver.recv().await {
                Ok(event) => self.handle_event(event),
                Err(RecvError::Lagged(missed)) => {
                    self.clear_cache();
                    warn!(missed, "Gate CrossEx public WS receiver lagged");
                }
                Err(RecvError::Closed) => return,
            }
        }
    }

    fn handle_event(&self, event: WsEvent) {
        match event {
            WsEvent::Text(text) => self.on_text(&text),
            WsEvent::Connected => self.on_connected(),
            WsEvent::Disconnected(reason) => {
                debug!(%reason, "Gate CrossEx public WS disconnected");
                self.clear_cache();
            }
            WsEvent::CircuitOpened => self.clear_cache(),
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let mut grouped = HashMap::<String, Vec<String>>::new();
        for row in self.subscriptions.iter() {
            grouped
                .entry(row.key().channel.clone())
                .or_default()
                .push(row.key().symbol.clone());
        }
        for (channel, symbols) in grouped {
            self.spawn_subscription("subscribe", &channel, &symbols);
        }
    }

    fn apply_reference(&self, update: CrossExReferenceUpdate) {
        match update {
            CrossExReferenceUpdate::Mark {
                route,
                value,
                timestamp,
            } => {
                self.references
                    .entry(route.native_symbol.clone())
                    .and_modify(|state| state.mark = Some((value, timestamp)))
                    .or_insert(ReferenceState {
                        route,
                        mark: Some((value, timestamp)),
                        index: None,
                        open_interest: None,
                    });
            }
            CrossExReferenceUpdate::Index {
                route,
                value,
                timestamp,
            } => {
                self.references
                    .entry(route.native_symbol.clone())
                    .and_modify(|state| state.index = Some((value, timestamp)))
                    .or_insert(ReferenceState {
                        route,
                        mark: None,
                        index: Some((value, timestamp)),
                        open_interest: None,
                    });
            }
            CrossExReferenceUpdate::OpenInterest {
                route,
                quantity,
                value,
                timestamp,
            } => {
                self.references
                    .entry(route.native_symbol.clone())
                    .and_modify(|state| {
                        state.open_interest = Some((quantity, value, timestamp));
                    })
                    .or_insert(ReferenceState {
                        route,
                        mark: None,
                        index: None,
                        open_interest: Some((quantity, value, timestamp)),
                    });
            }
        }
    }

    fn apply_incremental_book(&self, update: &super::gate_crossex_data::CrossExBookDelta) {
        let symbol = update.route.native_symbol.clone();
        if update.snapshot {
            if let Some(state) = CrossExBookState::from_snapshot(update) {
                self.incremental_books.insert(symbol, state);
            }
            return;
        }
        let outcome = self
            .incremental_books
            .get_mut(&symbol)
            .map(|mut state| state.apply(update));
        if matches!(outcome, Some(MergeOutcome::Gap)) {
            self.incremental_books.remove(&symbol);
            self.reset_subscription("order_book_update", symbol);
        }
    }

    fn reset_subscription(&self, channel: &'static str, symbol: String) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                return;
            }
            let rows = [symbol];
            let _ = manager
                .send_text(subscription_payload("unsubscribe", channel, &rows))
                .await;
            let _ = manager
                .send_text(subscription_payload("subscribe", channel, &rows))
                .await;
        });
    }

    fn clear_cache(&self) {
        self.tickers.clear();
        self.funding.clear();
        self.references.clear();
        self.full_books.clear();
        self.incremental_books.clear();
    }

    async fn run_cleaner(self: Arc<Self>) {
        let mut interval = tokio::time::interval(CLEAN_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            self.prune_idle().await;
        }
    }

    async fn prune_idle(&self) {
        let now = now_ms();
        let mut removed = HashMap::<String, Vec<String>>::new();
        self.subscriptions.retain(|key, state| {
            let ttl = if key.channel.starts_with("order_book_") {
                BOOK_IDLE_TTL_MS
            } else {
                MARKET_IDLE_TTL_MS
            };
            let keep = state.last_touched_ms >= now.saturating_sub(ttl);
            if !keep {
                removed
                    .entry(key.channel.clone())
                    .or_default()
                    .push(key.symbol.clone());
            }
            keep
        });
        for (channel, symbols) in removed {
            self.spawn_subscription("unsubscribe", &channel, &symbols);
        }
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("public-market").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("public-market");
            }
        }
    }
}

fn book_channel(route: &CrossExRoute, requested_depth: usize) -> Option<String> {
    let supported: &[usize] = match route.underlying_venue.as_str() {
        "BINANCE" => &[5, 10, 20],
        "OKX" => &[1, 5],
        "GATE" => &[1, 5, 10, 20, 50, 100],
        "BYBIT" => &[1],
        "KRAKEN" => return Some("order_book_update".to_owned()),
        "HYPERLIQUID" => &[1, 5, 10, 20, 30, 50, 100, 400],
        "DERIBIT" => &[1, 5, 10, 20],
        _ => return None,
    };
    let depth = supported
        .iter()
        .copied()
        .find(|depth| *depth >= requested_depth.max(1))
        .unwrap_or_else(|| *supported.last().unwrap_or(&1));
    Some(format!("order_book_{depth}"))
}

fn spawn_send(manager: Arc<WsManager>, payload: String) {
    tokio::spawn(async move {
        wait_until_connected(&manager).await;
        if manager.is_connected().await {
            let _ = manager.send_text(payload).await;
        }
    });
}

async fn wait_until_connected(manager: &WsManager) {
    let deadline = std::time::Instant::now() + CONNECT_WAIT;
    while std::time::Instant::now() < deadline && !manager.is_connected().await {
        tokio::time::sleep(CONNECT_POLL).await;
    }
}

fn subscription_payload(event: &str, channel: &str, symbols: &[String]) -> String {
    json!({
        "time": common::time::now_ms() / 1_000,
        "event": event,
        "channel": channel,
        "payload": symbols,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn subscriptions_include_required_time_and_never_subscribe_all() {
        let payload: Value = serde_json::from_str(&subscription_payload(
            "subscribe",
            "ticker",
            &["GATE_FUTURE_BTC_USDT".to_owned()],
        ))
        .unwrap();
        assert!(payload["time"].as_i64().unwrap() > 0);
        assert_eq!(payload["event"], "subscribe");
        assert_eq!(payload["payload"][0], "GATE_FUTURE_BTC_USDT");
        assert_ne!(payload["payload"][0], "!all");
    }

    #[test]
    fn depth_rounding_matches_official_per_underlying_limits() {
        let gate = CrossExRoute::parse("GATE_FUTURE_BTC_USDT").unwrap();
        let bybit = CrossExRoute::parse("BYBIT_FUTURE_BTC_USDT").unwrap();
        let kraken = CrossExRoute::parse("KRAKEN_FUTURE_BTC_USD").unwrap();
        assert_eq!(book_channel(&gate, 6).as_deref(), Some("order_book_10"));
        assert_eq!(book_channel(&bybit, 20).as_deref(), Some("order_book_1"));
        assert_eq!(
            book_channel(&kraken, 20).as_deref(),
            Some("order_book_update")
        );
    }

    #[tokio::test]
    async fn public_socket_is_dormant_until_a_native_route_is_requested() {
        let stream = GateCrossExPublicStream::new("ws://127.0.0.1:1".to_owned());
        assert!(!stream.manager.is_active());

        stream.touch_tickers(&["GATE_FUTURE_BTC_USDT".to_owned()]);
        assert!(stream.manager.is_active());

        stream.subscriptions.clear();
        stream.prune_idle().await;
        assert!(!stream.manager.is_active());
    }
}
