use super::*;

#[test]
fn runtime_projection_preserves_structured_failure_evidence() {
    let mut legacy = operation(VenueOperationStatus::Blocked, Some(true), Some(true));
    legacy.error = Some("transport failed".to_owned());
    legacy.retry_after_ms = Some(500);
    legacy.problem = Some(
        ApiProblem::new("RATE_LIMITED", "venue rate limited")
            .with_request_id(Some("problem-request".to_owned()))
            .with_retry_after_ms(Some(2_000)),
    );
    legacy.evidence = Some(evidence(Some("evidence-request")));

    let runtime = VenueRuntimeOperationHealth::from_operation_health(
        VenueRuntimeOperation::PrivateRest,
        &legacy,
    );

    assert_eq!(runtime.last_success_ms, None);
    assert_eq!(runtime.last_error.as_deref(), Some("transport failed"));
    assert_eq!(runtime.retry_after_ms, Some(2_000));
    assert_eq!(runtime.request_id.as_deref(), Some("evidence-request"));
    assert_eq!(
        runtime
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("RATE_LIMITED")
    );
    assert!(!runtime.currently_usable);
}

#[test]
fn runtime_projection_preserves_latency_and_sample_counters() {
    let mut legacy = operation(VenueOperationStatus::Warn, Some(true), Some(true));
    legacy.requested = Some(4);
    legacy.rows = Some(3);
    legacy.latency_ms = Some(12);
    legacy.latency_p95_ms = Some(20);

    let runtime = VenueRuntimeOperationHealth::from_operation_health(
        VenueRuntimeOperation::OrderStream,
        &legacy,
    );

    assert_eq!(runtime.requested, Some(4));
    assert_eq!(runtime.rows, Some(3));
    assert_eq!(runtime.latency_ms, Some(12));
    assert_eq!(runtime.latency_p95_ms, Some(20));
}
