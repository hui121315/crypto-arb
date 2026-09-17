use super::super::super::*;
use super::super::fixtures::*;

#[tokio::test]
async fn submitted_close_run_keeps_action_run_accepted_until_finality() {
    let state = test_state().await;
    let action_run = action_runs::begin(
        &state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::PortfolioClosePosition,
            actor: "test".to_owned(),
            target: Some("binance:MUUSDT".to_owned()),
            idempotency_key: None,
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.action_run_id = Some(action_run.id.clone());
    record(&state, run);

    let updated = project_ledger_event_update(&state, &ledger_fill_event("order-1", 0.4));

    assert_eq!(updated[0].status, CloseRunStatus::Submitted);
    let stored = action_runs::get(&state, &action_run.id)
        .unwrap_or_else(|| panic!("close action run missing"));
    assert_eq!(stored.status, ActionRunStatus::Accepted);
    let replayed = action_runs::replay_payload::<CloseRun>(&stored)
        .unwrap_or_else(|error| panic!("close action run payload missing: {error}"));
    assert_eq!(replayed.status, CloseRunStatus::Submitted);
}

#[tokio::test]
async fn filled_leg_plus_cancelled_leg_requires_close_run_unwind() {
    let state = test_state().await;
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    record(&state, run);

    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-1", 1.0));
    let updated = project_ledger_event_update(
        &state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );

    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].status, CloseRunStatus::UnwindRequired);
    assert_eq!(updated[0].failed_leg_count, 1);
    assert_eq!(updated[0].naked_exposure_usd, 100.0);
    assert_eq!(
        updated[0]
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::CLOSE_RUN_UNWIND_REQUIRED)
    );
    let details = updated[0]
        .problem
        .as_ref()
        .and_then(|problem| problem.details.as_ref())
        .unwrap_or_else(|| panic!("unwind problem missing details"));
    assert_unwind_problem_details(details);
    let plan = updated[0]
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("unwind plan missing"));
    assert_unwind_plan_actions(plan);
}

fn assert_unwind_problem_details(details: &serde_json::Value) {
    assert_eq!(
        details
            .get("autoUnwindStatus")
            .and_then(|value| value.as_str()),
        Some("blocked_pending_manual_recheck")
    );
    assert_eq!(
        details
            .get("compensationCandidates")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("orderId"))
            .and_then(|value| value.as_str()),
        Some("order-1")
    );
    assert_eq!(
        details
            .get("compensationCandidates")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("symbol"))
            .and_then(|value| value.as_str()),
        Some("MUUSDT")
    );
    assert_eq!(
        details
            .get("compensationCandidates")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("compensationOrderSide"))
            .and_then(|value| value.as_str()),
        Some("buy")
    );
    assert_eq!(
        details
            .get("filledLegs")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        details
            .get("remainingPositions")
            .and_then(|value| value.as_array())
            .map(Vec::len),
        Some(1)
    );
    assert_eq!(
        details
            .get("nextActions")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("kind"))
            .and_then(|value| value.as_str()),
        Some("submit_compensation_order")
    );
    assert_eq!(
        details
            .get("nextActions")
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("candidateIndex"))
            .and_then(|value| value.as_u64()),
        Some(0)
    );
}

fn assert_unwind_plan_actions(plan: &CloseRunUnwindPlan) {
    assert_eq!(plan.compensation_candidates.len(), 1);
    assert_eq!(
        plan.compensation_candidates[0].compensation_order_side,
        Some(OrderSide::Buy)
    );
    assert_eq!(
        plan.compensation_candidates[0].confirmed_quantity,
        Some(1.0)
    );
    assert_eq!(plan.remaining_positions.len(), 1);
    assert_eq!(
        plan.remaining_positions[0].status,
        CloseLegStatus::Cancelled
    );
    assert_eq!(plan.next_actions.len(), 1);
    assert_eq!(
        plan.next_actions[0].kind,
        CloseRunNextActionKind::SubmitCompensationOrder
    );
    assert_eq!(plan.next_actions[0].candidate_index, Some(0));
    assert!(plan.next_actions[0].requires_confirmation);
}
