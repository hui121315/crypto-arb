fn host_gate_fixture(
    rate_limit_retry_after_ms: Option<u64>,
    circuit_retry_after_ms: Option<u64>,
) -> HostGateSnapshot {
    HostGateSnapshot {
        exchange: "binance".to_owned(),
        host: "api.binance.com".to_owned(),
        consecutive_failures: 2,
        rate_limit_retry_after_ms,
        circuit_retry_after_ms,
        inflight_keys: 7,
        inflight_active_keys: 2,
        inflight_pruned_total: 5,
        inflight_oldest_idle_ms: Some(900),
        observed_at_ms: 10,
    }
}

#[test]
fn host_gate_rate_limit_retry_maps_to_typed_blocked_row() {
    let row = host_gate_row(host_gate_fixture(Some(2_500), None));

    assert_eq!(row.venue, "binance");
    assert_eq!(row.source, SOURCE_HOST_GATE);
    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.retry_after_ms, Some(2_500));
    assert_eq!(row.rows, Some(7));
    assert!(row.message.contains("上游限频退避 2500ms"));
    assert!(row.message.contains("active 2"));
    assert!(row.message.contains("pruned 5"));
    assert!(row.message.contains("oldest idle 900ms"));
    let problem = row.problem.as_ref().expect("host gate problem");
    assert_eq!(problem.code, HOST_GATE_RATE_LIMITED_CODE);
    assert_eq!(problem.retry_after_ms, Some(2_500));
    assert_eq!(problem.source.as_deref(), Some(SOURCE_HOST_GATE));
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("cause"))
            .and_then(|cause| cause.as_str()),
        Some("rate_limit_backoff")
    );
}

#[test]
fn host_gate_circuit_retry_is_distinct_from_rate_limit() {
    let row = host_gate_row(host_gate_fixture(None, Some(6_000)));

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert!(row.message.contains("熔断开启 6000ms"));
    let problem = row.problem.as_ref().expect("circuit problem");
    assert_eq!(problem.code, codes::CIRCUIT_BREAKER_OPEN);
    assert_eq!(problem.retry_after_ms, Some(6_000));
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("cause"))
            .and_then(|cause| cause.as_str()),
        Some("circuit_open")
    );
}

#[test]
fn host_gate_failure_streak_is_typed_before_backoff_opens() {
    let row = host_gate_row(host_gate_fixture(None, None));

    assert_eq!(row.status, VenueOperationStatus::Warn);
    let problem = row.problem.as_ref().expect("failure streak problem");
    assert_eq!(problem.code, HOST_GATE_FAILURE_STREAK_CODE);
    assert!(problem.retry_after_ms.is_none());
}

#[test]
fn rate_limiter_normal_throttle_wait_remains_ok() {
    let snapshot = RateLimiterSnapshot {
        name: "binance".to_owned(),
        qps: 20,
        parent: None,
        wait_total: 2,
        wait_ms_total: 120,
        try_acquire_total: 1,
        try_acquire_rejected_total: 0,
        last_wait_ms: Some(80),
        last_wait_observed_at_ms: Some(20_000),
        last_rejected_observed_at_ms: None,
        observed_at_ms: 21_000,
    };
    let row = rate_limiter_row(&snapshot, 21_000);

    assert_eq!(row.venue, "binance");
    assert_eq!(row.source, SOURCE_RATE_LIMITER);
    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.requested, Some(20));
    assert_eq!(row.rows, Some(2));
    assert_eq!(row.freshness_ms, Some(1_000));
    assert!(row.message.contains("正常节流"));
    assert!(row.problem.is_none());
}

#[test]
fn rate_limiter_long_recent_wait_maps_to_typed_warn_row() {
    let snapshot = RateLimiterSnapshot {
        name: "hyperliquid:xyz".to_owned(),
        qps: 20,
        parent: Some("hyperliquid".to_owned()),
        wait_total: 2,
        wait_ms_total: 10_500,
        try_acquire_total: 1,
        try_acquire_rejected_total: 0,
        last_wait_ms: Some(10_500),
        last_wait_observed_at_ms: Some(20_000),
        last_rejected_observed_at_ms: None,
        observed_at_ms: 21_000,
    };
    let row = rate_limiter_row(&snapshot, 21_000);

    assert_eq!(row.status, VenueOperationStatus::Warn);
    let problem = row.problem.as_ref().expect("rate limiter problem");
    assert_eq!(problem.code, RATE_LIMITER_PRESSURE_CODE);
    assert_eq!(problem.source.as_deref(), Some(SOURCE_RATE_LIMITER));
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("lastWaitMs"))
            .and_then(serde_json::Value::as_u64),
        Some(10_500)
    );
}
