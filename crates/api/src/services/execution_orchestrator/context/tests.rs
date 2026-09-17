use super::*;

#[test]
fn attach_problem_preserves_non_object_details() {
    let context = shared_types::HedgeConfirmContext {
        opportunity_id: "opp-test".into(),
        ..shared_types::HedgeConfirmContext::default()
    };
    let mut problem = ApiProblem::new("UPSTREAM", "route unavailable");
    problem.details = Some(serde_json::json!(["venue", "offline"]));

    let attached = attach_problem(problem, &context);

    assert_eq!(
        attached
            .details
            .as_ref()
            .and_then(|details| details.get("originalDetails")),
        Some(&serde_json::json!(["venue", "offline"]))
    );
    assert_eq!(
        attached
            .details
            .as_ref()
            .and_then(|details| details.get("confirmContext"))
            .and_then(|details| details.get("opportunityId")),
        Some(&serde_json::json!("opp-test"))
    );
}

#[test]
fn second_leg_failure_keeps_identity_environment_and_both_venue_problems() {
    let context = shared_types::HedgeConfirmContext {
        opportunity_id: "opp-test".into(),
        idempotency_key: "idem-1".into(),
        ticket_id: Some("ticket-test".into()),
        environment: Some(shared_types::ExecutionEnvironment::Paper),
        long_venue: Some("okx".into()),
        short_venue: Some("bybit".into()),
        ..shared_types::HedgeConfirmContext::default()
    };
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-1".into(),
        status: HedgeConfirmStatus::HedgeBrokenUnwindFailed,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: Some(test_run()),
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: Some(ApiProblem::new(
            codes::HEDGE_UNWIND_SUBMIT_FAILED,
            "unwind failed",
        )),
        partial_outcome: Some(HedgeConfirmPartialOutcome {
            context: shared_types::HedgeConfirmContext::default(),
            cause: HedgeConfirmPartialCause::HedgeBroken,
            original_status: HedgeConfirmStatus::HedgeBrokenUnwindFailed,
            run_id: "run-idem-1".into(),
            run_state: ExecutionRunState::UnwindRequired,
            net_exposure_usd: 12.0,
            recovery_action: Some(RecoveryAction::ManualReview),
            primary_message: Some("short submit failed".into()),
            primary_problem: Some(ApiProblem::new("RATE_LIMITED", "short submit failed")),
            unwind_status: HedgeConfirmUnwindStatus::SubmitFailed,
            unwind_target_leg: Some(HedgeLegRole::Long),
            unwind_quantity: Some(1.0),
            unwind_problem: Some(ApiProblem::new(
                codes::HEDGE_UNWIND_SUBMIT_FAILED,
                "unwind failed",
            )),
            manual_review_required: true,
        }),
        error: Some("short submit failed; unwind failed".into()),
    };

    let response = attach_response(response, context, HedgeLegRole::Long);

    assert_eq!(response.context.run_id.as_deref(), Some("run-idem-1"));
    assert_eq!(response.context.ticket_id.as_deref(), Some("ticket-test"));
    assert_eq!(
        response.context.environment,
        Some(shared_types::ExecutionEnvironment::Paper)
    );
    assert_eq!(
        response
            .context
            .short_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("RATE_LIMITED")
    );
    assert_eq!(
        response
            .context
            .long_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_UNWIND_SUBMIT_FAILED)
    );
    assert!(response
        .problem
        .as_ref()
        .and_then(|problem| problem.details.as_ref())
        .and_then(|details| details.get("confirmContext"))
        .is_some());
}

#[test]
fn short_first_submission_failure_is_attached_to_short_venue() {
    let context = shared_types::HedgeConfirmContext {
        opportunity_id: "opp-test".into(),
        idempotency_key: "idem-short".into(),
        long_venue: Some("okx".into()),
        short_venue: Some("bybit".into()),
        ..shared_types::HedgeConfirmContext::default()
    };
    let response = HedgeConfirmResponse {
        idempotency_key: "idem-short".into(),
        status: HedgeConfirmStatus::LongLegFailed,
        context: shared_types::HedgeConfirmContext::default(),
        execution_run: None,
        long_record: None,
        short_record: None,
        unwind_record: None,
        problem: Some(ApiProblem::new("SUBMIT_FAILED", "short route failed")),
        partial_outcome: None,
        error: Some("short route failed".into()),
    };

    let response = attach_response(response, context, HedgeLegRole::Short);

    assert!(response.context.long_problem.is_none());
    assert_eq!(
        response
            .context
            .short_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some("SUBMIT_FAILED")
    );
}

fn test_run() -> ExecutionRun {
    ExecutionRun {
        run_id: "run-idem-1".into(),
        ticket_id: "ticket-test".into(),
        opportunity_id: "opp-test".into(),
        state: ExecutionRunState::UnwindRequired,
        long_leg: test_leg(HedgeLegRole::Long, "okx"),
        short_leg: test_leg(HedgeLegRole::Short, "bybit"),
        net_exposure_usd: 12.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: Some(RecoveryAction::ManualReview),
        status_reason: "manual review".into(),
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}

fn test_leg(role: HedgeLegRole, venue: &str) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: venue.into(),
        symbol: "BTCUSDT".into(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Failed,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
