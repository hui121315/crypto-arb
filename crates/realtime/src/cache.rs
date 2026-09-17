//! 通用 TTL 内存缓存（基于 [`moka::future::Cache`]）。
//!
//! 与 Python `analysis-service/services/arbitrage_cache_v3.py` 的 30s TTL 对齐。

use moka::future::Cache;
use std::fmt;
use std::hash::Hash;
use std::time::Duration;

/// 通用 K-V 缓存，每条记录带 TTL。
#[derive(Clone)]
pub struct TtlCache<K, V>
where
    K: Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    inner: Cache<K, V>,
}

impl<K, V> fmt::Debug for TtlCache<K, V>
where
    K: Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TtlCache")
            .field("entry_count", &self.entry_count())
            .finish_non_exhaustive()
    }
}

impl<K, V> TtlCache<K, V>
where
    K: Eq + Hash + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    /// `max_capacity`：最大条目数；`ttl`：单条记录的存活时间。
    pub fn new(max_capacity: u64, ttl: Duration) -> Self {
        let inner = Cache::builder()
            .max_capacity(max_capacity)
            .time_to_live(ttl)
            .build();
        Self { inner }
    }

    pub async fn get(&self, key: &K) -> Option<V> {
        self.inner.get(key).await
    }

    pub async fn insert(&self, key: K, value: V) {
        self.inner.insert(key, value).await;
    }

    pub async fn invalidate(&self, key: &K) {
        self.inner.invalidate(key).await;
    }

    /// 获取或加载：缓存未命中时调用 `loader` 加载并写入。
    pub async fn get_or_load<F, Fut>(&self, key: K, loader: F) -> V
    where
        F: FnOnce() -> Fut + Send,
        Fut: std::future::Future<Output = V> + Send,
        K: Clone,
    {
        if let Some(v) = self.inner.get(&key).await {
            return v;
        }
        let v = loader().await;
        self.inner.insert(key, v.clone()).await;
        v
    }

    pub fn entry_count(&self) -> u64 {
        self.inner.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[tokio::test]
    async fn insert_and_get() {
        let cache: TtlCache<String, i32> = TtlCache::new(100, Duration::from_secs(60));
        cache.insert("a".into(), 1).await;
        assert_eq!(cache.get(&"a".to_owned()).await, Some(1));
        assert_eq!(cache.get(&"missing".to_owned()).await, None);
    }

    #[tokio::test]
    async fn invalidate_removes() {
        let cache: TtlCache<String, i32> = TtlCache::new(100, Duration::from_secs(60));
        cache.insert("a".into(), 1).await;
        cache.invalidate(&"a".to_owned()).await;
        assert_eq!(cache.get(&"a".to_owned()).await, None);
    }

    #[tokio::test]
    async fn get_or_load_calls_loader_once_per_key() {
        let cache: TtlCache<String, i32> = TtlCache::new(100, Duration::from_secs(60));
        let counter = Arc::new(AtomicUsize::new(0));

        let c1 = Arc::clone(&counter);
        let v1 = cache
            .get_or_load("k".into(), || async move {
                c1.fetch_add(1, Ordering::SeqCst);
                42
            })
            .await;
        assert_eq!(v1, 42);

        let c2 = Arc::clone(&counter);
        let v2 = cache
            .get_or_load("k".into(), || async move {
                c2.fetch_add(1, Ordering::SeqCst);
                999 // 不应被调用
            })
            .await;
        assert_eq!(v2, 42, "second call should hit cache");
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
