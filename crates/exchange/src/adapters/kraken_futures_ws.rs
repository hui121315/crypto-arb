//! Shared Kraken Derivatives public WebSocket stream.

use super::kraken_config::{KrakenConfig, FUTURES_WS_URL};
use super::kraken_futures_book::{FuturesBookState, MergeOutcome};
use super::kraken_futures_data::FuturesTickerUpdate;
use crate::ws::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use rust_decimal::Decimal;
use serde_json::json;
use shared_types::{FundingRateData, MarkIndexInfo, OrderBookInfo, TickerInfo};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tracing::{debug, warn};

const VENUE: &str = "kraken";
const TICKER_STALE_MS: i64 = 30_000;
const BOOK_STALE_MS: i64 = 10_000;
const TICKER_IDLE_TTL_MS: i64 = 20_000;
const BOOK_IDLE_TTL_MS: i64 = 10_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const CONNECT_WAIT: Duration = Duration::from_secs(5);
const CONNECT_POLL: Duration = Duration::from_millis(100);
const SUBSCRIBE_BATCH: usize = 50;

mod dispatch;

static SHARED: OnceLock<Arc<KrakenFuturesPublicStream>> = OnceLock::new();

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
}

#[derive(Debug, Clone)]
struct CachedTicker {
    update: FuturesTickerUpdate,
    observed_at_ms: i64,
}

#[derive(Debug)]
pub(super) struct KrakenFuturesPublicStream {
    manager: Arc<WsManager>,
    tickers: Arc<DashMap<String, CachedTicker>>,
    books: Arc<DashMap<String, FuturesBookState>>,
    ticker_subscriptions: Arc<DashMap<String, SubscriptionState>>,
    book_subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl KrakenFuturesPublicStream {
    pub(super) fn shared(config: &KrakenConfig) -> Arc<Self> {
        if let Some(url) = &config.futures_ws_url_override {
            return Self::new(url.clone());
        }
        Arc::clone(SHARED.get_or_init(|| Self::new(FUTURES_WS_URL.to_owned())))
    }

    fn new(url: String) -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url,
                exchange: format!("{VENUE}:futures"),
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
            tickers: Arc::new(DashMap::new()),
            books: Arc::new(DashMap::new()),
            ticker_subscriptions: Arc::new(DashMap::new()),
            book_subscriptions: Arc::new(DashMap::new()),
        });
        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "kraken futures ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    pub(super) fn touch_tickers(&self, product_ids: &[String]) {
        if product_ids.is_empty() {
            return;
        }
        self.manager.activate_scope("perp-market");
        let added = touch_subscriptions(&self.ticker_subscriptions, product_ids);
        self.spawn_subscription("subscribe", "ticker", &added);
    }

    pub(super) fn latest_ticker(&self, product_id: &str) -> Option<TickerInfo> {
        self.latest_update(product_id).map(|row| row.ticker)
    }

    pub(super) fn latest_funding(&self, product_id: &str) -> Option<FundingRateData> {
        self.latest_update(product_id).and_then(|row| row.funding)
    }

    pub(super) fn latest_mark_index(&self, product_id: &str) -> Option<MarkIndexInfo> {
        self.latest_update(product_id).map(|row| row.mark_index)
    }

    pub(super) fn ticker_snapshot(&self, product_ids: &[String]) -> Vec<TickerInfo> {
        product_ids
            .iter()
            .filter_map(|id| self.latest_ticker(id))
            .collect()
    }

    pub(super) fn funding_snapshot(&self, product_ids: &[String]) -> Vec<FundingRateData> {
        product_ids
            .iter()
            .filter_map(|id| self.latest_funding(id))
            .collect()
    }

    pub(super) fn mark_index_snapshot(&self, product_ids: &[String]) -> Vec<MarkIndexInfo> {
        product_ids
            .iter()
            .filter_map(|id| self.latest_mark_index(id))
            .collect()
    }

    pub(super) fn touch_book(&self, product_id: &str) {
        self.manager.activate_scope("perp-book");
        let added = touch_subscriptions(&self.book_subscriptions, &[product_id.to_owned()]);
        self.spawn_subscription("subscribe", "book", &added);
    }

    pub(super) fn latest_book(
        &self,
        product_id: &str,
        depth: usize,
        contract_multiplier: Decimal,
    ) -> Option<OrderBookInfo> {
        let state = self.books.get(product_id)?;
        if now_ms().saturating_sub(state.timestamp_ms()) > BOOK_STALE_MS {
            return None;
        }
        Some(state.snapshot(depth.max(1), contract_multiplier))
    }

    fn latest_update(&self, product_id: &str) -> Option<FuturesTickerUpdate> {
        let row = self.tickers.get(product_id)?;
        (now_ms().saturating_sub(row.observed_at_ms) <= TICKER_STALE_MS).then(|| row.update.clone())
    }

    fn spawn_subscription(&self, event: &'static str, feed: &'static str, ids: &[String]) {
        for batch in ids.chunks(SUBSCRIBE_BATCH) {
            if batch.is_empty() {
                continue;
            }
            let payload = subscription_payload(event, feed, batch);
            let manager = Arc::clone(&self.manager);
            tokio::spawn(async move {
                wait_until_connected(&manager).await;
                if manager.is_connected().await {
                    let _ = manager.send_text(payload).await;
                }
            });
        }
    }

    async fn run_dispatch(self: Arc<Self>) {
        let mut receiver = self.manager.subscribe();
        loop {
            match receiver.recv().await {
                Ok(event) => self.handle_event(event),
                Err(RecvError::Lagged(missed)) => {
                    self.clear_cache();
                    warn!(missed, "kraken futures ws receiver lagged");
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
                debug!(%reason, "kraken futures ws disconnected");
                self.clear_cache();
            }
            WsEvent::CircuitOpened => self.clear_cache(),
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let tickers = subscription_keys(&self.ticker_subscriptions);
        let books = subscription_keys(&self.book_subscriptions);
        self.spawn_subscription("subscribe", "ticker", &tickers);
        self.spawn_subscription("subscribe", "book", &books);
    }

    fn apply_book_delta(&self, delta: &super::kraken_futures_data::FuturesBookDelta) {
        let product_id = delta.product_id.clone();
        let outcome = self
            .books
            .get_mut(&product_id)
            .map(|mut state| state.apply(delta));
        match outcome {
            Some(MergeOutcome::Applied | MergeOutcome::Duplicate) => {}
            Some(MergeOutcome::Gap) | None => self.reset_book(product_id),
        }
    }

    fn reset_book(&self, product_id: String) {
        self.books.remove(&product_id);
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                return;
            }
            let ids = [product_id];
            let _ = manager
                .send_text(subscription_payload("unsubscribe", "book", &ids))
                .await;
            let _ = manager
                .send_text(subscription_payload("subscribe", "book", &ids))
                .await;
        });
    }

    fn clear_cache(&self) {
        self.tickers.clear();
        self.books.clear();
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
        let ticker_cutoff = now.saturating_sub(TICKER_IDLE_TTL_MS);
        let book_cutoff = now.saturating_sub(BOOK_IDLE_TTL_MS);
        let mut tickers = Vec::new();
        self.ticker_subscriptions.retain(|id, state| {
            let keep = state.last_touched_ms >= ticker_cutoff;
            if !keep {
                tickers.push(id.clone());
                self.tickers.remove(id);
            }
            keep
        });
        self.spawn_subscription("unsubscribe", "ticker", &tickers);
        let mut books = Vec::new();
        self.book_subscriptions.retain(|id, state| {
            let keep = state.last_touched_ms >= book_cutoff;
            if !keep {
                books.push(id.clone());
                self.books.remove(id);
            }
            keep
        });
        self.spawn_subscription("unsubscribe", "book", &books);
        if self.ticker_subscriptions.is_empty() {
            self.manager.suspend_scope("perp-market").await;
            if !self.ticker_subscriptions.is_empty() {
                self.manager.activate_scope("perp-market");
            }
        }
        if self.book_subscriptions.is_empty() {
            self.manager.suspend_scope("perp-book").await;
            if !self.book_subscriptions.is_empty() {
                self.manager.activate_scope("perp-book");
            }
        }
    }
}

fn touch_subscriptions(
    subscriptions: &DashMap<String, SubscriptionState>,
    ids: &[String],
) -> Vec<String> {
    let now = now_ms();
    let mut added = Vec::new();
    for id in ids {
        let mut is_new = false;
        subscriptions
            .entry(id.clone())
            .and_modify(|state| state.last_touched_ms = now)
            .or_insert_with(|| {
                is_new = true;
                SubscriptionState {
                    last_touched_ms: now,
                }
            });
        if is_new {
            added.push(id.clone());
        }
    }
    added
}

fn subscription_keys(subscriptions: &DashMap<String, SubscriptionState>) -> Vec<String> {
    subscriptions.iter().map(|row| row.key().clone()).collect()
}

async fn wait_until_connected(manager: &WsManager) {
    let deadline = std::time::Instant::now() + CONNECT_WAIT;
    while std::time::Instant::now() < deadline && !manager.is_connected().await {
        tokio::time::sleep(CONNECT_POLL).await;
    }
}

fn subscription_payload(event: &str, feed: &str, product_ids: &[String]) -> String {
    json!({"event":event,"feed":feed,"product_ids":product_ids}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriptions_match_official_futures_schema() {
        let value: serde_json::Value = serde_json::from_str(&subscription_payload(
            "subscribe",
            "ticker",
            &["PF_XBTUSD".to_owned()],
        ))
        .unwrap();
        assert_eq!(value["event"], "subscribe");
        assert_eq!(value["feed"], "ticker");
        assert_eq!(value["product_ids"][0], "PF_XBTUSD");
    }
}
