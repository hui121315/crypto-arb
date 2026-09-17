//! KuCoin Futures Pro `mark-price` and `funding-fee` WebSocket cache.
//!
//! The public Pro endpoint needs no REST token. One connection owns both
//! channels. Independent bounded watchlists prevent the one-minute funding
//! channel from pulling an unnecessary one-second mark feed for every symbol.
//!
//! Official docs:
//! - Base protocol: <https://www.kucoin.com/docs-new/websocket-api/base-info/introduction-uta>
//! - Mark price: <https://www.kucoin.com/docs-new/3470272w0>
//! - Funding fee: <https://www.kucoin.com/docs-new/3470270w0>
//! - Rate limits: <https://www.kucoin.com/docs-new/rate-limit>

use super::kucoin_config::KucoinConfig;
use super::kucoin_ws_mark_index_data::{
    parse_funding_update, parse_mark_index_update, CachedFunding, CachedMarkIndex,
};
use super::kucoin_ws_market::{kucoin_to_normalized, normalized_to_kucoin};
use crate::ws::manager::{WsConfig, WsEvent, WsHeartbeat, WsInboundCodec, WsManager, WsServerPing};
use common::time::now_ms;
use dashmap::DashMap;
use serde_json::json;
use shared_types::{FundingRateData, MarkIndexInfo};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const EXCHANGE: &str = "kucoin";
const PRO_FUTURES_WS_URL: &str = "wss://x-push-futures.kucoin.com";
const CHANNEL_MARK_PRICE: &str = "mark-price";
const CHANNEL_FUNDING_FEE: &str = "funding-fee";
const MARK_CACHE_MAX_AGE_MS: i64 = 10_000;
const FUNDING_CACHE_MAX_AGE_MS: i64 = 90_000;
const MARK_SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const FUNDING_SUBSCRIPTION_IDLE_TTL_MS: i64 = 75_000;
const MAX_SUBSCRIPTIONS_PER_CHANNEL: usize = 64;
const CLEAN_INTERVAL: Duration = Duration::from_secs(5);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);
const COMMAND_INTERVAL: Duration = Duration::from_millis(110);
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

static SHARED_STREAM: OnceLock<Arc<MarkIndexStream>> = OnceLock::new();
static COMMAND_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug)]
pub(crate) struct MarkIndexStream {
    manager: Arc<WsManager>,
    rows: Arc<DashMap<String, CachedMarkIndex>>,
    funding_rows: Arc<DashMap<String, CachedFunding>>,
    mark_subscriptions: Arc<DashMap<String, SubscriptionState>>,
    funding_subscriptions: Arc<DashMap<String, SubscriptionState>>,
    command_gate: Arc<Mutex<Option<Instant>>>,
}

pub(crate) fn snapshot_funding(
    config: &KucoinConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<FundingRateData>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_funding(symbols);
    let now = now_ms();
    let rows: Vec<FundingRateData> = symbols
        .iter()
        .filter_map(|symbol| stream.latest_funding(symbol, now))
        .collect();
    (!rows.is_empty()).then_some(rows)
}

pub(crate) fn snapshot_mark_index(
    config: &KucoinConfig,
    symbols: Option<&[String]>,
) -> Option<Vec<MarkIndexInfo>> {
    let symbols = symbols.filter(|symbols| !symbols.is_empty())?;
    let stream = enabled_stream(config)?;
    stream.touch_mark(symbols);
    let rows = stream.snapshot(symbols);
    (!rows.is_empty()).then_some(rows)
}

fn enabled_stream(config: &KucoinConfig) -> Option<Arc<MarkIndexStream>> {
    if config.base_url_override.is_some() {
        return None;
    }
    Some(Arc::clone(SHARED_STREAM.get_or_init(MarkIndexStream::new)))
}

impl MarkIndexStream {
    fn new() -> Arc<Self> {
        let manager = Arc::new(
            WsManager::new(WsConfig {
                url: PRO_FUTURES_WS_URL.into(),
                exchange: EXCHANGE.into(),
                heartbeat_interval: HEARTBEAT_INTERVAL,
                heartbeat: WsHeartbeat::Text(r#"{"id":"crossline-ping","type":"ping"}"#.into()),
                inbound_codec: WsInboundCodec::Utf8Text,
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
            funding_rows: Arc::new(DashMap::new()),
            mark_subscriptions: Arc::new(DashMap::new()),
            funding_subscriptions: Arc::new(DashMap::new()),
            command_gate: Arc::new(Mutex::new(None)),
        });

        let supervisor = Arc::clone(&manager);
        tokio::spawn(async move {
            if let Err(error) = supervisor.run().await {
                warn!(error = %error, "kucoin pro mark/funding ws supervisor exited");
            }
        });
        tokio::spawn(Arc::clone(&stream).run_dispatch());
        tokio::spawn(Arc::clone(&stream).run_cleaner());
        stream
    }

    fn touch_mark(&self, symbols: &[String]) {
        self.touch_channel(CHANNEL_MARK_PRICE, &self.mark_subscriptions, symbols);
    }

    fn touch_funding(&self, symbols: &[String]) {
        self.touch_channel(CHANNEL_FUNDING_FEE, &self.funding_subscriptions, symbols);
    }

    fn touch_channel(
        &self,
        channel: &'static str,
        subscriptions: &Arc<DashMap<String, SubscriptionState>>,
        symbols: &[String],
    ) {
        if symbols.is_empty() {
            return;
        }
        let scope = if channel == CHANNEL_FUNDING_FEE {
            "funding"
        } else {
            "perp-mark"
        };
        self.manager.activate_scope(scope);
        let now = now_ms();
        let mut new_symbols = Vec::new();
        for symbol in symbols {
            let kucoin = normalized_to_kucoin(symbol);
            let mut needs_subscribe = false;
            subscriptions
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
                new_symbols.push(kucoin);
            }
        }
        for kucoin in new_symbols {
            self.spawn_subscription("SUBSCRIBE", channel, Arc::clone(subscriptions), kucoin);
        }
    }

    fn latest_funding(&self, symbol: &str, now: i64) -> Option<FundingRateData> {
        let kucoin_symbol = normalized_to_kucoin(symbol);
        let row = self.funding_rows.get(&kucoin_symbol)?;
        if !is_fresh(row.cached_at_ms, now, FUNDING_CACHE_MAX_AGE_MS)
            || row.next_funding_time_ms <= now
            || row.interval_hours == 0
        {
            return None;
        }
        Some(FundingRateData {
            symbol: kucoin_to_normalized(&kucoin_symbol),
            exchange: EXCHANGE.into(),
            rate: row.rate,
            rate_8h: row.rate * (8.0 / f64::from(row.interval_hours)),
            predicted_rate: None,
            next_funding_time: row.next_funding_time_ms,
            funding_interval: row.interval_hours,
            volume_24h: 0.0,
            timestamp: row.data_timestamp_ms,
            smoothed_rate: None,
            rate_std: None,
            is_outlier: false,
        })
    }

    fn latest(&self, symbol: &str) -> Option<MarkIndexInfo> {
        let kucoin = normalized_to_kucoin(symbol);
        let row = self.rows.get(&kucoin)?;
        if !is_fresh(row.cached_at_ms, now_ms(), MARK_CACHE_MAX_AGE_MS) {
            return None;
        }
        Some(MarkIndexInfo {
            symbol: kucoin_to_normalized(&kucoin),
            exchange: EXCHANGE.into(),
            mark_price: row.mark_price,
            index_price: row.index_price,
            open_interest: row.open_interest,
            open_interest_value: None,
            timestamp: row.data_timestamp_ms,
        })
    }

    fn snapshot(&self, symbols: &[String]) -> Vec<MarkIndexInfo> {
        symbols
            .iter()
            .filter_map(|symbol| self.latest(symbol))
            .collect()
    }

    fn spawn_subscription(
        &self,
        action: &'static str,
        channel: &'static str,
        subscriptions: Arc<DashMap<String, SubscriptionState>>,
        kucoin_symbol: String,
    ) {
        let manager = Arc::clone(&self.manager);
        let command_gate = Arc::clone(&self.command_gate);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                debug!(action, symbol = %kucoin_symbol, "kucoin pro ws not connected yet");
                return;
            }
            let mut last_sent = command_gate.lock().await;
            let state = subscriptions.get(&kucoin_symbol).map(|entry| *entry);
            if command_is_obsolete(action, state) {
                return;
            }
            pace_command(&mut last_sent).await;
            let payload = command_payload(action, channel, &kucoin_symbol);
            if let Err(error) = manager.send(Message::Text(payload)).await {
                warn!(
                    action,
                    channel,
                    symbol = %kucoin_symbol,
                    error = %error,
                    "kucoin pro ws subscription send failed"
                );
                return;
            }
            *last_sent = Some(Instant::now());
            if action == "SUBSCRIBE" {
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
                warn!(missed, "kucoin pro mark/funding ws receiver lagged");
                self.clear_rows();
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
                self.clear_rows();
                warn!("kucoin pro mark/funding ws circuit opened");
            }
            WsEvent::Binary(_) => {}
        }
    }

    fn on_connected(&self) {
        self.resubscribe_channel(CHANNEL_MARK_PRICE, &self.mark_subscriptions);
        self.resubscribe_channel(CHANNEL_FUNDING_FEE, &self.funding_subscriptions);
    }

    fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "kucoin pro mark/funding ws disconnected");
        self.clear_rows();
        reset_subscription_state(&self.mark_subscriptions);
        reset_subscription_state(&self.funding_subscriptions);
    }

    fn on_text(&self, text: &str) {
        if let Some(parsed) = parse_funding_update(text) {
            self.funding_rows
                .insert(parsed.kucoin_symbol, parsed.cached);
            return;
        }
        if let Some(parsed) = parse_mark_index_update(text) {
            self.rows.insert(parsed.kucoin_symbol, parsed.cached);
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
        self.prune_channel(
            CHANNEL_MARK_PRICE,
            &self.mark_subscriptions,
            &self.rows,
            MARK_SUBSCRIPTION_IDLE_TTL_MS,
        );
        self.prune_channel(
            CHANNEL_FUNDING_FEE,
            &self.funding_subscriptions,
            &self.funding_rows,
            FUNDING_SUBSCRIPTION_IDLE_TTL_MS,
        );
        if self.mark_subscriptions.is_empty() {
            self.manager.suspend_scope("perp-mark").await;
            if !self.mark_subscriptions.is_empty() {
                self.manager.activate_scope("perp-mark");
            }
        }
        if self.funding_subscriptions.is_empty() {
            self.manager.suspend_scope("funding").await;
            if !self.funding_subscriptions.is_empty() {
                self.manager.activate_scope("funding");
            }
        }
    }

    fn clear_rows(&self) {
        self.rows.clear();
        self.funding_rows.clear();
    }

    fn resubscribe_channel(
        &self,
        channel: &'static str,
        subscriptions: &Arc<DashMap<String, SubscriptionState>>,
    ) {
        reset_subscription_state(subscriptions);
        let symbols = subscriptions
            .iter()
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        for symbol in symbols {
            self.spawn_subscription("SUBSCRIBE", channel, Arc::clone(subscriptions), symbol);
        }
    }

    fn prune_channel<T>(
        &self,
        channel: &'static str,
        subscriptions: &Arc<DashMap<String, SubscriptionState>>,
        rows: &DashMap<String, T>,
        idle_ttl_ms: i64,
    ) {
        let cutoff = now_ms().saturating_sub(idle_ttl_ms);
        let mut ordered = subscriptions
            .iter()
            .map(|entry| (entry.key().clone(), entry.last_touched_ms))
            .collect::<Vec<_>>();
        ordered.sort_unstable_by_key(|(_, touched)| std::cmp::Reverse(*touched));
        let mut pruned = Vec::new();
        for (rank, (symbol, touched)) in ordered.into_iter().enumerate() {
            if touched >= cutoff && rank < MAX_SUBSCRIPTIONS_PER_CHANNEL {
                continue;
            }
            if subscriptions.remove(&symbol).is_some() {
                rows.remove(&symbol);
                pruned.push(symbol);
            }
        }
        for symbol in pruned.iter().cloned() {
            self.spawn_subscription("UNSUBSCRIBE", channel, Arc::clone(subscriptions), symbol);
        }
        if !pruned.is_empty() {
            info!(
                channel,
                pruned = pruned.len(),
                remaining = subscriptions.len(),
                "kucoin pro ws idle subscriptions pruned"
            );
        }
    }
}

async fn pace_command(last_sent: &mut Option<Instant>) {
    if let Some(last) = *last_sent {
        tokio::time::sleep(COMMAND_INTERVAL.saturating_sub(last.elapsed())).await;
    }
}

fn command_is_obsolete(action: &str, state: Option<SubscriptionState>) -> bool {
    match action {
        "SUBSCRIBE" => match state {
            Some(state) => state.sent_on_current_connection,
            None => true,
        },
        "UNSUBSCRIBE" => state.is_some(),
        _ => true,
    }
}

fn reset_subscription_state(subscriptions: &DashMap<String, SubscriptionState>) {
    for mut entry in subscriptions.iter_mut() {
        entry.value_mut().sent_on_current_connection = false;
    }
}

async fn wait_until_connected(manager: &WsManager) {
    let deadline = Instant::now() + SUBSCRIBE_CONNECT_WAIT;
    while Instant::now() < deadline {
        if manager.is_connected().await {
            return;
        }
        tokio::time::sleep(SUBSCRIBE_POLL_INTERVAL).await;
    }
}

fn command_payload(action: &str, channel: &str, kucoin_symbol: &str) -> String {
    let id = COMMAND_ID.fetch_add(1, Ordering::Relaxed);
    json!({
        "id": format!("crossline-{id}"),
        "action": action,
        "channel": channel,
        "symbol": kucoin_symbol,
    })
    .to_string()
}

fn is_fresh(cached_at_ms: i64, now: i64, max_age_ms: i64) -> bool {
    cached_at_ms > 0 && cached_at_ms <= now && now.saturating_sub(cached_at_ms) <= max_age_ms
}

#[cfg(test)]
#[path = "kucoin_ws_mark_index_tests.rs"]
mod tests;
