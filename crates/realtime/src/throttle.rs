use parking_lot::Mutex;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::task::JoinHandle;

struct ThrottleInner<T> {
    latest: Option<T>,
    scheduled: bool,
    last_flush_at: Option<Instant>,
}

struct ThrottleState<T> {
    interval: Duration,
    inner: Mutex<ThrottleInner<T>>,
    closed: AtomicBool,
    task: Mutex<Option<JoinHandle<()>>>,
    sink: Arc<dyn Fn(T) + Send + Sync>,
}

/// 前沿触发 + 尾沿合并的节流器。
///
/// 三个 throttled 频道的稳态生产节奏（portfolio 2s / system 5s / arbitrage 5s）
/// 都远大于 interval（100ms）：纯尾沿实现会给每条消息加上整整一个 interval 的
/// 固定延迟、且从不真正合并，还为每条消息 spawn/销毁一个 task。本实现在距上次
/// 发射 ≥ interval 时立即发射（前沿），仅 interval 窗口内的后续消息进入尾沿
/// 合并——两次发射之间的间隔仍保证 ≥ interval（满足 AGENTS §4.3 的批量约束）。
#[derive(Clone)]
pub struct Throttle<T> {
    state: Arc<ThrottleState<T>>,
}

impl<T> fmt::Debug for Throttle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Throttle")
            .field("interval", &self.state.interval)
            .field("scheduled", &self.state.inner.lock().scheduled)
            .finish_non_exhaustive()
    }
}

impl<T: Send + 'static> Throttle<T> {
    pub fn new(interval: Duration, sink: impl Fn(T) + Send + Sync + 'static) -> Self {
        Self {
            state: Arc::new(ThrottleState {
                interval,
                inner: Mutex::new(ThrottleInner {
                    latest: None,
                    scheduled: false,
                    last_flush_at: None,
                }),
                closed: AtomicBool::new(false),
                task: Mutex::new(None),
                sink: Arc::new(sink),
            }),
        }
    }

    pub fn push(&self, item: T) {
        if self.state.closed.load(Ordering::Acquire) {
            return;
        }
        let now = Instant::now();
        let mut inner = self.state.inner.lock();
        let interval_elapsed = inner
            .last_flush_at
            .is_none_or(|at| now.duration_since(at) >= self.state.interval);
        // 前沿：窗口外且无尾沿挂起 → 立即发射（sink 在锁外调用）。
        if interval_elapsed && !inner.scheduled {
            inner.last_flush_at = Some(now);
            drop(inner);
            (self.state.sink)(item);
            return;
        }
        // 窗口内：合并到 latest，必要时安排一次对齐到窗口边界的尾沿发射。
        inner.latest = Some(item);
        if !inner.scheduled {
            inner.scheduled = true;
            let delay = inner.last_flush_at.map_or(self.state.interval, |at| {
                self.state.interval.saturating_sub(now.duration_since(at))
            });
            drop(inner);
            let state = Arc::clone(&self.state);
            let handle = tokio::spawn(async move { flush_after(state, delay).await });
            *self.state.task.lock() = Some(handle);
        }
    }

    pub fn shutdown(&self) {
        self.state.shutdown();
    }
}

impl<T> ThrottleState<T> {
    fn shutdown(&self) {
        self.closed.store(true, Ordering::Release);
        {
            let mut inner = self.inner.lock();
            inner.latest = None;
            inner.scheduled = false;
        }
        if let Some(handle) = self.task.lock().take() {
            handle.abort();
        }
    }
}

async fn flush_after<T: Send + 'static>(state: Arc<ThrottleState<T>>, mut delay: Duration) {
    loop {
        tokio::time::sleep(delay).await;
        if state.closed.load(Ordering::Acquire) {
            state.inner.lock().scheduled = false;
            break;
        }
        let item = {
            let mut inner = state.inner.lock();
            let item = inner.latest.take();
            if item.is_some() {
                inner.last_flush_at = Some(Instant::now());
            }
            item
        };
        if let Some(item) = item {
            (state.sink)(item);
        }
        let mut inner = state.inner.lock();
        if inner.latest.is_some() {
            // 发射期间又有新消息：常驻本 task 继续下一个窗口，不重新 spawn。
            delay = state.interval;
            continue;
        }
        inner.scheduled = false;
        break;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pretty_assertions::assert_eq;

    #[tokio::test(start_paused = true)]
    async fn leading_edge_emits_immediately_then_coalesces_trailing() {
        let out = Arc::new(Mutex::new(Vec::new()));
        let sink_out = Arc::clone(&out);
        let throttle = Throttle::new(Duration::from_millis(100), move |item| {
            sink_out.lock().push(item);
        });

        // 稳态（每条消息间隔 ≥ interval）零延迟发射。
        throttle.push("first");
        assert_eq!(out.lock().as_slice(), ["first"]);

        // 窗口内的洪峰合并到窗口边界，只保留最新。
        throttle.push("old");
        throttle.push("new");
        tokio::task::yield_now().await;
        assert_eq!(out.lock().as_slice(), ["first"]);

        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
        assert_eq!(out.lock().as_slice(), ["first", "new"]);
    }

    #[tokio::test(start_paused = true)]
    async fn emissions_stay_at_least_one_interval_apart() {
        let out = Arc::new(Mutex::new(Vec::new()));
        let sink_out = Arc::clone(&out);
        let throttle = Throttle::new(Duration::from_millis(100), move |item| {
            sink_out.lock().push(item);
        });

        throttle.push(1);
        throttle.push(2);
        // 让尾沿 task 先注册好定时器，再推进虚拟时钟。
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
        assert_eq!(out.lock().as_slice(), [1, 2]);

        // 尾沿发射后仍在新窗口内的消息继续合并，不会提前发射。
        throttle.push(3);
        tokio::task::yield_now().await;
        assert_eq!(out.lock().as_slice(), [1, 2]);
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;
        assert_eq!(out.lock().as_slice(), [1, 2, 3]);
    }

    #[tokio::test(start_paused = true)]
    async fn shutdown_aborts_pending_trailing_flush() {
        let out = Arc::new(Mutex::new(Vec::new()));
        let sink_out = Arc::clone(&out);
        let throttle = Throttle::new(Duration::from_millis(100), move |item| {
            sink_out.lock().push(item);
        });

        throttle.push("leading");
        throttle.push("pending");
        throttle.shutdown();
        tokio::time::advance(Duration::from_millis(100)).await;
        tokio::task::yield_now().await;

        assert_eq!(out.lock().as_slice(), ["leading"]);
    }
}
