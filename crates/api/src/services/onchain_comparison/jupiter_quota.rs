use super::provider_runtime::jupiter_rate_profile;
use reqwest::header::HeaderMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::OnceLock;
use tokio::sync::Mutex;
use tokio::time::{sleep_until, Instant};

struct RequestGate {
    next_request_at: Mutex<Instant>,
    rate_limit_retry_at_ms: AtomicI64,
}

impl RequestGate {
    fn new() -> Self {
        Self {
            next_request_at: Mutex::new(Instant::now()),
            rate_limit_retry_at_ms: AtomicI64::new(0),
        }
    }

    async fn wait(&self, gap_ms: u64, max_wait_ms: Option<i64>) -> Result<(), i64> {
        let mut next_request_at = self.next_request_at.lock().await;
        let now = Instant::now();
        if *next_request_at > now {
            let wait_ms = i64::try_from(next_request_at.duration_since(now).as_millis())
                .unwrap_or(i64::MAX)
                .max(1);
            if max_wait_ms.is_some_and(|budget_ms| wait_ms > budget_ms.max(0)) {
                return Err(wait_ms);
            }
            sleep_until(*next_request_at).await;
        }
        *next_request_at = Instant::now() + std::time::Duration::from_millis(gap_ms);
        Ok(())
    }

    async fn defer_for(&self, delay_ms: i64) {
        let delay_ms = u64::try_from(delay_ms.max(1)).unwrap_or(1);
        let requested_at = Instant::now() + std::time::Duration::from_millis(delay_ms);
        let mut next_request_at = self.next_request_at.lock().await;
        if requested_at > *next_request_at {
            *next_request_at = requested_at;
        }
    }

    fn record_rate_limit_retry(&self, now_ms: i64, delay_ms: i64) {
        self.rate_limit_retry_at_ms
            .store(now_ms.saturating_add(delay_ms.max(1)), Ordering::Release);
    }

    fn clear_rate_limit_retry(&self) {
        self.rate_limit_retry_at_ms.store(0, Ordering::Release);
    }

    fn retry_after_ms(&self, now_ms: i64) -> Option<i64> {
        let delay_ms = self
            .rate_limit_retry_at_ms
            .load(Ordering::Acquire)
            .saturating_sub(now_ms);
        (delay_ms > 0).then_some(delay_ms)
    }
}

pub(super) async fn wait_for_general_request(keyed: bool) {
    // Jupiter applies 1 RPS per free key and 0.5 RPS per IP for keyless traffic.
    // The rate profile adds a 5% scheduling margin around that official boundary.
    // https://developers.jup.ag/docs/portal/plans
    let _ = request_gate(keyed)
        .wait(jupiter_rate_profile(keyed).request_gap_ms, None)
        .await;
}

pub(super) async fn wait_for_quote_request(keyed: bool) -> Result<(), i64> {
    let rate = jupiter_rate_profile(keyed);
    let max_wait_ms = i64::try_from(rate.request_gap_ms)
        .unwrap_or(i64::MAX)
        .saturating_add(250);
    request_gate(keyed)
        .wait(rate.request_gap_ms, Some(max_wait_ms))
        .await
}

pub(super) async fn observe_general_response(
    keyed: bool,
    status: reqwest::StatusCode,
    headers: &HeaderMap,
    now_ms: i64,
) -> Option<i64> {
    let gate = request_gate(keyed);
    let Some(retry_after_ms) = rate_limit_delay_ms(status, headers, now_ms) else {
        if status.is_success() {
            gate.clear_rate_limit_retry();
        }
        return None;
    };
    gate.defer_for(retry_after_ms).await;
    gate.record_rate_limit_retry(now_ms, retry_after_ms);
    Some(retry_after_ms)
}

pub(super) fn rate_limit_retry_after_ms(keyed: bool, now_ms: i64) -> Option<i64> {
    request_gate(keyed).retry_after_ms(now_ms)
}

fn request_gate(keyed: bool) -> &'static RequestGate {
    static KEYLESS_GATE: OnceLock<RequestGate> = OnceLock::new();
    static KEYED_GATE: OnceLock<RequestGate> = OnceLock::new();
    if keyed {
        KEYED_GATE.get_or_init(RequestGate::new)
    } else {
        KEYLESS_GATE.get_or_init(RequestGate::new)
    }
}

fn rate_limit_delay_ms(
    status: reqwest::StatusCode,
    headers: &HeaderMap,
    now_ms: i64,
) -> Option<i64> {
    if status != reqwest::StatusCode::TOO_MANY_REQUESTS {
        let remaining = headers
            .get("x-ratelimit-remaining")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok())?;
        if remaining > 0 {
            return None;
        }
    }
    let reset_delay_ms = headers
        .get("x-ratelimit-reset")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<i64>().ok())
        .map(|reset_seconds| {
            reset_seconds
                .saturating_mul(1_000)
                .saturating_sub(now_ms)
                .max(1)
        });
    Some(reset_delay_ms.unwrap_or(1_000))
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::header::HeaderValue;

    #[test]
    fn official_reset_header_becomes_an_absolute_retry_delay() {
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-reset", HeaderValue::from_static("1235"));
        assert_eq!(
            rate_limit_delay_ms(reqwest::StatusCode::TOO_MANY_REQUESTS, &headers, 1_234_250),
            Some(750)
        );
    }

    #[test]
    fn exhausted_success_response_defers_and_unlimited_response_does_not() {
        let mut exhausted = HeaderMap::new();
        exhausted.insert("x-ratelimit-remaining", HeaderValue::from_static("0"));
        exhausted.insert("x-ratelimit-reset", HeaderValue::from_static("1240"));
        assert_eq!(
            rate_limit_delay_ms(reqwest::StatusCode::OK, &exhausted, 1_234_000),
            Some(6_000)
        );

        let mut available = HeaderMap::new();
        available.insert("x-ratelimit-remaining", HeaderValue::from_static("2"));
        assert_eq!(
            rate_limit_delay_ms(reqwest::StatusCode::OK, &available, 1_234_000),
            None
        );
    }

    #[tokio::test]
    async fn quote_request_returns_the_long_wait_instead_of_blocking_refresh() {
        let gate = RequestGate::new();
        gate.defer_for(5_000).await;
        let retry_after_ms = gate
            .wait(1_050, Some(1_300))
            .await
            .expect_err("long quota wait should be returned to the scheduler");
        assert!((4_900..=5_000).contains(&retry_after_ms));
    }
}
