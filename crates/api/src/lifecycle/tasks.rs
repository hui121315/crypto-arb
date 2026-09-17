mod panic_detail;
mod shutdown;

use crate::task_registry::TaskRegistry;
use futures::FutureExt;
use panic_detail::panic_message;
use std::collections::VecDeque;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::time::{Duration, Instant};
use tokio::sync::watch;
use tokio::task::JoinSet;
use tokio::time::Interval;
use tracing::{error, warn};

const DEFAULT_RESTART_MAX_ATTEMPTS: usize = 3;
const DEFAULT_RESTART_WINDOW: Duration = Duration::from_secs(60);
const DEFAULT_RESTART_BASE_BACKOFF: Duration = Duration::from_millis(250);
const DEFAULT_RESTART_MAX_BACKOFF: Duration = Duration::from_secs(5);

/// 协作式关停信号：控制端 `BackgroundTasks::shutdown` 触发，任务端用
/// `tick_or_shutdown` 在迭代边界观察后退出，让 ledger/history/audit 等写任务在被
/// abort 兜底前完成当前批次。
#[derive(Clone)]
pub(crate) struct ShutdownToken {
    rx: watch::Receiver<bool>,
}

impl ShutdownToken {
    /// 关停被触发（或控制端被丢弃）后返回；已触发则立即返回。
    pub(crate) async fn cancelled(&self) {
        let mut rx = self.rx.clone();
        let _ = rx.wait_for(|stop| *stop).await;
    }

    /// 是否已请求关停。
    pub(crate) fn is_shutdown(&self) -> bool {
        *self.rx.borrow()
    }
}

/// 等待下一次 tick；若关停已触发则立即返回 `false`，调用方据此退出循环。
/// 关停优先于 tick，避免在请求关停后再执行一次完整迭代。
pub(super) async fn tick_or_shutdown(tick: &mut Interval, shutdown: &ShutdownToken) -> bool {
    tokio::select! {
        biased;
        () = shutdown.cancelled() => false,
        _ = tick.tick() => true,
    }
}

/// Stagger cold-start work without delaying cooperative shutdown.
pub(super) async fn delay_or_shutdown(delay: Duration, shutdown: &ShutdownToken) -> bool {
    tokio::select! {
        biased;
        () = shutdown.cancelled() => false,
        () = tokio::time::sleep(delay) => true,
    }
}

/// 后台任务集合：登记存活状态、监视退出、优雅关闭时先协作 drain 再 abort 兜底。
pub(crate) struct BackgroundTasks {
    registry: TaskRegistry,
    watchers: JoinSet<()>,
    shutdown_tx: watch::Sender<bool>,
}

#[derive(Clone, Copy)]
struct RestartPolicy {
    max_attempts: usize,
    window: Duration,
    base_backoff: Duration,
    max_backoff: Duration,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_RESTART_MAX_ATTEMPTS,
            window: DEFAULT_RESTART_WINDOW,
            base_backoff: DEFAULT_RESTART_BASE_BACKOFF,
            max_backoff: DEFAULT_RESTART_MAX_BACKOFF,
        }
    }
}

struct RestartBudget {
    policy: RestartPolicy,
    attempts: VecDeque<Instant>,
}

impl RestartBudget {
    fn new(policy: RestartPolicy) -> Self {
        Self {
            policy,
            attempts: VecDeque::with_capacity(policy.max_attempts),
        }
    }

    fn reserve(&mut self, now: Instant) -> Option<Duration> {
        while self
            .attempts
            .front()
            .is_some_and(|at| now.saturating_duration_since(*at) >= self.policy.window)
        {
            self.attempts.pop_front();
        }
        if self.attempts.len() >= self.policy.max_attempts {
            return None;
        }
        let shift = self.attempts.len().min(31) as u32;
        let delay = self
            .policy
            .base_backoff
            .saturating_mul(1_u32 << shift)
            .min(self.policy.max_backoff);
        self.attempts.push_back(now);
        Some(delay)
    }

    fn attempt_count(&self) -> usize {
        self.attempts.len()
    }
}

impl BackgroundTasks {
    pub(crate) fn new(registry: TaskRegistry) -> Self {
        let (shutdown_tx, _rx) = watch::channel(false);
        Self {
            registry,
            watchers: JoinSet::new(),
            shutdown_tx,
        }
    }

    /// 派发给后台任务的协作式关停令牌。
    pub(crate) fn shutdown_token(&self) -> ShutdownToken {
        ShutdownToken {
            rx: self.shutdown_tx.subscribe(),
        }
    }

    /// 把未启用的可选任务显式投影到 runtime health。
    pub(crate) fn register_disabled(&self, name: &'static str, interval_ms: i64) {
        self.registry.register_disabled(name, interval_ms);
    }

    /// 监视可重建的后台任务。意外返回或 unwind panic 会在有界预算内指数退避重启；
    /// 预算耗尽后任务保持 `TASK_DOWN`，使 readiness fail-closed。
    pub(crate) fn supervise<F, Fut>(&mut self, name: &'static str, interval_ms: i64, factory: F)
    where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.supervise_with_policy(name, interval_ms, RestartPolicy::default(), factory);
    }

    fn supervise_with_policy<F, Fut>(
        &mut self,
        name: &'static str,
        interval_ms: i64,
        policy: RestartPolicy,
        factory: F,
    ) where
        F: FnMut() -> Fut + Send + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        self.registry.register(name, interval_ms);
        let registry = self.registry.clone();
        let shutdown = self.shutdown_token();
        self.watchers.spawn(run_supervised_task(
            name, registry, shutdown, policy, factory,
        ));
    }
}

async fn run_supervised_task<F, Fut>(
    name: &'static str,
    registry: TaskRegistry,
    shutdown: ShutdownToken,
    policy: RestartPolicy,
    mut factory: F,
) where
    F: FnMut() -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    let mut budget = RestartBudget::new(policy);
    loop {
        let outcome = shutdown::run_task_until_shutdown(name, &shutdown, &mut factory).await;
        let Some(outcome) = outcome else {
            registry.mark_exited(name, "stopped for shutdown");
            return;
        };
        if shutdown.is_shutdown() {
            registry.mark_exited(name, "stopped for shutdown");
            return;
        }
        let reason = outcome
            .err()
            .unwrap_or_else(|| "task returned unexpectedly".to_owned());
        if !restart_after_exit(name, &registry, &shutdown, &mut budget, reason).await {
            return;
        }
    }
}

async fn restart_after_exit(
    name: &'static str,
    registry: &TaskRegistry,
    shutdown: &ShutdownToken,
    budget: &mut RestartBudget,
    reason: String,
) -> bool {
    let Some(delay) = reserve_restart(name, registry, budget, &reason) else {
        return false;
    };
    let retry_at_ms = common::time::now_ms().saturating_add(delay.as_millis() as i64);
    registry.mark_retry_scheduled(name, reason.clone(), retry_at_ms);
    warn!(
        task = name,
        attempt = budget.attempt_count(),
        backoff_ms = delay.as_millis() as u64,
        %reason,
        "background task will restart"
    );
    complete_restart(name, registry, shutdown, delay).await
}

fn reserve_restart(
    name: &'static str,
    registry: &TaskRegistry,
    budget: &mut RestartBudget,
    reason: &str,
) -> Option<Duration> {
    let Some(delay) = budget.reserve(Instant::now()) else {
        let attempts = budget.attempt_count();
        registry.mark_exited(
            name,
            format!("{reason}; restart budget exhausted after {attempts} attempt(s)"),
        );
        error!(task = name, attempts, %reason, "background task restart budget exhausted");
        return None;
    };
    Some(delay)
}

async fn complete_restart(
    name: &'static str,
    registry: &TaskRegistry,
    shutdown: &ShutdownToken,
    delay: Duration,
) -> bool {
    if !delay_or_shutdown(delay, shutdown).await {
        return false;
    }
    registry.record_restart(name);
    true
}

async fn run_task_once<F, Fut>(factory: &mut F) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = ()>,
{
    let future = std::panic::catch_unwind(AssertUnwindSafe(&mut *factory))
        .map_err(|payload| panic_message(payload.as_ref()))?;
    AssertUnwindSafe(future)
        .catch_unwind()
        .await
        .map_err(|payload| panic_message(payload.as_ref()))
}

#[cfg(test)]
mod tests;
