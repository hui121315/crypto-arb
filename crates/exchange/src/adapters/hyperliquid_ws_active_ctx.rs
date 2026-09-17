//! Hyperliquid `activeAssetCtx` WebSocket subscriber.
//!
//! Maintains a single persistent WebSocket connection to
//! `wss://api.hyperliquid.xyz/ws` and caches the latest per-coin context
//! pushed by the `activeAssetCtx` channel: funding rate, mark / mid / oracle
//! price, open interest, 24h notional volume and the daily prev-day price.
//!
//! HL funding settles every UTC hour (`funding_interval = 1h`), so
//! [`latest_funding`] computes `next_funding_time` as the next top-of-hour
//! after [`common::time::now_ms`]; the WS payload itself does not surface a
//! next-funding timestamp.
//!
//! Reference: <https://hyperliquid.gitbook.io/hyperliquid-docs/for-developers/api/websocket/subscriptions>

use super::hyperliquid_market_data::{
    parse_mark_index_for_venue, parse_ticker_for_venue, AssetCtx,
};
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Value};
use shared_types::{FundingRateData, MarkIndexInfo, TickerInfo};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const WS_URL: &str = "wss://api.hyperliquid.xyz/ws";
const CHANNEL: &str = "activeAssetCtx";
const CACHE_MAX_AGE_MS: i64 = 30_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

const HL_FUNDING_INTERVAL_HOURS: u32 = 1;
const HOUR_MS: i64 = 3_600_000;

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug, Clone)]
struct CachedRow {
    item: AssetCtx,
    cached_at_ms: i64,
}

#[derive(Debug)]
pub(super) struct ActiveAssetCtxStream {
    venue: &'static str,
    manager: Arc<WsManager>,
    rows: Arc<DashMap<String, CachedRow>>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
}

impl ActiveAssetCtxStream {
    pub(super) fn new(venue: &'static str) -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: WS_URL.into(),
                exchange: venue.into(),
                heartbeat_interval: Duration::from_secs(30),
                heartbeat: WsHeartbeat::Text(json!({"method": "ping"}).to_string()),
                inbound_codec: WsInboundCodec::Plain,
                server_ping: WsServerPing::None,
                initial_reconnect_delay: Duration::from_secs(1),
                max_reconnect_delay: Duration::from_secs(30),
                circuit_breaker_threshold: 10,
            })
            .with_demand_control(),
        );
        let stream = Arc::new(Self {
            venue,
            manager: Arc::clone(&manager),
            rows: Arc::new(DashMap::new()),
            subscriptions: Arc::new(DashMap::new()),
        });

        let supervisor = Arc::clone(&manager);
        let venue_label = venue;
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(venue = venue_label, error = %error, "hl active asset ctx ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    /// Build a `FundingRateData` row for `coin` if a fresh `activeAssetCtx`
    /// frame is cached. Touches the subscription so the WS keeps the symbol
    /// active. Returns `None` when the cache is empty / stale, or the cached
    /// row's `funding` field is unparsable.
    pub(super) fn latest_funding(&self, coin: &str) -> Option<FundingRateData> {
        self.touch(coin);
        let row = self.rows.get(coin)?;
        if !is_fresh(row.cached_at_ms, now_ms()) {
            return None;
        }
        parse_funding(self.venue, coin, &row.item)
    }

    /// Snapshot funding rows for `coins`. Returns `None` when **any** coin
    /// is missing a fresh row, mirroring the other venue WS funding helpers
    /// so the adapter can decide between WS-fast-path and REST atomically.
    pub(super) fn snapshot_funding(&self, coins: &[String]) -> Option<Vec<FundingRateData>> {
        for coin in coins {
            self.touch(coin);
        }
        let now = now_ms();
        let mut out = Vec::with_capacity(coins.len());
        for coin in coins {
            let row = self.rows.get(coin)?;
            if !is_fresh(row.cached_at_ms, now) {
                return None;
            }
            out.push(parse_funding(self.venue, coin, &row.item)?);
        }
        Some(out)
    }

    /// Build a [`TickerInfo`] for `coin` if a fresh `activeAssetCtx` frame is
    /// cached. Touches the subscription so the WS keeps the symbol active.
    pub(super) fn latest_ticker(&self, coin: &str) -> Option<TickerInfo> {
        self.touch(coin);
        let row = self.rows.get(coin)?;
        if !is_fresh(row.cached_at_ms, now_ms()) {
            return None;
        }
        parse_ticker_for_venue(self.venue, coin, &row.item)
    }

    /// Snapshot ticker rows for `coins`. Returns `None` when **any** coin is
    /// missing a fresh row – the adapter falls back to REST in that case.
    pub(super) fn snapshot_tickers(&self, coins: &[String]) -> Option<Vec<TickerInfo>> {
        for coin in coins {
            self.touch(coin);
        }
        let now = now_ms();
        let mut out = Vec::with_capacity(coins.len());
        for coin in coins {
            let row = self.rows.get(coin)?;
            if !is_fresh(row.cached_at_ms, now) {
                return None;
            }
            if let Some(ticker) = parse_ticker_for_venue(self.venue, coin, &row.item) {
                out.push(ticker);
            }
        }
        Some(out)
    }

    pub(super) fn snapshot_mark_index(&self, coins: &[String]) -> Option<Vec<MarkIndexInfo>> {
        for coin in coins {
            self.touch(coin);
        }
        let now = now_ms();
        let mut out = Vec::with_capacity(coins.len());
        for coin in coins {
            let row = self.rows.get(coin)?;
            if !is_fresh(row.cached_at_ms, now) {
                return None;
            }
            out.push(parse_mark_index_for_venue(self.venue, coin, &row.item)?);
        }
        Some(out)
    }

    fn touch(&self, coin: &str) {
        self.manager.activate_scope("market-context");
        let now = now_ms();
        let mut needs_subscribe = false;
        self.subscriptions
            .entry(coin.to_owned())
            .and_modify(|state| state.last_touched_ms = now)
            .or_insert_with(|| {
                needs_subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: false,
                }
            });
        if needs_subscribe {
            self.spawn_subscribe(coin.to_owned());
        }
    }

    fn spawn_subscribe(&self, coin: String) {
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        let venue = self.venue;
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                debug!(
                    venue,
                    coin = %coin,
                    "hl active asset ctx ws not connected yet; subscribe resumes after reconnect"
                );
                return;
            }
            match manager.send(Message::Text(subscribe_payload(&coin))).await {
                Ok(()) => {
                    if let Some(mut state) = subscriptions.get_mut(&coin) {
                        state.sent_on_current_connection = true;
                    }
                    debug!(venue, coin = %coin, "hl active asset ctx ws subscribed");
                }
                Err(error) => warn!(
                    venue,
                    coin = %coin,
                    error = %error,
                    "hl active asset ctx ws subscribe failed"
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
                warn!(missed, "hl active asset ctx ws broadcast receiver lagged");
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
                warn!("hl active asset ctx ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        let coins: Vec<String> = self
            .subscriptions
            .iter()
            .map(|entry| entry.key().clone())
            .collect();
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
        for coin in coins {
            self.spawn_subscribe(coin);
        }
    }

    fn on_disconnected(&self, reason: &str) {
        self.clear_cache();
        debug!(%reason, "hl active asset ctx ws disconnected");
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
    }

    fn clear_cache(&self) {
        self.rows.clear();
    }

    fn on_text(&self, text: &str) {
        let Some((coin, ctx)) = parse_active_asset_ctx(text) else {
            return;
        };
        self.rows.insert(
            coin,
            CachedRow {
                item: ctx,
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
        let pruned = self.collect_stale_subscriptions();
        self.drop_rows(&pruned);
        self.send_unsubscribes(&pruned).await;
        self.log_prune(&pruned);
        if self.subscriptions.is_empty() {
            self.manager.suspend_scope("market-context").await;
            if !self.subscriptions.is_empty() {
                self.manager.activate_scope("market-context");
            }
        }
    }

    fn collect_stale_subscriptions(&self) -> Vec<String> {
        let cutoff = now_ms().saturating_sub(SUBSCRIPTION_IDLE_TTL_MS);
        let mut pruned = Vec::new();
        self.subscriptions.retain(|coin, state| {
            if state.last_touched_ms < cutoff {
                pruned.push(coin.clone());
                false
            } else {
                true
            }
        });
        pruned
    }

    fn drop_rows(&self, pruned: &[String]) {
        for coin in pruned {
            self.rows.remove(coin);
        }
    }

    async fn send_unsubscribes(&self, pruned: &[String]) {
        for coin in pruned {
            if let Err(error) = self
                .manager
                .send(Message::Text(unsubscribe_payload(coin)))
                .await
            {
                debug!(coin = %coin, error = %error, "hl active asset ctx ws unsubscribe failed");
            }
        }
    }

    fn log_prune(&self, pruned: &[String]) {
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "hl active asset ctx ws idle prune"
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

fn subscribe_payload(coin: &str) -> String {
    json!({
        "method": "subscribe",
        "subscription": {"type": CHANNEL, "coin": coin}
    })
    .to_string()
}

fn unsubscribe_payload(coin: &str) -> String {
    json!({
        "method": "unsubscribe",
        "subscription": {"type": CHANNEL, "coin": coin}
    })
    .to_string()
}

fn is_fresh(cached_at_ms: i64, now: i64) -> bool {
    now.saturating_sub(cached_at_ms) <= CACHE_MAX_AGE_MS
}

/// Returns the next UTC top-of-hour timestamp in ms relative to `now_ms`.
/// HL funding settles at the top of every UTC hour.
fn next_funding_time_ms(now_ms: i64) -> i64 {
    let next_hour = (now_ms / HOUR_MS) + 1;
    next_hour.saturating_mul(HOUR_MS)
}

// AssetCtx is defined once in hyperliquid_market_data.rs and reused by both
// the REST aggregator (`metaAndAssetCtxs`) and this WS subscriber. Reusing it
// lets `parse_ticker_for_venue` build bid/ask from `impactPxs` and gives the
// funding path access to `funding` / `dayNtlVlm` from the same row.

#[derive(Debug, Deserialize)]
struct ActiveAssetCtxData {
    coin: String,
    ctx: AssetCtx,
}

#[derive(Debug, Deserialize)]
struct ActiveAssetCtxEnvelope {
    channel: String,
    #[serde(default)]
    data: Option<Value>,
}

fn parse_active_asset_ctx(text: &str) -> Option<(String, AssetCtx)> {
    let envelope: ActiveAssetCtxEnvelope = serde_json::from_str(text).ok()?;
    if envelope.channel != CHANNEL {
        return None;
    }
    let data: ActiveAssetCtxData = serde_json::from_value(envelope.data?).ok()?;
    Some((data.coin, data.ctx))
}

fn parse_funding(venue: &str, coin: &str, item: &AssetCtx) -> Option<FundingRateData> {
    let rate: f64 = item.funding.parse().ok().filter(|v: &f64| v.is_finite())?;
    let volume_24h = item.day_ntl_vlm.parse().ok().unwrap_or(0.0);
    let now = now_ms();
    Some(FundingRateData {
        symbol: coin.to_owned(),
        exchange: venue.into(),
        rate,
        rate_8h: rate * (8.0 / f64::from(HL_FUNDING_INTERVAL_HOURS)),
        predicted_rate: None,
        next_funding_time: next_funding_time_ms(now),
        funding_interval: HL_FUNDING_INTERVAL_HOURS,
        volume_24h,
        timestamp: now,
        smoothed_rate: None,
        rate_std: None,
        is_outlier: false,
    })
}

#[cfg(test)]
#[path = "hyperliquid_ws_active_ctx_tests.rs"]
mod tests;
