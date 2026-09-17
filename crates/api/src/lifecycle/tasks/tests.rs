use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn test_policy(max_attempts: usize) -> RestartPolicy {
    RestartPolicy {
        max_attempts,
        window: Duration::from_secs(1),
        base_backoff: Duration::from_millis(1),
        max_backoff: Duration::from_millis(2),
    }
}

#[tokio::test]
async fn tick_or_shutdown_stops_when_cancelled() {
    let (tx, rx) = watch::channel(false);
    let token = ShutdownToken { rx };
    let mut tick = tokio::time::interval(Duration::from_millis(5));

    // 首个 tick 立即就绪：未关停时正常迭代。
    assert!(tick_or_shutdown(&mut tick, &token).await);

    assert!(tx.send(true).is_ok());
    // 关停后即使 tick 就绪也优先退出。
    assert!(!tick_or_shutdown(&mut tick, &token).await);
}

#[tokio::test]
async fn cold_start_delay_stops_when_cancelled() {
    let (tx, rx) = watch::channel(false);
    let token = ShutdownToken { rx };
    assert!(tx.send(true).is_ok());

    assert!(!delay_or_shutdown(Duration::from_secs(30), &token).await);
}

#[tokio::test]
async fn cooperative_task_drains_within_budget() {
    let mut tasks = BackgroundTasks::new(TaskRegistry::default());
    let token = tasks.shutdown_token();
    tasks.supervise("coop", 10, move || {
        let token = token.clone();
        async move {
            let mut tick = tokio::time::interval(Duration::from_millis(10));
            while tick_or_shutdown(&mut tick, &token).await {}
        }
    });

    assert!(tasks.shutdown(Duration::from_secs(2)).await);
}

#[tokio::test]
#[allow(clippy::panic)]
async fn panicked_task_restarts_within_budget() {
    let registry = TaskRegistry::default();
    let mut tasks = BackgroundTasks::new(registry.clone());
    let token = tasks.shutdown_token();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    tasks.supervise_with_policy("restartable", 10, test_policy(2), move || {
        let token = token.clone();
        let attempt = observed.fetch_add(1, Ordering::SeqCst);
        async move {
            if attempt == 0 {
                panic!("first attempt");
            }
            token.cancelled().await;
        }
    });

    assert!(wait_until(|| attempts.load(Ordering::SeqCst) >= 2).await);
    let snapshot = registry.task_snapshots(common::time::now_ms()).remove(0);
    assert!(snapshot.running);
    assert_eq!(snapshot.exit_count, 1);
    assert_eq!(snapshot.restart_count, 1);
    assert!(tasks.shutdown(Duration::from_secs(1)).await);
}

#[tokio::test]
#[allow(clippy::panic)]
async fn factory_panic_restarts_within_budget() {
    let registry = TaskRegistry::default();
    let mut tasks = BackgroundTasks::new(registry.clone());
    let token = tasks.shutdown_token();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    tasks.supervise_with_policy("factory_panic", 10, test_policy(2), move || {
        let attempt = observed.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            panic!("factory panic");
        }
        let token = token.clone();
        async move { token.cancelled().await }
    });

    assert!(wait_until(|| attempts.load(Ordering::SeqCst) >= 2).await);
    let snapshot = registry.task_snapshots(common::time::now_ms()).remove(0);
    assert!(snapshot.running);
    assert_eq!(snapshot.exit_count, 1);
    assert_eq!(snapshot.restart_count, 1);
    assert!(tasks.shutdown(Duration::from_secs(1)).await);
}

#[tokio::test]
async fn unexpected_returns_exhaust_restart_budget() {
    let registry = TaskRegistry::default();
    let mut tasks = BackgroundTasks::new(registry.clone());
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&attempts);
    tasks.supervise_with_policy("exhausted", 10, test_policy(2), move || {
        observed.fetch_add(1, Ordering::SeqCst);
        async {}
    });

    assert!(wait_until(|| attempts.load(Ordering::SeqCst) >= 3).await);
    assert!(
        wait_until(|| {
            registry
                .task_snapshots(common::time::now_ms())
                .first()
                .is_some_and(|snapshot| !snapshot.running)
        })
        .await
    );
    let snapshot = registry.task_snapshots(common::time::now_ms()).remove(0);
    assert_eq!(snapshot.exit_count, 3);
    assert_eq!(snapshot.restart_count, 2);
    assert!(snapshot
        .last_exit_reason
        .is_some_and(|reason| reason.contains("restart budget exhausted")));
    assert!(tasks.shutdown(Duration::from_secs(1)).await);
}

#[tokio::test]
async fn cooperative_shutdown_does_not_restart_task() {
    let registry = TaskRegistry::default();
    let mut tasks = BackgroundTasks::new(registry.clone());
    let token = tasks.shutdown_token();
    tasks.supervise("shutdown", 10, move || {
        let token = token.clone();
        async move { token.cancelled().await }
    });

    assert!(tasks.shutdown(Duration::from_secs(1)).await);
    let snapshot = registry.task_snapshots(common::time::now_ms()).remove(0);
    assert_eq!(snapshot.restart_count, 0);
    assert_eq!(snapshot.exit_count, 1);
}

#[tokio::test]
async fn stubborn_task_hits_abort_fallback() {
    let mut tasks = BackgroundTasks::new(TaskRegistry::default());
    tasks.supervise("stubborn", 10, || async {
        std::future::pending::<()>().await;
    });

    assert!(!tasks.shutdown(Duration::from_millis(50)).await);
}

#[tokio::test]
async fn pending_iteration_is_cancelled_before_global_drain_budget() {
    let mut tasks = BackgroundTasks::new(TaskRegistry::default());
    tasks.supervise("pending", 10, || async {
        std::future::pending::<()>().await;
    });

    assert!(tasks.shutdown(Duration::from_secs(2)).await);
}

#[tokio::test]
async fn uncooperative_watcher_is_aborted_before_global_drain_budget() {
    let mut tasks = BackgroundTasks::new(TaskRegistry::default());
    tasks.watchers.spawn(std::future::pending::<()>());

    let started_at = Instant::now();
    assert!(tasks.shutdown(Duration::from_secs(2)).await);
    assert!(started_at.elapsed() < Duration::from_secs(2));
}

async fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
    tokio::time::timeout(Duration::from_secs(1), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    })
    .await
    .is_ok()
}
