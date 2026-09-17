//! 基于 [`governor`] 的令牌桶限频器。
//!
//! 每家交易所对应一个 [`RateLimiter`] 实例，通过 [`HttpClient`] 在每次请求前
//! 自动等待。
//!
//! [`HttpClient`]: super::super::http::HttpClient

use dashmap::mapref::entry::Entry;
use dashmap::DashMap;
use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter as GovRateLimiter};
use std::fmt;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

type Inner = GovRateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>;

#[derive(Clone)]
pub struct RateLimiter {
    inner: Arc<Inner>,
    name: &'static str,
    qps: u32,
    parent: Option<Arc<RateLimiter>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimiterSnapshot {
    pub name: String,
    pub qps: u32,
    pub parent: Option<String>,
    pub wait_total: u64,
    pub wait_ms_total: u64,
    pub try_acquire_total: u64,
    pub try_acquire_rejected_total: u64,
    pub last_wait_ms: Option<u64>,
    pub last_wait_observed_at_ms: Option<i64>,
    pub last_rejected_observed_at_ms: Option<i64>,
    pub observed_at_ms: i64,
}

impl fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RateLimiter")
            .field("name", &self.name)
            .field("qps", &self.qps)
            .field("parent", &self.parent.as_ref().map(|parent| parent.name()))
            .finish_non_exhaustive()
    }
}

impl RateLimiter {
    /// 创建按 QPS 限制的限频器。`qps = 0` 会被夹紧到 1。
    pub fn per_second(name: &'static str, qps: u32) -> Self {
        Self::new(name, qps, None)
    }

    pub fn with_shared_budget(
        name: &'static str,
        qps: u32,
        budget_name: &'static str,
        budget_qps: u32,
    ) -> Self {
        Self::new(
            name,
            qps,
            Some(Self::shared_per_second(budget_name, budget_qps)),
        )
    }

    pub fn shared_per_second(name: &'static str, qps: u32) -> Arc<Self> {
        let registry = shared_registry();
        match registry.entry(name) {
            Entry::Occupied(entry) => Arc::clone(entry.get()),
            Entry::Vacant(entry) => {
                let limiter = Arc::new(Self::per_second(name, qps));
                entry.insert(Arc::clone(&limiter));
                limiter
            }
        }
    }

    fn new(name: &'static str, qps: u32, parent: Option<Arc<RateLimiter>>) -> Self {
        let qps = qps.max(1);
        let q = NonZeroU32::new(qps).unwrap_or(NonZeroU32::MIN);
        register_rate_limiter(name, qps, parent.as_ref().map(|parent| parent.name));
        Self {
            inner: Arc::new(GovRateLimiter::direct(Quota::per_second(q))),
            name,
            qps,
            parent,
        }
    }

    pub fn name(&self) -> &str {
        self.name
    }

    pub fn qps(&self) -> u32 {
        self.qps
    }

    /// 等到有 token 可用为止。
    pub async fn wait(&self) {
        self.wait_weight(1).await;
    }

    /// 等到指定权重的 token 可用为止。交易所 REST endpoint 的 request weight
    /// 由 `EndpointSpec` 提供。
    ///
    /// 用 `until_n_ready` 原子等待整块 token（超出桶容量时按容量分块）——
    /// 此前逐 token 循环等待，weight=80 的请求（如 Binance 全量 ticker）要经历
    /// 80 次独立唤醒，仅限流等待即注入约 3 秒延迟。
    pub async fn wait_weight(&self, weight: u32) {
        let started = Instant::now();
        let weight = weight.max(1);
        if let Some(parent) = &self.parent {
            wait_n_chunked(&parent.inner, parent.qps, weight).await;
        }
        wait_n_chunked(&self.inner, self.qps, weight).await;
        self.record_wait(weight, elapsed_ms(started));
    }

    /// 立即尝试获取一个 token；若无可用 token 立即返回 `false`。
    pub fn try_acquire(&self) -> bool {
        let acquired = if self
            .parent
            .as_ref()
            .is_some_and(|parent| parent.inner.check().is_err())
        {
            false
        } else {
            self.inner.check().is_ok()
        };
        self.record_try_acquire(!acquired);
        acquired
    }

    fn record_wait(&self, weight: u32, elapsed_ms: u64) {
        let row = limiter_metrics().entry(self.name).or_insert_with(|| {
            RateLimiterMetric::new(self.name, self.qps, parent_name(&self.parent))
        });
        row.wait_total
            .fetch_add(u64::from(weight), Ordering::Relaxed);
        row.wait_ms_total.fetch_add(elapsed_ms, Ordering::Relaxed);
        row.last_wait_ms.store(elapsed_ms, Ordering::Relaxed);
        row.last_wait_observed_at_ms
            .store(common::time::now_ms(), Ordering::Relaxed);
    }

    fn record_try_acquire(&self, rejected: bool) {
        let row = limiter_metrics().entry(self.name).or_insert_with(|| {
            RateLimiterMetric::new(self.name, self.qps, parent_name(&self.parent))
        });
        row.try_acquire_total.fetch_add(1, Ordering::Relaxed);
        if rejected {
            row.try_acquire_rejected_total
                .fetch_add(1, Ordering::Relaxed);
            row.last_rejected_observed_at_ms
                .store(common::time::now_ms(), Ordering::Relaxed);
        }
    }
}

#[derive(Debug)]
struct RateLimiterMetric {
    name: &'static str,
    qps: AtomicU32,
    parent: Option<&'static str>,
    wait_total: AtomicU64,
    wait_ms_total: AtomicU64,
    try_acquire_total: AtomicU64,
    try_acquire_rejected_total: AtomicU64,
    last_wait_ms: AtomicU64,
    last_wait_observed_at_ms: AtomicI64,
    last_rejected_observed_at_ms: AtomicI64,
}

impl RateLimiterMetric {
    fn new(name: &'static str, qps: u32, parent: Option<&'static str>) -> Self {
        Self {
            name,
            qps: AtomicU32::new(qps),
            parent,
            wait_total: AtomicU64::new(0),
            wait_ms_total: AtomicU64::new(0),
            try_acquire_total: AtomicU64::new(0),
            try_acquire_rejected_total: AtomicU64::new(0),
            last_wait_ms: AtomicU64::new(0),
            last_wait_observed_at_ms: AtomicI64::new(0),
            last_rejected_observed_at_ms: AtomicI64::new(0),
        }
    }

    fn snapshot(&self, observed_at_ms: i64) -> RateLimiterSnapshot {
        RateLimiterSnapshot {
            name: self.name.to_owned(),
            qps: self.qps.load(Ordering::Relaxed),
            parent: self.parent.map(str::to_owned),
            wait_total: self.wait_total.load(Ordering::Relaxed),
            wait_ms_total: self.wait_ms_total.load(Ordering::Relaxed),
            try_acquire_total: self.try_acquire_total.load(Ordering::Relaxed),
            try_acquire_rejected_total: self.try_acquire_rejected_total.load(Ordering::Relaxed),
            last_wait_ms: non_zero_u64(self.last_wait_ms.load(Ordering::Relaxed)),
            last_wait_observed_at_ms: non_zero_i64(
                self.last_wait_observed_at_ms.load(Ordering::Relaxed),
            ),
            last_rejected_observed_at_ms: non_zero_i64(
                self.last_rejected_observed_at_ms.load(Ordering::Relaxed),
            ),
            observed_at_ms,
        }
    }
}

pub fn rate_limiter_snapshots(observed_at_ms: i64) -> Vec<RateLimiterSnapshot> {
    let mut rows = limiter_metrics()
        .iter()
        .map(|entry| entry.value().snapshot(observed_at_ms))
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| left.name.cmp(&right.name));
    rows
}

fn shared_registry() -> &'static DashMap<&'static str, Arc<RateLimiter>> {
    static REGISTRY: OnceLock<DashMap<&'static str, Arc<RateLimiter>>> = OnceLock::new();
    REGISTRY.get_or_init(DashMap::new)
}

fn limiter_metrics() -> &'static DashMap<&'static str, RateLimiterMetric> {
    static METRICS: OnceLock<DashMap<&'static str, RateLimiterMetric>> = OnceLock::new();
    METRICS.get_or_init(DashMap::new)
}

fn register_rate_limiter(name: &'static str, qps: u32, parent: Option<&'static str>) {
    limiter_metrics()
        .entry(name)
        .or_insert_with(|| RateLimiterMetric::new(name, qps, parent));
}

fn parent_name(parent: &Option<Arc<RateLimiter>>) -> Option<&'static str> {
    parent.as_ref().map(|parent| parent.name)
}

/// 原子等待 n 个 token；n 超过桶容量（`until_n_ready` 报 `InsufficientCapacity`）
/// 时按容量分块，仍远优于逐 token 唤醒（80 权重 / 容量 20 → 4 次等待而非 80 次）。
async fn wait_n_chunked(limiter: &Inner, capacity: u32, n: u32) {
    let mut remaining = n;
    let capacity = capacity.max(1);
    while remaining > 0 {
        let chunk = remaining.min(capacity);
        let Some(chunk_n) = NonZeroU32::new(chunk) else {
            break;
        };
        if limiter.until_n_ready(chunk_n).await.is_err() {
            // 容量与配额语义不一致时的兜底：退化为逐 token 等待，绝不 panic。
            for _ in 0..chunk {
                limiter.until_ready().await;
            }
        }
        remaining -= chunk;
    }
}

fn elapsed_ms(started: Instant) -> u64 {
    let elapsed = started.elapsed().as_millis();
    elapsed.min(u128::from(u64::MAX)) as u64
}

fn non_zero_u64(value: u64) -> Option<u64> {
    (value > 0).then_some(value)
}

fn non_zero_i64(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[tokio::test(flavor = "current_thread", start_paused = false)]
    async fn try_acquire_returns_false_when_exhausted() {
        let rl = RateLimiter::per_second("test", 2);
        // 桶初始有 2 个 token
        assert!(rl.try_acquire());
        assert!(rl.try_acquire());
        // 第 3 次应失败
        assert!(!rl.try_acquire());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn wait_eventually_succeeds() {
        let rl = RateLimiter::per_second("test", 5);
        // 耗尽桶
        for _ in 0..5 {
            let _ = rl.try_acquire();
        }
        let start = Instant::now();
        rl.wait().await;
        // wait 不应耗时过长（最多 1s 内补充新 token）
        assert!(start.elapsed().as_millis() < 1500);
    }

    #[test]
    fn shared_per_second_reuses_one_limiter_per_name() {
        let first = RateLimiter::shared_per_second("shared-reuse-test", 2);
        let second = RateLimiter::shared_per_second("shared-reuse-test", 20);

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.qps(), 2);
    }

    #[test]
    fn shared_budget_is_consumed_across_local_limiters() {
        let a = RateLimiter::with_shared_budget("local-a-test", 10, "shared-budget-test", 2);
        let b = RateLimiter::with_shared_budget("local-b-test", 10, "shared-budget-test", 2);

        assert!(a.try_acquire());
        assert!(b.try_acquire());
        assert!(!a.try_acquire());
    }

    #[test]
    fn weighted_try_acquire_consumes_multiple_tokens() {
        let rl = RateLimiter::per_second("weighted-test", 3);

        assert!(rl.try_acquire());
        assert!(rl.try_acquire());
        assert!(rl.try_acquire());
        assert!(!rl.try_acquire());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn snapshot_records_wait_and_try_acquire_rejection() {
        let rl = RateLimiter::per_second("snapshot-test", 1);
        assert!(rl.try_acquire());
        assert!(!rl.try_acquire());

        rl.wait().await;

        let rows = rate_limiter_snapshots(common::time::now_ms());
        let row = rows
            .iter()
            .find(|row| row.name == "snapshot-test")
            .expect("snapshot row");

        assert_eq!(row.qps, 1);
        assert!(row.wait_total >= 1);
        assert!(row.try_acquire_total >= 2);
        assert!(row.try_acquire_rejected_total >= 1);
        assert!(row.last_wait_observed_at_ms.is_some());
        assert!(row.last_rejected_observed_at_ms.is_some());
    }
}
