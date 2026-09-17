use dashmap::DashMap;
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, LazyLock, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::warn;

// TLS handshakes share the same proxy path as live market frames. Keep recovery
// bursts below the point where packet loss turns reconnects into more congestion.
const WS_CONNECT_GLOBAL_CONCURRENCY: usize = 4;
const WS_CONNECT_PER_HOST_CONCURRENCY: usize = 1;
const WS_CONNECT_RECOVERY_CONCURRENCY: usize = 1;
// Reserve one lane for the exact market currently visible to the operator.
// It bypasses a recovery backlog but remains bounded to one handshake globally.
const WS_CONNECT_PRIORITY_CONCURRENCY: usize = 1;
const RECOVERY_FAILURE_WINDOW: Duration = Duration::from_secs(20);
const RECOVERY_HOLD: Duration = Duration::from_secs(45);
const RECOVERY_DISTINCT_HOSTS: usize = 3;

static GLOBAL_BUDGET: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(WS_CONNECT_GLOBAL_CONCURRENCY)));
static RECOVERY_BUDGET: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(WS_CONNECT_RECOVERY_CONCURRENCY)));
static PRIORITY_BUDGET: LazyLock<Arc<Semaphore>> =
    LazyLock::new(|| Arc::new(Semaphore::new(WS_CONNECT_PRIORITY_CONCURRENCY)));
static HOST_BUDGETS: LazyLock<DashMap<String, Arc<Semaphore>>> = LazyLock::new(DashMap::new);
static RECOVERY_TRACKER: LazyLock<StdMutex<RecoveryTracker>> =
    LazyLock::new(|| StdMutex::new(RecoveryTracker::default()));

#[derive(Default)]
struct RecoveryTracker {
    failures: VecDeque<(Instant, String)>,
    recovery_until: Option<Instant>,
}

impl RecoveryTracker {
    fn recovery_active(&mut self, now: Instant) -> bool {
        self.prune(now);
        self.recovery_until.is_some_and(|until| until > now)
    }

    fn record_failure(&mut self, now: Instant, host: &str) -> bool {
        self.prune(now);
        let was_active = self.recovery_until.is_some_and(|until| until > now);
        self.failures.push_back((now, host.to_owned()));
        let distinct_hosts = self
            .failures
            .iter()
            .map(|(_, host)| host.as_str())
            .collect::<HashSet<_>>()
            .len();
        if was_active || distinct_hosts >= RECOVERY_DISTINCT_HOSTS {
            self.recovery_until = Some(now + RECOVERY_HOLD);
        }
        !was_active && self.recovery_until.is_some_and(|until| until > now)
    }

    fn prune(&mut self, now: Instant) {
        while self
            .failures
            .front()
            .is_some_and(|(at, _)| now.saturating_duration_since(*at) > RECOVERY_FAILURE_WINDOW)
        {
            self.failures.pop_front();
        }
        if self.recovery_until.is_some_and(|until| until <= now) {
            self.recovery_until = None;
        }
    }
}

pub(super) struct ConnectPermit {
    host_key: String,
    succeeded: bool,
    _recovery: Option<OwnedSemaphorePermit>,
    _priority: Option<OwnedSemaphorePermit>,
    _host: Option<OwnedSemaphorePermit>,
    _global: Option<OwnedSemaphorePermit>,
}

impl ConnectPermit {
    pub(super) fn mark_succeeded(mut self) {
        self.succeeded = true;
    }
}

impl Drop for ConnectPermit {
    fn drop(&mut self) {
        if self.succeeded {
            return;
        }
        let activated = RECOVERY_TRACKER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .record_failure(Instant::now(), &self.host_key);
        if activated {
            warn!(
                recovery_ms = RECOVERY_HOLD.as_millis() as u64,
                distinct_hosts = RECOVERY_DISTINCT_HOSTS,
                "websocket connect recovery serialized after cross-host failures"
            );
        }
    }
}

pub(super) async fn acquire(raw_url: &str) -> Result<ConnectPermit, String> {
    let host_key = host_key(raw_url);
    let host_budget = Arc::clone(
        HOST_BUDGETS
            .entry(host_key.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(WS_CONNECT_PER_HOST_CONCURRENCY)))
            .value(),
    );
    let host = host_budget
        .acquire_owned()
        .await
        .map_err(|_| "websocket host connect scheduler is unavailable".to_owned())?;
    let global = Arc::clone(&GLOBAL_BUDGET)
        .acquire_owned()
        .await
        .map_err(|_| "websocket global connect scheduler is unavailable".to_owned())?;
    // Re-check only after this attempt reaches the front of the normal queue.
    // A cold-start burst can enqueue many hosts before the first failures make
    // the weak shared path visible; checking earlier would let that whole queue
    // bypass recovery serialization.
    let recovery = RECOVERY_TRACKER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .recovery_active(Instant::now());
    let recovery = if recovery {
        Some(
            Arc::clone(&RECOVERY_BUDGET)
                .acquire_owned()
                .await
                .map_err(|_| "websocket recovery connect scheduler is unavailable".to_owned())?,
        )
    } else {
        None
    };
    Ok(ConnectPermit {
        host_key,
        succeeded: false,
        _recovery: recovery,
        _priority: None,
        _host: Some(host),
        _global: Some(global),
    })
}

pub(super) async fn acquire_priority(raw_url: &str) -> Result<ConnectPermit, String> {
    let priority = Arc::clone(&PRIORITY_BUDGET)
        .acquire_owned()
        .await
        .map_err(|_| "websocket priority connect scheduler is unavailable".to_owned())?;
    Ok(ConnectPermit {
        host_key: host_key(raw_url),
        succeeded: false,
        _recovery: None,
        _priority: Some(priority),
        _host: None,
        _global: None,
    })
}

fn host_key(raw_url: &str) -> String {
    url::Url::parse(raw_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .unwrap_or_else(|| "invalid-websocket-url".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_budget_is_isolated_by_normalized_host() {
        assert_eq!(
            host_key("wss://STREAM.BINANCE.COM:9443/stream"),
            "stream.binance.com"
        );
        assert_eq!(
            host_key("wss://fstream.binance.com/ws"),
            "fstream.binance.com"
        );
        assert_ne!(
            host_key("wss://stream.binance.com/stream"),
            host_key("wss://fstream.binance.com/ws")
        );
    }

    #[tokio::test]
    async fn priority_lane_bypasses_the_same_host_normal_queue() {
        let url = "wss://priority-connect.test/ws";
        let normal = acquire(url).await.expect("normal lane");

        let priority = tokio::time::timeout(Duration::from_millis(100), acquire_priority(url))
            .await
            .expect("priority lane must not wait for the normal host permit")
            .expect("priority lane");

        drop(priority);
        drop(normal);
    }

    #[test]
    fn recovery_requires_distinct_hosts_in_a_short_window() {
        let start = Instant::now();
        let mut tracker = RecoveryTracker::default();

        assert!(!tracker.record_failure(start, "stream.binance.com"));
        assert!(!tracker.record_failure(start + Duration::from_secs(1), "stream.binance.com"));
        assert!(!tracker.record_failure(start + Duration::from_secs(2), "ws.bitget.com"));
        assert!(tracker.record_failure(start + Duration::from_secs(3), "stream.bybit.com"));
        assert!(tracker.recovery_active(start + Duration::from_secs(4)));
    }

    #[test]
    fn old_failures_do_not_trigger_recovery() {
        let start = Instant::now();
        let mut tracker = RecoveryTracker::default();

        assert!(!tracker.record_failure(start, "stream.binance.com"));
        assert!(!tracker.record_failure(
            start + RECOVERY_FAILURE_WINDOW + Duration::from_secs(1),
            "ws.bitget.com"
        ));
        assert!(!tracker.record_failure(
            start + RECOVERY_FAILURE_WINDOW + Duration::from_secs(2),
            "stream.bybit.com"
        ));
        assert!(!tracker.recovery_active(start + RECOVERY_FAILURE_WINDOW + Duration::from_secs(3)));
    }
}
