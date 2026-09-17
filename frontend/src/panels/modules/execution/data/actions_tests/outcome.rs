use super::*;
use shared_types::{
    problem::codes, ActionEvidence, ApiProblem, ExecutionRunState, HedgeConfirmResponse,
    HedgeConfirmStatus,
};

#[test]
fn success_label_keeps_run_and_idempotency_context() {
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-1".into(),
        status: HedgeConfirmStatus::Submitted,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(run("run-1")),
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: None,
        partial_outcome: None,
        error: None,
    };

    let label = confirm_outcome_label(&response, "模拟");

    assert_eq!(
        label,
        "模拟已提交，等待成交确认 · Run run-1 · Idempotency idem-1"
    );
}

#[test]
fn confirm_response_merges_pending_run_and_order_identity_evidence() {
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-1".into(),
        status: HedgeConfirmStatus::Submitted,
        context: shared_types::HedgeConfirmContext {
            run_id: Some("run-1".into()),
            ticket_id: Some("ticket-1".into()),
            ..shared_types::HedgeConfirmContext::default()
        },
        execution_run: Some(run("run-1")),
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: None,
        partial_outcome: None,
        error: None,
    };
    let pending = ActionEvidence::client_request("req-confirm", Some("idem-1".into()))
        .with_client_order_ids(["client-long".into(), "client-short".into()]);

    let evidence = confirm_response_evidence(&response, pending);

    assert_eq!(evidence.request_id.as_deref(), Some("req-confirm"));
    assert_eq!(evidence.run_id.as_deref(), Some("run-1"));
    assert_eq!(evidence.ticket_id.as_deref(), Some("ticket-1"));
    assert_eq!(evidence.client_order_ids, ["client-long", "client-short"]);
}

#[test]
fn partial_failure_label_keeps_status_run_and_idempotency_context() {
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-1".into(),
        status: HedgeConfirmStatus::FirstLegPartialUnwindAttempted,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(run_with_state("run-1", ExecutionRunState::Unwinding)),
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: None,
        partial_outcome: None,
        error: None,
    };

    let label = confirm_outcome_label(&response, "实盘");
    let problem = confirm_response_problem(&response);

    assert_eq!(
        label,
        "部分成交，已尝试反向平衡 · Run run-1 · Idempotency idem-1"
    );
    assert_eq!(
        problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_CONFIRM_NOT_HEDGED)
    );
}

#[test]
fn confirm_response_problem_marks_http_200_unwind_error_failed() {
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-1".into(),
        status: HedgeConfirmStatus::HedgeBrokenUnwindFailed,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(run("run-1")),
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: Some(ApiProblem::new(
            codes::HEDGE_UNWIND_SUBMIT_FAILED,
            "unwind route down",
        )),
        partial_outcome: None,
        error: Some("fallback".into()),
    };

    let problem = confirm_response_problem(&response);

    assert_eq!(
        problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_UNWIND_SUBMIT_FAILED)
    );
}

#[test]
fn confirm_response_problem_preserves_typed_problem_transport_context() {
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-1".into(),
        status: HedgeConfirmStatus::LongLegFailed,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: None,
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: Some(
            ApiProblem::new("HEDGE_CONFIRM_RATE_LIMITED", "hedge confirm rate limited")
                .with_status(429)
                .with_request_id(Some("req-confirm-429".into()))
                .with_retry_after_ms(Some(7_000))
                .with_source("e2e-fixture"),
        ),
        partial_outcome: None,
        error: Some("fallback".into()),
    };

    let problem = confirm_response_problem(&response);

    assert_eq!(
        problem.as_ref().map(|problem| problem.code.as_str()),
        Some("HEDGE_CONFIRM_RATE_LIMITED")
    );
    assert_eq!(
        problem.as_ref().map(|problem| problem.message.as_str()),
        Some("hedge confirm rate limited")
    );
    assert_eq!(
        problem.as_ref().and_then(|problem| problem.status),
        Some(429)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.request_id.as_deref()),
        Some("req-confirm-429")
    );
    assert_eq!(
        problem.as_ref().and_then(|problem| problem.retry_after_ms),
        Some(7_000)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.source.as_deref()),
        Some("e2e-fixture")
    );
}

#[test]
fn confirm_response_problem_falls_back_to_partial_outcome_typed_problem(
) -> Result<(), serde_json::Error> {
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-1".into(),
        status: HedgeConfirmStatus::HedgeBrokenUnwindFailed,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(run_with_state("run-1", ExecutionRunState::UnwindRequired)),
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: None,
        partial_outcome: Some(serde_json::from_value(serde_json::json!({
            "cause": "hedge_broken",
            "originalStatus": "hedge_broken_unwind_failed",
            "runId": "run-1",
            "runState": "unwind_required",
            "netExposureUsd": 10.0,
            "recoveryAction": "manual_review",
            "primaryMessage": "second leg failed",
            "primaryProblem": {
                "code": "RATE_LIMITED",
                "message": "second leg rate limited"
            },
            "unwindStatus": "submit_failed",
            "unwindTargetLeg": "long",
            "unwindQuantity": 1.0,
            "unwindProblem": {
                "code": codes::HEDGE_UNWIND_SUBMIT_FAILED,
                "message": "unwind rate limited",
                "status": 429,
                "requestId": "req-unwind-429",
                "retryAfterMs": 4000
            },
            "manualReviewRequired": true
        }))?),
        error: Some("fallback".into()),
    };

    let problem = confirm_response_problem(&response);

    assert_eq!(
        problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_UNWIND_SUBMIT_FAILED)
    );
    assert_eq!(
        problem.as_ref().and_then(|problem| problem.status),
        Some(429)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.request_id.as_deref()),
        Some("req-unwind-429")
    );
    assert_eq!(
        problem.as_ref().and_then(|problem| problem.retry_after_ms),
        Some(4_000)
    );
    Ok(())
}
