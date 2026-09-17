use super::super::*;

pub(super) fn arbitrage_snapshot_problem() -> RuntimeProblem {
    RuntimeProblem {
        scope: BACKGROUND_TASK_SCOPE.into(),
        operation: ARBITRAGE_SNAPSHOT_TASK.into(),
        code: "TASK_FAILING".into(),
        message: "background task 'arbitrage_snapshot': 连续失败".into(),
        venue: None,
        retry_after_ms: None,
        problem: None,
        observed_at_ms: 1,
    }
}

pub(super) fn api_problem() -> ApiProblem {
    ApiProblem::new("RATE_LIMITED", "slow down")
        .with_status(429)
        .with_request_id(Some("req-1".into()))
        .with_retry_after_ms(Some(2_000))
}

pub(super) fn operation_row(
    venue: &str,
    operation: &str,
    status: VenueOperationStatus,
) -> VenueOperationHealth {
    VenueOperationHealth {
        venue: venue.into(),
        operation: operation.into(),
        status,
        source: "market-data-cache".into(),
        message: "runtime health".into(),
        supported: Some(true),
        configured: Some(true),
        requested: Some(1),
        rows: Some(u64::from(status == VenueOperationStatus::Ok)),
        freshness_ms: Some(250),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: None,
        evidence: None,
        problem: None,
        observed_at_ms: 1_000,
    }
}
