use super::super::super::*;
use super::super::fixtures::*;

#[test]
fn legacy_paper_adapter_fill_replays_as_compensated() {
    let mut run = legacy_compensation_run(ExecutionMode::DryRun);

    assert!(normalize_replayed_paper_finality(&mut run));
    assert_eq!(run.status, CloseRunStatus::Compensated);
    assert_eq!(run.naked_exposure_usd, 0.0);
    let plan = run
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("compensated unwind plan missing"));
    assert_eq!(plan.status, CloseRunUnwindPlanStatus::Compensated);
    assert!(plan.remaining_positions.is_empty());
    assert!(plan.next_actions.is_empty());
    let attempt = plan
        .compensation_attempts
        .first()
        .unwrap_or_else(|| panic!("compensation attempt missing"));
    assert_eq!(attempt.status, CloseLegStatus::Filled);
    assert_eq!(
        attempt
            .order
            .as_ref()
            .and_then(|order| order.filled_quantity),
        Some(1.0)
    );
}

#[test]
fn live_adapter_ack_without_fill_evidence_remains_pending() {
    let mut run = legacy_compensation_run(ExecutionMode::Live);

    assert!(!normalize_replayed_paper_finality(&mut run));
    assert_eq!(run.status, CloseRunStatus::CompensationSubmitted);
    let attempt = run
        .unwind_plan
        .as_ref()
        .and_then(|plan| plan.compensation_attempts.first())
        .unwrap_or_else(|| panic!("compensation attempt missing"));
    assert_eq!(attempt.status, CloseLegStatus::Accepted);
    assert!(attempt
        .order
        .as_ref()
        .is_some_and(|order| order.filled_quantity.is_none()));
}

fn legacy_compensation_run(mode: ExecutionMode) -> CloseRun {
    let mut filled_leg = close_leg("order-filled", CloseLegStatus::Filled);
    if let Some(order) = filled_leg.order.as_mut() {
        order.state = LiveOrderState::Filled;
        order.last_update_source = OrderUpdateSource::PrivateWs;
        order.filled_quantity = Some(1.0);
        order.filled_price = Some(100.0);
    }
    filled_leg.finality_source = Some(OrderUpdateSource::PrivateWs);
    filled_leg.confirmed_filled_at_ms = Some(2);

    let mut failed_leg = close_leg("order-failed", CloseLegStatus::Cancelled);
    if let Some(order) = failed_leg.order.as_mut() {
        order.state = LiveOrderState::Cancelled;
        order.last_update_source = OrderUpdateSource::PrivateWs;
    }

    let mut run = close_run("close-legacy", filled_leg);
    run.legs.push(failed_leg);
    run.expected_leg_count = 2;
    refresh_run_summary(&mut run);
    let candidate = run
        .unwind_plan
        .as_ref()
        .and_then(|plan| plan.compensation_candidates.first())
        .cloned()
        .unwrap_or_else(|| panic!("compensation candidate missing"));

    let mut order = compensation_order_record("close-legacy", LiveOrderState::Filled);
    order.intent.mode = mode;
    order.last_update_source = OrderUpdateSource::AdapterAck;
    let attempt = compensation_attempt_from_order(&candidate, &order);
    run.unwind_plan
        .as_mut()
        .unwrap_or_else(|| panic!("unwind plan missing"))
        .compensation_attempts
        .push(attempt);
    refresh_run_summary(&mut run);
    assert_eq!(run.status, CloseRunStatus::CompensationSubmitted);
    run
}
