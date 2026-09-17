use super::super::*;
use super::*;

#[test]
fn confirm_action_problem_attaches_partial_outcome_details() {
    let mut response = confirm_response_with_run(shared_types::ExecutionRunState::Unwinding);
    response.status = shared_types::HedgeConfirmStatus::FirstLegPartialWaitingFillQty;
    response.error = Some("waiting for fill quantity".into());
    response.partial_outcome = Some(partial_outcome(
        shared_types::HedgeConfirmStatus::FirstLegPartialWaitingFillQty,
        shared_types::HedgeConfirmUnwindStatus::AwaitingFillQuantity,
    ));

    let details = confirm_action_problem(&response).and_then(|problem| problem.details);

    assert_eq!(
        details
            .as_ref()
            .and_then(|details| details.get("partialOutcome"))
            .and_then(|value| value.get("cause"))
            .and_then(|value| value.as_str()),
        Some("first_leg_partial")
    );
    assert_eq!(
        details
            .as_ref()
            .and_then(|details| details.get("partialOutcome"))
            .and_then(|value| value.get("unwindStatus"))
            .and_then(|value| value.as_str()),
        Some("awaiting_fill_quantity")
    );
    assert_eq!(
        details
            .as_ref()
            .and_then(|details| details.get("confirmStatus"))
            .and_then(|value| value.as_str()),
        Some("first_leg_partial_waiting_fill_qty")
    );
}

#[test]
fn confirm_action_problem_uses_partial_primary_problem_when_top_level_missing() {
    let mut response = confirm_response_with_run(shared_types::ExecutionRunState::UnwindRequired);
    response.status = shared_types::HedgeConfirmStatus::HedgeBrokenUnwindAttempted;
    response.problem = None;
    response.error = Some("second leg rate limited".into());
    let mut outcome = partial_outcome(
        shared_types::HedgeConfirmStatus::HedgeBrokenUnwindAttempted,
        shared_types::HedgeConfirmUnwindStatus::Submitted,
    );
    outcome.primary_problem = Some(
        shared_types::ApiProblem::new("RATE_LIMITED", "second leg rate limited")
            .with_status(429)
            .with_request_id(Some("req-second-leg-429".into()))
            .with_retry_after_ms(Some(8_000)),
    );
    outcome.unwind_problem = None;
    response.partial_outcome = Some(outcome);

    let problem = confirm_action_problem(&response);

    assert_eq!(
        problem.as_ref().map(|problem| problem.code.as_str()),
        Some("RATE_LIMITED")
    );
    assert_eq!(
        problem.as_ref().and_then(|problem| problem.status),
        Some(429)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.request_id.as_deref()),
        Some("req-second-leg-429")
    );
    assert_eq!(
        problem.as_ref().and_then(|problem| problem.retry_after_ms),
        Some(8_000)
    );
    assert_eq!(
        problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("partialOutcome"))
            .and_then(|value| value.get("primaryProblem"))
            .and_then(|value| value.get("requestId"))
            .and_then(|value| value.as_str()),
        Some("req-second-leg-429")
    );
}

#[test]
fn confirm_action_problem_preserves_non_object_problem_details() {
    let mut response = confirm_response_with_run(shared_types::ExecutionRunState::UnwindRequired);
    response.status = shared_types::HedgeConfirmStatus::HedgeBrokenUnwindAttempted;
    let mut problem = shared_types::ApiProblem::new("RATE_LIMITED", "second leg rate limited");
    problem.details = Some(serde_json::json!(["retry", "later"]));
    response.problem = Some(problem);
    response.partial_outcome = Some(partial_outcome(
        shared_types::HedgeConfirmStatus::HedgeBrokenUnwindAttempted,
        shared_types::HedgeConfirmUnwindStatus::Submitted,
    ));

    let details = confirm_action_problem(&response).and_then(|problem| problem.details);

    assert_eq!(
        details
            .as_ref()
            .and_then(|value| value.get("originalDetails")),
        Some(&serde_json::json!(["retry", "later"]))
    );
    assert!(details
        .as_ref()
        .and_then(|value| value.get("partialOutcome"))
        .is_some());
}

#[tokio::test]
async fn confirm_replay_with_hot_run_preserves_stored_partial_outcome() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let action_run = action_runs::begin(
        &state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::HedgeConfirm,
            actor: "test".to_owned(),
            target: Some("opp-test".to_owned()),
            idempotency_key: Some("hedge-test".to_owned()),
            message: "accepted".to_owned(),
        },
    )?;
    let mut response = confirm_response_with_run(shared_types::ExecutionRunState::Unwinding);
    response.status = shared_types::HedgeConfirmStatus::FirstLegPartialUnwindFailed;
    response.error = Some("route down".into());
    response.partial_outcome = Some(partial_outcome(
        shared_types::HedgeConfirmStatus::FirstLegPartialUnwindFailed,
        shared_types::HedgeConfirmUnwindStatus::SubmitFailed,
    ));

    let returned = finish_confirm_action(&state, &action_run, response)?;
    let stored = action_runs::get(&state, &action_run.id)
        .ok_or_else(|| anyhow::anyhow!("hedge action run missing"))?;
    state.execution_runs().insert(
        "run-hedge-test".into(),
        execution_run_with_state(shared_types::ExecutionRunState::Unwinding),
    );

    let replayed = replay_confirm_response(&state, &stored, "hedge-test")?;

    assert_eq!(
        returned
            .partial_outcome
            .as_ref()
            .map(|outcome| outcome.original_status),
        Some(shared_types::HedgeConfirmStatus::FirstLegPartialUnwindFailed)
    );
    assert_eq!(replayed.status, shared_types::HedgeConfirmStatus::Replayed);
    assert_eq!(
        replayed
            .partial_outcome
            .as_ref()
            .map(|outcome| outcome.original_status),
        Some(shared_types::HedgeConfirmStatus::FirstLegPartialUnwindFailed)
    );
    Ok(())
}

fn partial_outcome(
    original_status: shared_types::HedgeConfirmStatus,
    unwind_status: shared_types::HedgeConfirmUnwindStatus,
) -> shared_types::HedgeConfirmPartialOutcome {
    shared_types::HedgeConfirmPartialOutcome {
        context: shared_types::HedgeConfirmContext::default(),
        cause: shared_types::HedgeConfirmPartialCause::FirstLegPartial,
        original_status,
        run_id: "run-hedge-test".into(),
        run_state: shared_types::ExecutionRunState::UnwindRequired,
        net_exposure_usd: 42.0,
        recovery_action: Some(shared_types::RecoveryAction::ManualReview),
        primary_message: Some("first leg partial".into()),
        primary_problem: None,
        unwind_status,
        unwind_target_leg: Some(shared_types::HedgeLegRole::Long),
        unwind_quantity: Some(0.4),
        unwind_problem: Some(shared_types::ApiProblem::new(
            codes::HEDGE_UNWIND_SUBMIT_FAILED,
            "route down",
        )),
        manual_review_required: true,
    }
}
