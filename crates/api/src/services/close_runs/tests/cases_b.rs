#![allow(clippy::panic)]
use super::super::*;
use super::fixtures::*;

mod finality;
mod manual_terminal;
mod unwind;

#[tokio::test]
async fn failed_compensation_surfaces_manual_incident_review_action() {
    let state = test_state().await;
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    record(&state, run);

    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-1", 1.0));
    let _ = project_ledger_event_update(
        &state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );
    let compensation = compensation_order_record("comp-order-4", LiveOrderState::Submitted);
    record_compensation_order_for_candidate(&state, "close-1", 0, &compensation, None)
        .unwrap_or_else(|error| panic!("record compensation order failed: {error}"));

    let failed = project_ledger_event_update(
        &state,
        &ledger_state_event(&compensation.intent.id, LiveOrderState::Cancelled),
    );

    assert_eq!(failed[0].status, CloseRunStatus::CompensationFailed);
    assert_eq!(
        failed[0].unwind_plan.as_ref().map(|plan| plan.status),
        Some(CloseRunUnwindPlanStatus::CompensationFailed)
    );
    assert_eq!(
        failed[0]
            .unwind_plan
            .as_ref()
            .map(|plan| plan.remaining_positions.len()),
        Some(1)
    );
    assert_eq!(
        failed[0]
            .unwind_plan
            .as_ref()
            .and_then(|plan| plan.next_actions.first())
            .map(|action| action.kind),
        Some(CloseRunNextActionKind::ManualIncidentReview)
    );
    assert_eq!(
        failed[0]
            .problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("nextActions"))
            .and_then(|value| value.as_array())
            .and_then(|items| items.first())
            .and_then(|item| item.get("kind"))
            .and_then(|value| value.as_str()),
        Some("manual_incident_review")
    );
}
