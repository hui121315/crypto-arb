use super::gate_ws_ticker_data::{
    channel_payload, order_book_payload, stream_symbol, CachedBookTicker, CachedMarket,
    BOOK_TICKER_CHANNEL, ORDER_BOOK_CHANNEL, TICKERS_CHANNEL,
};
use crate::ws::manager::WsManager;
use common::time::now_ms;
use dashmap::DashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, info, warn};

const MARKET_SNAPSHOT_REFRESH_MS: i64 = 15_000;
const MARKET_MISSING_RECONNECT_MS: i64 = 30_000;
const SNAPSHOT_RECONNECT_COOLDOWN_MS: i64 = 120_000;
const BOOK_SNAPSHOT_REFRESH_MS: i64 = 20_000;
const SUBSCRIPTION_IDLE_TTL_MS: i64 = 20_000;
const SUBSCRIBE_BATCH_SIZE: usize = 50;
const SUBSCRIBE_CONNECT_WAIT: Duration = Duration::from_secs(5);
const SUBSCRIBE_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone, Copy)]
struct SubscriptionState {
    last_touched_ms: i64,
    sent_on_current_connection: bool,
}

#[derive(Debug)]
pub(super) struct GateTickerRuntime {
    manager: Arc<WsManager>,
    subscriptions: Arc<DashMap<String, SubscriptionState>>,
    market_refreshes: Arc<DashMap<String, i64>>,
    market_missing_since: Arc<DashMap<String, i64>>,
    last_snapshot_reconnect_ms: AtomicI64,
    book_bootstraps: Arc<DashMap<String, i64>>,
}

impl GateTickerRuntime {
    pub(super) fn new(manager: Arc<WsManager>) -> Self {
        Self {
            manager,
            subscriptions: Arc::new(DashMap::new()),
            market_refreshes: Arc::new(DashMap::new()),
            market_missing_since: Arc::new(DashMap::new()),
            last_snapshot_reconnect_ms: AtomicI64::new(0),
            book_bootstraps: Arc::new(DashMap::new()),
        }
    }

    pub(super) fn touch_many(
        &self,
        symbols: &[String],
        markets: &DashMap<String, CachedMarket>,
        books: &DashMap<String, CachedBookTicker>,
        cache_max_age_ms: i64,
    ) {
        if symbols.is_empty() {
            return;
        }
        self.manager.activate_scope("perp-market");
        let now = now_ms();
        let mut new_symbols = Vec::new();
        let mut refresh_symbols = Vec::new();
        let mut bootstrap_symbols = Vec::new();
        let mut reconnect_for_snapshot = false;
        for symbol in symbols {
            let symbol = stream_symbol(symbol);
            let needs_subscribe = self.touch_subscription(&symbol, now);
            if needs_subscribe {
                new_symbols.push(symbol.clone());
                self.market_refreshes.insert(symbol.clone(), now);
            } else {
                reconnect_for_snapshot |= self.plan_market_recovery(
                    &symbol,
                    now,
                    markets,
                    cache_max_age_ms,
                    &mut refresh_symbols,
                );
            }
            self.plan_book_snapshot(&symbol, now, books, &mut bootstrap_symbols);
        }
        self.spawn_subscribe_batches(&new_symbols);
        self.spawn_market_refreshes(&refresh_symbols);
        self.spawn_book_snapshots(&bootstrap_symbols);
        if reconnect_for_snapshot && claim_snapshot_reconnect(&self.last_snapshot_reconnect_ms, now)
        {
            self.spawn_snapshot_reconnect();
        }
    }

    fn touch_subscription(&self, symbol: &str, now: i64) -> bool {
        let mut needs_subscribe = false;
        self.subscriptions
            .entry(symbol.to_owned())
            .and_modify(|state| {
                state.last_touched_ms = now;
                needs_subscribe = claim_subscription(state);
            })
            .or_insert_with(|| {
                needs_subscribe = true;
                SubscriptionState {
                    last_touched_ms: now,
                    sent_on_current_connection: true,
                }
            });
        needs_subscribe
    }

    fn plan_market_recovery(
        &self,
        symbol: &str,
        now: i64,
        markets: &DashMap<String, CachedMarket>,
        cache_max_age_ms: i64,
        refresh_symbols: &mut Vec<String>,
    ) -> bool {
        let market_is_fresh = markets
            .get(symbol)
            .is_some_and(|row| now.saturating_sub(row.cached_at_ms) <= cache_max_age_ms);
        let reconnect = if market_is_fresh {
            self.market_missing_since.remove(symbol);
            false
        } else {
            let missing_since = *self
                .market_missing_since
                .entry(symbol.to_owned())
                .or_insert(now);
            now.saturating_sub(missing_since) >= MARKET_MISSING_RECONNECT_MS
        };
        let needs_refresh = markets
            .get(symbol)
            .is_none_or(|row| now.saturating_sub(row.cached_at_ms) >= MARKET_SNAPSHOT_REFRESH_MS);
        if needs_refresh
            && claim_refresh(
                &self.market_refreshes,
                symbol,
                now,
                MARKET_SNAPSHOT_REFRESH_MS,
            )
        {
            refresh_symbols.push(symbol.to_owned());
        }
        reconnect
    }

    fn plan_book_snapshot(
        &self,
        symbol: &str,
        now: i64,
        books: &DashMap<String, CachedBookTicker>,
        bootstrap_symbols: &mut Vec<String>,
    ) {
        let needs_refresh = books
            .get(symbol)
            .is_none_or(|row| now.saturating_sub(row.cached_at_ms) >= BOOK_SNAPSHOT_REFRESH_MS);
        if needs_refresh
            && claim_refresh(&self.book_bootstraps, symbol, now, BOOK_SNAPSHOT_REFRESH_MS)
        {
            bootstrap_symbols.push(symbol.to_owned());
        }
    }

    pub(super) fn on_connected(&self) {
        let mut symbols = Vec::with_capacity(self.subscriptions.len());
        for mut entry in self.subscriptions.iter_mut() {
            if claim_subscription(entry.value_mut()) {
                symbols.push(entry.key().clone());
            }
        }
        self.spawn_subscribe_batches(&symbols);
        let now = now_ms();
        let bootstrap_symbols = symbols
            .iter()
            .filter(|symbol| {
                claim_refresh(&self.book_bootstraps, symbol, now, BOOK_SNAPSHOT_REFRESH_MS)
            })
            .cloned()
            .collect::<Vec<_>>();
        for symbol in symbols {
            self.market_refreshes.insert(symbol, now);
        }
        self.spawn_book_snapshots(&bootstrap_symbols);
    }

    pub(super) fn on_disconnected(&self, reason: &str) {
        debug!(%reason, "gate ticker ws disconnected");
        for mut entry in self.subscriptions.iter_mut() {
            entry.value_mut().sent_on_current_connection = false;
        }
        self.market_refreshes.clear();
        self.market_missing_since.clear();
        self.book_bootstraps.clear();
    }

    pub(super) fn unsubscribe_book_snapshot(&self, symbol: String) {
        self.spawn_subscription("unsubscribe", ORDER_BOOK_CHANNEL, vec![symbol]);
    }

    pub(super) async fn prune_idle(
        &self,
        markets: &DashMap<String, CachedMarket>,
        books: &DashMap<String, CachedBookTicker>,
    ) {
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
        for chunk in pruned.chunks(SUBSCRIBE_BATCH_SIZE) {
            for symbol in chunk {
                markets.remove(symbol);
                books.remove(symbol);
                self.market_refreshes.remove(symbol);
                self.market_missing_since.remove(symbol);
                self.book_bootstraps.remove(symbol);
            }
        }
        if !pruned.is_empty() {
            info!(
                pruned = pruned.len(),
                remaining = self.subscriptions.len(),
                "gate ticker ws idle prune"
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

    fn spawn_subscribe_batches(&self, symbols: &[String]) {
        for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
            self.spawn_subscription("subscribe", TICKERS_CHANNEL, batch.to_vec());
            self.spawn_subscription("subscribe", BOOK_TICKER_CHANNEL, batch.to_vec());
        }
    }

    fn spawn_book_snapshots(&self, symbols: &[String]) {
        for symbol in symbols {
            self.spawn_subscription("subscribe", ORDER_BOOK_CHANNEL, vec![symbol.clone()]);
        }
    }

    fn spawn_market_refreshes(&self, symbols: &[String]) {
        for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
            let symbols = batch.to_vec();
            let manager = Arc::clone(&self.manager);
            let market_refreshes = Arc::clone(&self.market_refreshes);
            tokio::spawn(async move {
                wait_until_connected(&manager).await;
                if !manager.is_connected().await {
                    release_refreshes(&market_refreshes, &symbols);
                    return;
                }
                for op in ["unsubscribe", "subscribe"] {
                    let payload = channel_payload(op, TICKERS_CHANNEL, &symbols);
                    if let Err(error) = manager.send(Message::Text(payload)).await {
                        warn!(
                            op,
                            count = symbols.len(),
                            error = %error,
                            "gate ticker ws snapshot refresh failed"
                        );
                        return;
                    }
                }
            });
        }
    }

    fn spawn_snapshot_reconnect(&self) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if let Err(error) = manager.send(Message::Close(None)).await {
                debug!(error = %error, "gate ticker ws snapshot reconnect deferred");
            }
        });
    }

    fn spawn_subscription(&self, op: &'static str, channel: &'static str, symbols: Vec<String>) {
        if symbols.is_empty() {
            return;
        }
        let manager = Arc::clone(&self.manager);
        let subscriptions = Arc::clone(&self.subscriptions);
        let book_bootstraps = Arc::clone(&self.book_bootstraps);
        tokio::spawn(async move {
            wait_until_connected(&manager).await;
            if !manager.is_connected().await {
                release_subscription_claims(
                    &subscriptions,
                    &book_bootstraps,
                    &symbols,
                    op,
                    channel,
                );
                debug!(
                    channel,
                    count = symbols.len(),
                    "gate ticker ws not connected yet; subscribe resumes after reconnect"
                );
                return;
            }
            let payload = if channel == ORDER_BOOK_CHANNEL {
                order_book_payload(op, &symbols[0])
            } else {
                channel_payload(op, channel, &symbols)
            };
            if let Err(error) = manager.send(Message::Text(payload)).await {
                warn!(
                    op,
                    channel,
                    count = symbols.len(),
                    error = %error,
                    "gate ticker ws subscription send failed"
                );
            }
        });
    }

    fn spawn_idle_unsubscribes(&self, symbols: Vec<String>) {
        let manager = Arc::clone(&self.manager);
        tokio::spawn(async move {
            if !manager.is_connected().await {
                return;
            }
            for batch in symbols.chunks(SUBSCRIBE_BATCH_SIZE) {
                for channel in [TICKERS_CHANNEL, BOOK_TICKER_CHANNEL] {
                    let payload = channel_payload("unsubscribe", channel, batch);
                    if let Err(error) = manager.send(Message::Text(payload)).await {
                        debug!(
                            channel,
                            count = symbols.len(),
                            error = %error,
                            "gate ticker ws idle unsubscribe stopped after disconnect"
                        );
                        return;
                    }
                }
            }
            for symbol in &symbols {
                let payload = order_book_payload("unsubscribe", symbol);
                if let Err(error) = manager.send(Message::Text(payload)).await {
                    debug!(
                        count = symbols.len(),
                        error = %error,
                        "gate ticker ws idle order-book unsubscribe stopped after disconnect"
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

fn claim_subscription(state: &mut SubscriptionState) -> bool {
    if state.sent_on_current_connection {
        return false;
    }
    state.sent_on_current_connection = true;
    true
}

fn release_subscriptions(
    subscriptions: &DashMap<String, SubscriptionState>,
    symbols: &[String],
    op: &str,
) {
    if op != "subscribe" {
        return;
    }
    for symbol in symbols {
        if let Some(mut state) = subscriptions.get_mut(symbol) {
            state.sent_on_current_connection = false;
        }
    }
}

fn claim_refresh(
    refreshes: &DashMap<String, i64>,
    symbol: &str,
    now: i64,
    refresh_ms: i64,
) -> bool {
    let mut claimed = false;
    refreshes
        .entry(symbol.to_owned())
        .and_modify(|last_requested_ms| {
            if now.saturating_sub(*last_requested_ms) >= refresh_ms {
                *last_requested_ms = now;
                claimed = true;
            }
        })
        .or_insert_with(|| {
            claimed = true;
            now
        });
    claimed
}

fn release_refreshes(refreshes: &DashMap<String, i64>, symbols: &[String]) {
    for symbol in symbols {
        refreshes.remove(symbol);
    }
}

fn claim_snapshot_reconnect(last_reconnect_ms: &AtomicI64, now: i64) -> bool {
    let mut previous = last_reconnect_ms.load(Ordering::Relaxed);
    loop {
        if previous > 0 && now.saturating_sub(previous) < SNAPSHOT_RECONNECT_COOLDOWN_MS {
            return false;
        }
        match last_reconnect_ms.compare_exchange_weak(
            previous,
            now,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return true,
            Err(actual) => previous = actual,
        }
    }
}

fn release_subscription_claims(
    subscriptions: &DashMap<String, SubscriptionState>,
    book_bootstraps: &DashMap<String, i64>,
    symbols: &[String],
    op: &str,
    channel: &str,
) {
    if op != "subscribe" {
        return;
    }
    if channel == ORDER_BOOK_CHANNEL {
        release_refreshes(book_bootstraps, symbols);
    } else {
        release_subscriptions(subscriptions, symbols, op);
    }
}

#[cfg(test)]
#[path = "gate_ws_ticker_runtime_tests.rs"]
mod tests;
