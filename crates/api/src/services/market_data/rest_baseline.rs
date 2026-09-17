use super::key::MarketKey;
use dashmap::DashMap;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Mutex, OwnedMutexGuard};

const ORDERBOOK_GUARD_TTL_MS: i64 = 30 * 60 * 1_000;
const ORDERBOOK_GUARD_PRUNE_INTERVAL_MS: i64 = 60_000;
const MAX_ORDERBOOK_GUARD_KEYS: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SnapshotFeed {
    PerpTickers,
    SpotTicks,
}

#[derive(Default)]
pub(crate) struct RestBaselineCoordinator {
    orderbook_locks: DashMap<MarketKey, Arc<OrderbookGuardCell>>,
    snapshot_locks: DashMap<SnapshotFeed, Arc<Mutex<()>>>,
    orderbook_wait_count_total: AtomicU64,
    orderbook_wait_ms_total: AtomicU64,
    orderbook_guard_evicted_total: AtomicU64,
    orderbook_guard_last_prune_ms: AtomicI64,
    snapshot_wait_count_total: AtomicU64,
    snapshot_wait_ms_total: AtomicU64,
}

struct OrderbookGuardCell {
    lock: Arc<Mutex<()>>,
    last_used_ms: AtomicI64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RestBaselineGuardStats {
    pub orderbook_keys: usize,
    pub orderbook_in_flight: usize,
    pub orderbook_wait_count_total: u64,
    pub orderbook_wait_ms_total: u64,
    pub orderbook_guard_evicted_total: u64,
    pub orderbook_guard_oldest_idle_ms: u64,
    pub snapshot_feed_keys: usize,
    pub snapshot_feed_in_flight: usize,
    pub snapshot_wait_count_total: u64,
    pub snapshot_wait_ms_total: u64,
}

impl RestBaselineCoordinator {
    pub(crate) fn stats_snapshot(&self) -> RestBaselineGuardStats {
        let now_ms = common::time::now_ms();
        RestBaselineGuardStats {
            orderbook_keys: self.orderbook_locks.len(),
            orderbook_in_flight: self
                .orderbook_locks
                .iter()
                .filter(|entry| entry.value().lock.try_lock().is_err())
                .count(),
            orderbook_wait_count_total: self.orderbook_wait_count_total.load(Ordering::Relaxed),
            orderbook_wait_ms_total: self.orderbook_wait_ms_total.load(Ordering::Relaxed),
            orderbook_guard_evicted_total: self
                .orderbook_guard_evicted_total
                .load(Ordering::Relaxed),
            orderbook_guard_oldest_idle_ms: self.orderbook_guard_oldest_idle_ms(now_ms),
            snapshot_feed_keys: self.snapshot_locks.len(),
            snapshot_feed_in_flight: self
                .snapshot_locks
                .iter()
                .filter(|entry| entry.value().try_lock().is_err())
                .count(),
            snapshot_wait_count_total: self.snapshot_wait_count_total.load(Ordering::Relaxed),
            snapshot_wait_ms_total: self.snapshot_wait_ms_total.load(Ordering::Relaxed),
        }
    }

    pub(crate) async fn snapshot_guard(&self, feed: SnapshotFeed) -> OwnedMutexGuard<()> {
        let entry = self
            .snapshot_locks
            .entry(feed)
            .or_insert_with(|| Arc::new(Mutex::new(())));
        let lock = Arc::clone(entry.value());
        drop(entry);
        if let Ok(guard) = Arc::clone(&lock).try_lock_owned() {
            return guard;
        }
        let started = Instant::now();
        let guard = lock.lock_owned().await;
        self.record_snapshot_wait(started);
        guard
    }

    pub(crate) fn try_snapshot_guard(&self, feed: SnapshotFeed) -> Option<OwnedMutexGuard<()>> {
        let entry = self
            .snapshot_locks
            .entry(feed)
            .or_insert_with(|| Arc::new(Mutex::new(())));
        let lock = Arc::clone(entry.value());
        drop(entry);
        lock.try_lock_owned().ok()
    }

    pub(crate) async fn orderbook_guard(&self, key: &MarketKey) -> OwnedMutexGuard<()> {
        let now_ms = common::time::now_ms();
        self.prune_orderbook_guards_if_due(now_ms);
        let entry = self
            .orderbook_locks
            .entry(key.clone())
            .or_insert_with(|| Arc::new(OrderbookGuardCell::new(now_ms)));
        let cell = Arc::clone(entry.value());
        drop(entry);
        cell.touch(now_ms);
        let lock = Arc::clone(&cell.lock);
        if let Ok(guard) = Arc::clone(&lock).try_lock_owned() {
            return guard;
        }
        let started = Instant::now();
        let guard = lock.lock_owned().await;
        cell.touch(common::time::now_ms());
        self.record_orderbook_wait(started);
        guard
    }

    fn record_orderbook_wait(&self, started: Instant) {
        self.orderbook_wait_count_total
            .fetch_add(1, Ordering::Relaxed);
        self.orderbook_wait_ms_total
            .fetch_add(elapsed_ms(started), Ordering::Relaxed);
    }

    fn record_snapshot_wait(&self, started: Instant) {
        self.snapshot_wait_count_total
            .fetch_add(1, Ordering::Relaxed);
        self.snapshot_wait_ms_total
            .fetch_add(elapsed_ms(started), Ordering::Relaxed);
    }

    fn prune_orderbook_guards_if_due(&self, now_ms: i64) {
        let last_ms = self.orderbook_guard_last_prune_ms.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last_ms) < ORDERBOOK_GUARD_PRUNE_INTERVAL_MS {
            return;
        }
        if self
            .orderbook_guard_last_prune_ms
            .compare_exchange(last_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            self.prune_orderbook_guards(now_ms);
        }
    }

    fn prune_orderbook_guards(&self, now_ms: i64) {
        let evicted = self.prune_orderbook_guards_by_ttl(now_ms)
            + self.prune_orderbook_guards_by_limit(now_ms, MAX_ORDERBOOK_GUARD_KEYS);
        if evicted > 0 {
            self.orderbook_guard_evicted_total
                .fetch_add(evicted, Ordering::Relaxed);
        }
    }

    fn prune_orderbook_guards_by_ttl(&self, now_ms: i64) -> u64 {
        let mut evicted = 0_u64;
        self.orderbook_locks.retain(|_, cell| {
            let keep = keep_orderbook_guard(cell, now_ms);
            if !keep {
                evicted = evicted.saturating_add(1);
            }
            keep
        });
        evicted
    }

    fn prune_orderbook_guards_by_limit(&self, now_ms: i64, max_keys: usize) -> u64 {
        let excess = self.orderbook_locks.len().saturating_sub(max_keys);
        if excess == 0 {
            return 0;
        }
        let mut candidates: Vec<_> = self
            .orderbook_locks
            .iter()
            .filter(|entry| removable_orderbook_guard(entry.value()))
            .map(|entry| (entry.key().clone(), entry.value().last_used_ms()))
            .collect();
        candidates.sort_unstable_by_key(|(_, last_used_ms)| *last_used_ms);
        candidates
            .into_iter()
            .take(excess)
            .filter(|(key, last_used_ms)| {
                self.remove_orderbook_guard_if_idle(key, *last_used_ms, now_ms)
            })
            .count() as u64
    }

    fn remove_orderbook_guard_if_idle(
        &self,
        key: &MarketKey,
        expected_last_used_ms: i64,
        now_ms: i64,
    ) -> bool {
        self.orderbook_locks
            .remove_if(key, |_, cell| {
                cell.last_used_ms() == expected_last_used_ms
                    && cell.idle_ms(now_ms) > 0
                    && removable_orderbook_guard(cell)
            })
            .is_some()
    }

    fn orderbook_guard_oldest_idle_ms(&self, now_ms: i64) -> u64 {
        self.orderbook_locks
            .iter()
            .map(|entry| entry.value().idle_ms(now_ms))
            .max()
            .unwrap_or(0)
    }

    #[cfg(test)]
    fn set_orderbook_guard_last_used_for_test(&self, key: &MarketKey, last_used_ms: i64) {
        if let Some(entry) = self.orderbook_locks.get(key) {
            entry
                .value()
                .last_used_ms
                .store(last_used_ms, Ordering::Relaxed);
        }
    }

    #[cfg(test)]
    fn prune_orderbook_guards_to_limit_for_test(&self, max_keys: usize, now_ms: i64) {
        let evicted = self.prune_orderbook_guards_by_limit(now_ms, max_keys);
        if evicted > 0 {
            self.orderbook_guard_evicted_total
                .fetch_add(evicted, Ordering::Relaxed);
        }
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
}

impl OrderbookGuardCell {
    fn new(now_ms: i64) -> Self {
        Self {
            lock: Arc::new(Mutex::new(())),
            last_used_ms: AtomicI64::new(now_ms),
        }
    }

    fn touch(&self, now_ms: i64) {
        self.last_used_ms.store(now_ms, Ordering::Relaxed);
    }

    fn idle_ms(&self, now_ms: i64) -> u64 {
        now_ms.saturating_sub(self.last_used_ms()).max(0) as u64
    }

    fn last_used_ms(&self) -> i64 {
        self.last_used_ms.load(Ordering::Relaxed)
    }
}

fn keep_orderbook_guard(cell: &Arc<OrderbookGuardCell>, now_ms: i64) -> bool {
    if cell.idle_ms(now_ms) <= ORDERBOOK_GUARD_TTL_MS as u64 {
        return true;
    }
    !removable_orderbook_guard(cell)
}

fn removable_orderbook_guard(cell: &Arc<OrderbookGuardCell>) -> bool {
    if Arc::strong_count(cell) > 1 {
        return false;
    }
    cell.lock.try_lock().is_ok()
}

#[cfg(test)]
mod tests;
