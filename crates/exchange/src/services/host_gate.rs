use crate::error::ExchangeError;
use dashmap::DashMap;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use tokio::sync::{Mutex, OwnedMutexGuard};

const DEFAULT_CIRCUIT_THRESHOLD: u32 = 5;
const DEFAULT_CIRCUIT_OPEN_MS: i64 = 5_000;
const INFLIGHT_KEY_MAX: usize = 512;
const INFLIGHT_KEY_TARGET: usize = 384;
const INFLIGHT_KEY_TTL_MS: i64 = 60_000;
const INFLIGHT_PRUNE_INTERVAL_MS: i64 = 10_000;
const MIN_RETRY_AFTER_MS: i64 = 1_000;

static HOST_GATES: OnceLock<DashMap<String, Arc<HostGate>>> = OnceLock::new();

#[derive(Debug)]
pub(crate) struct HostGate {
    exchange: String,
    host: String,
    consecutive_failures: AtomicU32,
    circuit_open_until_ms: AtomicI64,
    rate_limit_until_ms: AtomicI64,
    inflight: DashMap<String, InflightEntry>,
    inflight_pruned_total: AtomicU64,
    last_inflight_prune_ms: AtomicI64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostGateSnapshot {
    pub exchange: String,
    pub host: String,
    pub consecutive_failures: u32,
    pub rate_limit_retry_after_ms: Option<u64>,
    pub circuit_retry_after_ms: Option<u64>,
    pub inflight_keys: u64,
    pub inflight_active_keys: u64,
    pub inflight_pruned_total: u64,
    pub inflight_oldest_idle_ms: Option<u64>,
    pub observed_at_ms: i64,
}

#[derive(Debug)]
struct InflightEntry {
    lock: Arc<Mutex<()>>,
    last_used_ms: AtomicI64,
}

impl HostGate {
    pub(crate) fn shared(exchange: &str, host: &str) -> Arc<Self> {
        let key = gate_key(exchange, host);
        host_gates()
            .entry(key)
            .or_insert_with(|| Arc::new(Self::new(exchange, host)))
            .clone()
    }

    pub(crate) fn check_ready(&self, now_ms: i64) -> Result<(), ExchangeError> {
        if let Some(wait_ms) = wait_until(self.rate_limit_until_ms.load(Ordering::Relaxed), now_ms)
        {
            return Err(ExchangeError::RateLimited {
                retry_after_secs: ceil_ms_to_secs(wait_ms),
            });
        }
        if wait_until(self.circuit_open_until_ms.load(Ordering::Relaxed), now_ms).is_some() {
            return Err(ExchangeError::CircuitBreaker {
                exchange: self.exchange.clone(),
            });
        }
        Ok(())
    }

    pub(crate) async fn singleflight(&self, request_key: String) -> OwnedMutexGuard<()> {
        let now_ms = common::time::now_ms();
        self.maybe_prune_inflight(now_ms);
        let entry = self
            .inflight
            .entry(request_key.clone())
            .or_insert_with(|| InflightEntry::new(now_ms));
        entry.value().touch(now_ms);
        let lock = Arc::clone(&entry.value().lock);
        drop(entry);
        let guard = lock.lock_owned().await;
        if let Some(entry) = self.inflight.get(&request_key) {
            entry.value().touch(common::time::now_ms());
        }
        guard
    }

    pub(crate) fn record_success(&self) {
        self.consecutive_failures.store(0, Ordering::Relaxed);
    }

    pub(crate) fn record_error(&self, error: &ExchangeError, now_ms: i64) {
        match error {
            ExchangeError::RateLimited { retry_after_secs } => {
                self.record_rate_limit(*retry_after_secs, now_ms);
            }
            ExchangeError::Http { status, .. } if *status >= 500 => {
                self.record_failure(now_ms);
            }
            ExchangeError::Timeout { .. } | ExchangeError::Network(_) => {
                self.record_failure(now_ms);
            }
            _ => {}
        }
    }

    fn snapshot(&self, now_ms: i64) -> HostGateSnapshot {
        self.maybe_prune_inflight(now_ms);
        HostGateSnapshot {
            exchange: self.exchange.clone(),
            host: self.host.clone(),
            consecutive_failures: self.consecutive_failures.load(Ordering::Relaxed),
            rate_limit_retry_after_ms: wait_until(
                self.rate_limit_until_ms.load(Ordering::Relaxed),
                now_ms,
            )
            .map(wait_ms_to_u64),
            circuit_retry_after_ms: wait_until(
                self.circuit_open_until_ms.load(Ordering::Relaxed),
                now_ms,
            )
            .map(wait_ms_to_u64),
            inflight_keys: self.inflight.len() as u64,
            inflight_active_keys: self.inflight_active_keys() as u64,
            inflight_pruned_total: self.inflight_pruned_total.load(Ordering::Relaxed),
            inflight_oldest_idle_ms: self.inflight_oldest_idle_ms(now_ms),
            observed_at_ms: now_ms,
        }
    }

    fn new(exchange: &str, host: &str) -> Self {
        Self {
            exchange: exchange.to_owned(),
            host: host.to_owned(),
            consecutive_failures: AtomicU32::new(0),
            circuit_open_until_ms: AtomicI64::new(0),
            rate_limit_until_ms: AtomicI64::new(0),
            inflight: DashMap::new(),
            inflight_pruned_total: AtomicU64::new(0),
            last_inflight_prune_ms: AtomicI64::new(0),
        }
    }

    fn record_rate_limit(&self, retry_after_secs: u64, now_ms: i64) {
        let retry_ms = (retry_after_secs as i64)
            .saturating_mul(1_000)
            .max(MIN_RETRY_AFTER_MS);
        self.rate_limit_until_ms
            .store(now_ms.saturating_add(retry_ms), Ordering::Relaxed);
    }

    fn record_failure(&self, now_ms: i64) {
        let failures = self
            .consecutive_failures
            .fetch_add(1, Ordering::Relaxed)
            .saturating_add(1);
        if failures >= DEFAULT_CIRCUIT_THRESHOLD {
            self.circuit_open_until_ms.store(
                now_ms.saturating_add(DEFAULT_CIRCUIT_OPEN_MS),
                Ordering::Relaxed,
            );
            self.consecutive_failures.store(0, Ordering::Relaxed);
        }
    }

    fn maybe_prune_inflight(&self, now_ms: i64) {
        let len = self.inflight.len();
        let last_prune_ms = self.last_inflight_prune_ms.load(Ordering::Relaxed);
        let prune_due = now_ms.saturating_sub(last_prune_ms) >= INFLIGHT_PRUNE_INTERVAL_MS;
        if len <= INFLIGHT_KEY_MAX && !prune_due {
            return;
        }
        if self
            .last_inflight_prune_ms
            .compare_exchange(last_prune_ms, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
            && len <= INFLIGHT_KEY_MAX
        {
            return;
        }
        self.prune_inflight(now_ms);
    }

    fn prune_inflight(&self, now_ms: i64) {
        let mut evicted = self.remove_stale_inflight(now_ms);
        evicted = evicted.saturating_add(self.trim_inflight_to_target());
        if evicted > 0 {
            self.inflight_pruned_total
                .fetch_add(evicted as u64, Ordering::Relaxed);
        }
    }

    fn remove_stale_inflight(&self, now_ms: i64) -> usize {
        let keys = self
            .inflight
            .iter()
            .filter(|entry| {
                entry
                    .value()
                    .safe_to_evict_after(now_ms, INFLIGHT_KEY_TTL_MS)
            })
            .map(|entry| entry.key().clone())
            .collect::<Vec<_>>();
        self.remove_if_idle(keys)
    }

    fn trim_inflight_to_target(&self) -> usize {
        let len = self.inflight.len();
        if len <= INFLIGHT_KEY_MAX {
            return 0;
        }
        let remove_count = len.saturating_sub(INFLIGHT_KEY_TARGET);
        let mut keys = self
            .inflight
            .iter()
            .filter(|entry| entry.value().safe_to_evict())
            .map(|entry| (entry.value().last_used_ms(), entry.key().clone()))
            .collect::<Vec<_>>();
        keys.sort_by_key(|(last_used_ms, _)| *last_used_ms);
        self.remove_if_idle(
            keys.into_iter()
                .take(remove_count)
                .map(|(_, key)| key)
                .collect(),
        )
    }

    fn remove_if_idle(&self, keys: Vec<String>) -> usize {
        keys.into_iter()
            .filter(|key| {
                self.inflight
                    .remove_if(key, |_key, entry| entry.safe_to_evict())
                    .is_some()
            })
            .count()
    }

    fn inflight_oldest_idle_ms(&self, now_ms: i64) -> Option<u64> {
        self.inflight
            .iter()
            .filter(|entry| entry.value().safe_to_evict())
            .map(|entry| wait_ms_to_u64(now_ms.saturating_sub(entry.value().last_used_ms())))
            .max()
    }

    fn inflight_active_keys(&self) -> usize {
        self.inflight
            .iter()
            .filter(|entry| !entry.value().safe_to_evict())
            .count()
    }
}

impl InflightEntry {
    fn new(now_ms: i64) -> Self {
        Self {
            lock: Arc::new(Mutex::new(())),
            last_used_ms: AtomicI64::new(now_ms),
        }
    }

    fn touch(&self, now_ms: i64) {
        self.last_used_ms.store(now_ms, Ordering::Relaxed);
    }

    fn last_used_ms(&self) -> i64 {
        self.last_used_ms.load(Ordering::Relaxed)
    }

    fn safe_to_evict(&self) -> bool {
        Arc::strong_count(&self.lock) == 1
    }

    fn safe_to_evict_after(&self, now_ms: i64, min_idle_ms: i64) -> bool {
        self.safe_to_evict() && now_ms.saturating_sub(self.last_used_ms()) >= min_idle_ms
    }
}

pub fn host_gate_snapshots(now_ms: i64) -> Vec<HostGateSnapshot> {
    let mut rows = host_gates()
        .iter()
        .map(|entry| entry.value().snapshot(now_ms))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        left.exchange
            .cmp(&right.exchange)
            .then_with(|| left.host.cmp(&right.host))
    });
    rows
}

fn host_gates() -> &'static DashMap<String, Arc<HostGate>> {
    HOST_GATES.get_or_init(DashMap::new)
}

fn gate_key(exchange: &str, host: &str) -> String {
    format!("{}:{}", exchange.trim().to_ascii_lowercase(), host)
}

fn wait_until(until_ms: i64, now_ms: i64) -> Option<i64> {
    (until_ms > now_ms).then_some(until_ms - now_ms)
}

fn ceil_ms_to_secs(ms: i64) -> u64 {
    (ms.max(MIN_RETRY_AFTER_MS) as u64).div_ceil(1_000)
}

fn wait_ms_to_u64(ms: i64) -> u64 {
    ms.max(0) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limit_check_returns_remaining_wait() {
        let gate = test_gate();
        gate.record_rate_limit(2, 1_000);

        let err = gate.check_ready(1_500).unwrap_err();

        assert!(matches!(
            err,
            ExchangeError::RateLimited {
                retry_after_secs: 2
            }
        ));
        assert!(gate.check_ready(3_001).is_ok());
    }

    #[test]
    fn repeated_failures_open_circuit() {
        let gate = test_gate();
        for _ in 0..DEFAULT_CIRCUIT_THRESHOLD {
            gate.record_error(
                &ExchangeError::Http {
                    status: 503,
                    body: "down".to_owned(),
                },
                1_000,
            );
        }

        assert!(matches!(
            gate.check_ready(1_001),
            Err(ExchangeError::CircuitBreaker { .. })
        ));
        assert!(gate.check_ready(7_000).is_ok());
    }

    #[tokio::test]
    async fn singleflight_serializes_same_key() {
        let gate = test_gate();
        let first = gate.singleflight("GET:/depth".to_owned()).await;
        let pending = gate.inflight.get("GET:/depth").is_some();
        drop(first);

        assert!(pending);
    }

    #[tokio::test]
    async fn singleflight_keeps_map_entry_while_waiting() {
        let gate = Arc::new(HostGate::new("mock", "api.mock"));
        let first = gate.singleflight("GET:/depth".to_owned()).await;
        let waiting_gate = Arc::clone(&gate);
        let mut waiter =
            tokio::spawn(async move { waiting_gate.singleflight("GET:/depth".to_owned()).await });

        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let result = gate.inflight.try_get("GET:/depth");
        assert!(matches!(result, dashmap::try_result::TryResult::Present(_)));

        let early = tokio::time::timeout(std::time::Duration::from_millis(20), &mut waiter).await;
        assert!(early.is_err());

        drop(first);
        assert!(waiter.await.is_ok());
    }

    #[test]
    fn prune_removes_idle_keys_after_retention() {
        let gate = test_gate();
        insert_inflight(&gate, "GET:/old", 1_000);

        let snapshot = gate.snapshot(1_000 + INFLIGHT_KEY_TTL_MS + 1);

        assert_eq!(snapshot.inflight_keys, 0);
        assert_eq!(snapshot.inflight_pruned_total, 1);
    }

    #[tokio::test]
    async fn prune_keeps_locked_or_waiting_key() {
        let gate = Arc::new(HostGate::new("mock", "api.mock"));
        let first = gate.singleflight("GET:/depth".to_owned()).await;
        let waiting_gate = Arc::clone(&gate);
        let waiter =
            tokio::spawn(async move { waiting_gate.singleflight("GET:/depth".to_owned()).await });
        if let Some(entry) = gate.inflight.get("GET:/depth") {
            entry.value().last_used_ms.store(1_000, Ordering::Relaxed);
        }

        gate.prune_inflight(1_000 + INFLIGHT_KEY_TTL_MS + 1);

        assert_eq!(gate.inflight.len(), 1);
        assert_eq!(gate.inflight_pruned_total.load(Ordering::Relaxed), 0);

        drop(first);
        assert!(waiter.await.is_ok());
    }

    #[test]
    fn prune_trims_oldest_idle_keys_when_cap_exceeded() {
        let gate = test_gate();
        for index in 0..(INFLIGHT_KEY_MAX + 10) {
            insert_inflight(
                &gate,
                &format!("GET:/depth?symbol={index}"),
                10_000 + index as i64,
            );
        }

        gate.prune_inflight(20_000);

        assert!(gate.inflight.len() <= INFLIGHT_KEY_TARGET);
        assert_eq!(
            gate.inflight_pruned_total.load(Ordering::Relaxed),
            (INFLIGHT_KEY_MAX + 10 - INFLIGHT_KEY_TARGET) as u64
        );
    }

    #[test]
    fn snapshot_reports_retry_after_and_inflight_keys() {
        let gate = test_gate();
        gate.record_rate_limit(3, 1_000);
        {
            insert_inflight(&gate, "GET:/depth", 1_000);
        }

        let snapshot = gate.snapshot(1_500);

        assert_eq!(snapshot.exchange, "mock");
        assert_eq!(snapshot.host, "api.mock");
        assert_eq!(snapshot.rate_limit_retry_after_ms, Some(2_500));
        assert_eq!(snapshot.inflight_keys, 1);
        assert_eq!(snapshot.inflight_active_keys, 0);
        assert_eq!(snapshot.inflight_pruned_total, 0);
        assert_eq!(snapshot.inflight_oldest_idle_ms, Some(500));
    }

    fn test_gate() -> HostGate {
        HostGate::new("mock", "api.mock")
    }

    fn insert_inflight(gate: &HostGate, key: &str, last_used_ms: i64) {
        let entry = gate
            .inflight
            .entry(key.to_owned())
            .or_insert_with(|| InflightEntry::new(last_used_ms));
        entry.value().touch(last_used_ms);
        drop(entry);
    }
}
