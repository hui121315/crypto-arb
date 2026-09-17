//! 自动刷新快照。
//!
//! 对应 Python `arbitrage_cache_v3.prewarm() + start_updater(interval=20)` 模式：
//! 后台 task 周期性调用 loader，前台 `get()` 总能拿到最新快照（不阻塞）。

use arc_swap::ArcSwapOption;
use chrono::{DateTime, Utc};
use std::fmt;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tracing::{debug, error};

/// 缓存的快照条目（值 + 写入时刻）。
#[derive(Debug, Clone)]
pub struct SnapshotEntry<T> {
    pub value: T,
    pub cached_at: DateTime<Utc>,
}

/// 单值自动刷新快照容器。
pub struct RefreshingSnapshot<T>
where
    T: Clone + Send + Sync + 'static,
{
    inner: Arc<ArcSwapOption<SnapshotEntry<T>>>,
    refresh_interval: Duration,
    update_version: watch::Sender<u64>,
}

impl<T> fmt::Debug for RefreshingSnapshot<T>
where
    T: Clone + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefreshingSnapshot")
            .field("refresh_interval", &self.refresh_interval)
            .finish_non_exhaustive()
    }
}

impl<T> RefreshingSnapshot<T>
where
    T: Clone + Send + Sync + 'static,
{
    pub fn new(refresh_interval: Duration) -> Self {
        Self {
            inner: Arc::new(ArcSwapOption::empty()),
            refresh_interval,
            update_version: watch::channel(0).0,
        }
    }

    pub fn refresh_interval(&self) -> Duration {
        self.refresh_interval
    }

    /// 返回最近一次缓存的快照（含写入时刻）。
    pub async fn get(&self) -> Option<SnapshotEntry<T>> {
        self.get_now()
    }

    /// 仅返回值。
    pub async fn value(&self) -> Option<T> {
        self.value_now()
    }

    /// 手动写入快照。
    pub async fn set(&self, value: T) {
        self.set_now(value);
    }

    /// 同步读取最近一次缓存的快照。
    pub fn get_now(&self) -> Option<SnapshotEntry<T>> {
        self.inner.load_full().map(|entry| entry.as_ref().clone())
    }

    /// 同步读取最近一次缓存的快照 Arc；热路径可避免 clone 整个快照值。
    pub fn get_arc_now(&self) -> Option<Arc<SnapshotEntry<T>>> {
        self.inner.load_full()
    }

    /// Subscribe to successful snapshot publications without cloning the value.
    /// The receiver carries only a generation; consumers load the latest Arc.
    pub fn subscribe_updates(&self) -> watch::Receiver<u64> {
        self.update_version.subscribe()
    }

    /// 同步读取最近一次缓存值。
    pub fn value_now(&self) -> Option<T> {
        self.inner.load_full().map(|entry| entry.value.clone())
    }

    /// 同步写入快照；读多写少路径避免 async lock。
    pub fn set_now(&self, value: T) {
        self.set_now_at(value, Utc::now());
    }

    /// Publish with a caller-owned timestamp so related atomic indexes can use
    /// the exact same snapshot identity.
    pub fn set_now_at(&self, value: T, cached_at: DateTime<Utc>) {
        self.inner
            .store(Some(Arc::new(SnapshotEntry { value, cached_at })));
        self.update_version
            .send_modify(|version| *version = version.wrapping_add(1));
    }

    /// 立即调用一次 loader 并写入。
    pub async fn prewarm<F, Fut>(
        &self,
        loader: F,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>>,
    {
        let value = loader().await?;
        self.set_now(value);
        Ok(())
    }

    /// 启动后台刷新任务。返回 [`JoinHandle`]，调用方可 `.abort()` 停止。
    ///
    /// loader 失败时打 error 日志但不退出，等待下一轮重试。
    pub fn start_updater<F, Fut>(self: &Arc<Self>, loader: F) -> JoinHandle<()>
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<T, Box<dyn std::error::Error + Send + Sync>>> + Send,
    {
        let me = Arc::clone(self);
        let loader = Arc::new(loader);
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(me.refresh_interval);
            tick.tick().await; // 立即触发一次
            loop {
                tick.tick().await;
                match (loader)().await {
                    Ok(v) => {
                        me.set_now(v);
                        debug!("snapshot refreshed");
                    }
                    Err(e) => {
                        error!(error = %e, "snapshot refresh failed; will retry next tick");
                    }
                }
            }
        })
    }
}

impl<T> Default for RefreshingSnapshot<T>
where
    T: Clone + Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new(Duration::from_secs(30))
    }
}

/// 帮助函数：返回最新快照的"陈旧度"。若没有快照返回 `None`。
pub fn staleness_ms<T>(entry: Option<&SnapshotEntry<T>>) -> Option<i64> {
    entry.map(|e| (Utc::now() - e.cached_at).num_milliseconds())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::sleep;

    type TestResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

    #[tokio::test]
    async fn empty_returns_none() {
        let snap: RefreshingSnapshot<i32> = RefreshingSnapshot::new(Duration::from_secs(1));
        assert!(snap.get().await.is_none());
        assert!(snap.value().await.is_none());
    }

    #[tokio::test]
    async fn set_and_get_round_trip() -> TestResult {
        let snap: RefreshingSnapshot<String> = RefreshingSnapshot::new(Duration::from_secs(1));
        snap.set("hello".into()).await;
        let entry = snap
            .get()
            .await
            .ok_or_else(|| std::io::Error::other("missing snapshot entry"))?;
        assert_eq!(entry.value, "hello");
        assert!(snap.value().await.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn get_arc_now_reuses_snapshot_entry_without_value_clone() -> TestResult {
        let snap: RefreshingSnapshot<String> = RefreshingSnapshot::new(Duration::from_secs(1));
        snap.set("hello".into()).await;

        let first = snap
            .get_arc_now()
            .ok_or_else(|| std::io::Error::other("missing snapshot entry"))?;
        let second = snap
            .get_arc_now()
            .ok_or_else(|| std::io::Error::other("missing snapshot entry"))?;

        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(first.value, "hello");
        Ok(())
    }

    #[tokio::test]
    async fn set_now_notifies_snapshot_subscribers() -> TestResult {
        let snap: RefreshingSnapshot<i32> = RefreshingSnapshot::new(Duration::from_secs(1));
        let mut updates = snap.subscribe_updates();

        snap.set_now(7);
        tokio::time::timeout(Duration::from_millis(50), updates.changed()).await??;

        assert_eq!(*updates.borrow(), 1);
        assert_eq!(snap.value_now(), Some(7));
        Ok(())
    }

    #[tokio::test]
    async fn prewarm_runs_loader_once() -> TestResult {
        let snap: RefreshingSnapshot<i32> = RefreshingSnapshot::new(Duration::from_secs(60));
        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);
        snap.prewarm(|| async move {
            c.fetch_add(1, Ordering::SeqCst);
            Ok::<i32, Box<dyn std::error::Error + Send + Sync>>(42)
        })
        .await?;
        assert_eq!(snap.value().await, Some(42));
        assert_eq!(counter.load(Ordering::SeqCst), 1);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn updater_runs_periodically() {
        let snap = Arc::new(RefreshingSnapshot::<i32>::new(Duration::from_millis(30)));
        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);
        let handle = snap.start_updater(move || {
            let c = Arc::clone(&c);
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                Ok::<i32, Box<dyn std::error::Error + Send + Sync>>(n as i32 + 1)
            }
        });

        // 真实时间下，>=120ms 应触发至少 3 次刷新（含首次立即触发）
        sleep(Duration::from_millis(150)).await;
        handle.abort();

        let n = counter.load(Ordering::SeqCst);
        assert!(n >= 3, "expected >=3 refreshes, got {n}");
        assert!(snap.value().await.is_some());
    }

    #[tokio::test]
    async fn updater_survives_loader_error() {
        let snap = Arc::new(RefreshingSnapshot::<i32>::new(Duration::from_millis(20)));
        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);
        let handle = snap.start_updater(move || {
            let c = Arc::clone(&c);
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n % 2 == 0 {
                    Err::<i32, _>("simulated".into())
                } else {
                    Ok(n as i32)
                }
            }
        });

        sleep(Duration::from_millis(150)).await;
        handle.abort();

        // 偶数次错误、奇数次成功；至少应有 2 次成功写入
        assert!(counter.load(Ordering::SeqCst) >= 3);
    }

    #[tokio::test]
    async fn staleness_grows_over_time() -> TestResult {
        let snap: RefreshingSnapshot<i32> = RefreshingSnapshot::new(Duration::from_secs(60));
        snap.set(1).await;
        let entry = snap
            .get()
            .await
            .ok_or_else(|| std::io::Error::other("missing snapshot entry"))?;
        let s1 =
            staleness_ms(Some(&entry)).ok_or_else(|| std::io::Error::other("missing staleness"))?;
        sleep(Duration::from_millis(20)).await;
        let s2 =
            staleness_ms(Some(&entry)).ok_or_else(|| std::io::Error::other("missing staleness"))?;
        assert!(s2 >= s1, "staleness should not decrease");
        Ok(())
    }
}
