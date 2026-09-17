use super::*;
use shared_types::{ApiProblem, HedgeConfirmStatus};

#[test]
fn resolved_context_uses_backend_run_and_preserves_preview_runtime_truth() {
    let fallback = shared_types::HedgeConfirmContext {
        opportunity_id: "opp-1".into(),
        idempotency_key: "idem-1".into(),
        ticket_id: Some("ticket-1".into()),
        environment: Some(shared_types::ExecutionEnvironment::Live),
        long_venue: Some("okx".into()),
        short_venue: Some("bybit".into()),
        ..shared_types::HedgeConfirmContext::default()
    };
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

    let context = resolved_confirm_context(&response, &fallback);

    assert_eq!(context.run_id.as_deref(), Some("run-1"));
    assert_eq!(context.ticket_id.as_deref(), Some("ticket-1"));
    assert_eq!(
        context.environment,
        Some(shared_types::ExecutionEnvironment::Live)
    );
    assert_eq!(context.long_venue.as_deref(), Some("mock"));
    assert_eq!(context.short_venue.as_deref(), Some("mock"));
}

#[test]
fn failed_problem_decodes_shared_confirm_context() {
    let expected = shared_types::HedgeConfirmContext {
        opportunity_id: "opp-1".into(),
        idempotency_key: "idem-1".into(),
        ticket_id: Some("ticket-1".into()),
        long_venue: Some("okx".into()),
        short_venue: Some("bybit".into()),
        ..shared_types::HedgeConfirmContext::default()
    };
    let mut problem = ApiProblem::new("DENIED", "blocked");
    problem.details = Some(serde_json::json!({ "confirmContext": expected }));

    let decoded = confirm_context_from_problem(&problem);
    assert!(
        matches!(decoded, Ok(Some(_))),
        "confirm context must decode: {decoded:?}"
    );
    if let Ok(Some(decoded)) = decoded {
        assert_eq!(decoded.ticket_id.as_deref(), Some("ticket-1"));
        assert_eq!(decoded.long_venue.as_deref(), Some("okx"));
        assert_eq!(decoded.short_venue.as_deref(), Some("bybit"));
    }
}

#[test]
fn malformed_confirm_context_adds_visible_decode_problem() {
    let mut problem = ApiProblem::new("DENIED", "blocked");
    problem.details = Some(serde_json::json!({ "confirmContext": "invalid" }));

    let decoded = confirm_context_from_problem(&problem);
    assert!(decoded.is_err(), "malformed context must fail");
    if let Err(error) = decoded {
        attach_confirm_context_decode_problem(&mut problem, &error);
    }

    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("confirmContextDecodeProblem"))
            .and_then(|decode| decode.get("code"))
            .and_then(|code| code.as_str()),
        Some(shared_types::problem::codes::HEDGE_CONFIRM_CONTEXT_DECODE_FAILED)
    );
}
