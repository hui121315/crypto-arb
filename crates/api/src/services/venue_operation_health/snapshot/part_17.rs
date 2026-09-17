const HOST_GATE_RATE_LIMITED_CODE: &str = "HOST_GATE_RATE_LIMITED";
const HOST_GATE_FAILURE_STREAK_CODE: &str = "HOST_GATE_FAILURE_STREAK";
const RATE_LIMITER_PRESSURE_CODE: &str = "RATE_LIMITER_PRESSURE";

fn host_gate_status(
    snapshot: &HostGateSnapshot,
    retry_after_ms: Option<u64>,
) -> VenueOperationStatus {
    if retry_after_ms.is_some() {
        VenueOperationStatus::Blocked
    } else if snapshot.consecutive_failures > 0 {
        VenueOperationStatus::Warn
    } else {
        VenueOperationStatus::Ok
    }
}

fn host_gate_message(snapshot: &HostGateSnapshot, retry_after_ms: Option<u64>) -> String {
    let oldest_idle = snapshot
        .inflight_oldest_idle_ms
        .map(|ms| format!("，oldest idle {ms}ms"))
        .unwrap_or_default();
    match retry_after_ms {
        Some(wait_ms) => format!(
            "HostGate {} {} {}ms，连续失败 {}，inflight keys {}，active {}，pruned {}{}",
            snapshot.host,
            host_gate_backoff_label(snapshot),
            wait_ms,
            snapshot.consecutive_failures,
            snapshot.inflight_keys,
            snapshot.inflight_active_keys,
            snapshot.inflight_pruned_total,
            oldest_idle
        ),
        None => format!(
            "HostGate {} 可用，连续失败 {}，inflight keys {}，active {}，pruned {}{}",
            snapshot.host,
            snapshot.consecutive_failures,
            snapshot.inflight_keys,
            snapshot.inflight_active_keys,
            snapshot.inflight_pruned_total,
            oldest_idle
        ),
    }
}

fn host_gate_backoff_label(snapshot: &HostGateSnapshot) -> &'static str {
    match (
        snapshot.rate_limit_retry_after_ms.is_some(),
        snapshot.circuit_retry_after_ms.is_some(),
    ) {
        (true, true) => "限频退避且熔断开启",
        (false, true) => "熔断开启",
        (true, false) => "上游限频退避",
        (false, false) => "退避中",
    }
}

fn host_gate_problem(
    snapshot: &HostGateSnapshot,
    status: VenueOperationStatus,
    message: &str,
    retry_after_ms: Option<u64>,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok {
        return None;
    }
    let (code, cause) = if snapshot.circuit_retry_after_ms.is_some() {
        (codes::CIRCUIT_BREAKER_OPEN, "circuit_open")
    } else if snapshot.rate_limit_retry_after_ms.is_some() {
        (HOST_GATE_RATE_LIMITED_CODE, "rate_limit_backoff")
    } else {
        (HOST_GATE_FAILURE_STREAK_CODE, "failure_streak")
    };
    let mut problem = ApiProblem::new(code, message)
        .with_source(SOURCE_HOST_GATE)
        .with_retry_after_ms(retry_after_ms);
    problem.details = Some(serde_json::json!({
        "venue": snapshot.exchange.as_str(),
        "host": snapshot.host.as_str(),
        "cause": cause,
        "consecutiveFailures": snapshot.consecutive_failures,
        "rateLimitRetryAfterMs": snapshot.rate_limit_retry_after_ms,
        "circuitRetryAfterMs": snapshot.circuit_retry_after_ms,
        "inflightKeys": snapshot.inflight_keys,
        "inflightActiveKeys": snapshot.inflight_active_keys,
        "inflightPrunedTotal": snapshot.inflight_pruned_total,
        "inflightOldestIdleMs": snapshot.inflight_oldest_idle_ms,
    }));
    Some(problem)
}

fn latest_rate_limiter_observed_at(snapshot: &RateLimiterSnapshot) -> Option<i64> {
    match (
        snapshot.last_wait_observed_at_ms,
        snapshot.last_rejected_observed_at_ms,
    ) {
        (Some(wait_at), Some(rejected_at)) => Some(wait_at.max(rejected_at)),
        (Some(wait_at), None) => Some(wait_at),
        (None, Some(rejected_at)) => Some(rejected_at),
        (None, None) => None,
    }
}

fn rate_limiter_status(snapshot: &RateLimiterSnapshot, now_ms: i64) -> VenueOperationStatus {
    let recent_rejection = recent_at(snapshot.last_rejected_observed_at_ms, now_ms);
    let recent_wait = recent_at(snapshot.last_wait_observed_at_ms, now_ms)
        && snapshot.last_wait_ms.unwrap_or(0) >= RATE_LIMITER_PRESSURE_WAIT_MS;
    if recent_rejection || recent_wait {
        VenueOperationStatus::Warn
    } else if latest_rate_limiter_observed_at(snapshot).is_some() {
        VenueOperationStatus::Ok
    } else {
        VenueOperationStatus::Unknown
    }
}

fn recent_at(observed_at_ms: Option<i64>, now_ms: i64) -> bool {
    observed_at_ms
        .map(|observed_at_ms| freshness_since(observed_at_ms, now_ms) <= RATE_LIMITER_RECENT_MS)
        .unwrap_or(false)
}

fn rate_limiter_message(snapshot: &RateLimiterSnapshot, now_ms: i64) -> String {
    let parent = snapshot.parent.as_deref().unwrap_or("none");
    match rate_limiter_status(snapshot, now_ms) {
        VenueOperationStatus::Warn => format!(
            "RateLimiter {} 最近等待 {}ms，累计等待 {} 次 / {}ms，try_acquire 拒绝 {} 次，parent={}",
            snapshot.name,
            snapshot.last_wait_ms.unwrap_or(0),
            snapshot.wait_total,
            snapshot.wait_ms_total,
            snapshot.try_acquire_rejected_total,
            parent
        ),
        VenueOperationStatus::Ok => format!(
            "RateLimiter {} 正常节流，最近等待 {}ms，累计等待 {} 次 / {}ms，parent={}",
            snapshot.name,
            snapshot.last_wait_ms.unwrap_or(0),
            snapshot.wait_total,
            snapshot.wait_ms_total,
            parent
        ),
        VenueOperationStatus::Unknown => format!(
            "RateLimiter {} 已注册，尚无等待或拒绝样本，qps={}，parent={}",
            snapshot.name, snapshot.qps, parent
        ),
        VenueOperationStatus::Blocked | VenueOperationStatus::Unsupported => {
            format!("RateLimiter {} 状态异常，parent={}", snapshot.name, parent)
        }
    }
}

fn rate_limiter_problem(
    snapshot: &RateLimiterSnapshot,
    status: VenueOperationStatus,
    message: &str,
) -> Option<ApiProblem> {
    if status != VenueOperationStatus::Warn {
        return None;
    }
    let mut problem = ApiProblem::new(RATE_LIMITER_PRESSURE_CODE, message)
        .with_source(SOURCE_RATE_LIMITER);
    problem.details = Some(serde_json::json!({
        "name": snapshot.name.as_str(),
        "qps": snapshot.qps,
        "parent": snapshot.parent.as_deref(),
        "waitTotal": snapshot.wait_total,
        "waitMsTotal": snapshot.wait_ms_total,
        "lastWaitMs": snapshot.last_wait_ms,
        "tryAcquireTotal": snapshot.try_acquire_total,
        "tryAcquireRejectedTotal": snapshot.try_acquire_rejected_total,
        "lastWaitObservedAtMs": snapshot.last_wait_observed_at_ms,
        "lastRejectedObservedAtMs": snapshot.last_rejected_observed_at_ms,
    }));
    Some(problem)
}

fn attention_error(status: VenueOperationStatus, message: &str) -> Option<String> {
    matches!(
        status,
        VenueOperationStatus::Warn
            | VenueOperationStatus::Blocked
            | VenueOperationStatus::Unsupported
    )
    .then(|| message.to_owned())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
