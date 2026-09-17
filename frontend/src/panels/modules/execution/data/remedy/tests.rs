use super::*;
use shared_types::{
    ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole, LiveOrderState,
};

#[test]
fn cancelable_ids_only_include_open_leg_orders() {
    let mut run = run(ExecutionRunState::SubmittingSecondLeg);
    run.long_leg.state = LiveOrderState::Accepted;
    run.short_leg.state = LiveOrderState::Filled;

    assert_eq!(cancelable_order_ids(&run), vec!["long-order".to_owned()]);
}

#[test]
fn cancelable_ids_empty_when_orders_not_yet_at_venue_or_terminal() {
    let mut run = run(ExecutionRunState::Previewed);
    run.long_leg.state = LiveOrderState::Created;
    run.short_leg.state = LiveOrderState::Cancelled;

    assert!(cancelable_order_ids(&run).is_empty());
}

#[test]
fn partially_filled_leg_is_still_cancelable() {
    let mut run = run(ExecutionRunState::FirstLegPartial);
    run.long_leg.state = LiveOrderState::PartiallyFilled;
    run.short_leg.state = LiveOrderState::Created;

    assert_eq!(cancelable_order_ids(&run), vec!["long-order".to_owned()]);
}

#[test]
fn cancel_retries_get_distinct_attempt_keys_for_the_same_run_order() {
    let run = run(ExecutionRunState::SubmittingFirstLeg);
    let order_ids = vec!["long-order".to_owned()];

    let first = cancel_request_contexts(&run, &order_ids);
    let second = cancel_request_contexts(&run, &order_ids);
    let first_key = first[0].1.idempotency_key();
    let second_key = second[0].1.idempotency_key();

    assert_ne!(first_key, second_key);
    assert!(first_key.is_some_and(|key| key.starts_with("execution-cancel:run-1:long-order:web-")));
}

#[test]
fn hedged_run_needs_position_close_not_cancel() {
    let mut run = run(ExecutionRunState::Hedged);
    run.long_leg.state = LiveOrderState::Filled;
    run.short_leg.state = LiveOrderState::Filled;

    assert!(run_needs_position_close(&run));
    assert!(cancelable_order_ids(&run).is_empty());
}

#[test]
fn partial_fill_exposure_needs_position_close() {
    let mut run = run(ExecutionRunState::FirstLegPartial);
    run.long_leg.state = LiveOrderState::PartiallyFilled;
    run.long_leg.filled_quantity = Some(0.4);

    assert!(run_needs_position_close(&run));
}

#[test]
fn unfilled_run_does_not_need_position_close() {
    let mut run = run(ExecutionRunState::SubmittingFirstLeg);
    run.long_leg.state = LiveOrderState::Submitted;
    run.short_leg.state = LiveOrderState::Created;

    assert!(!run_needs_position_close(&run));
}

#[test]
fn closed_filled_run_does_not_offer_position_close_again() {
    let mut run = run(ExecutionRunState::Closed);
    run.long_leg.state = LiveOrderState::Filled;
    run.long_leg.filled_quantity = Some(1.0);
    run.short_leg.state = LiveOrderState::Filled;
    run.short_leg.filled_quantity = Some(1.0);

    assert!(!run_needs_position_close(&run));
}

fn run(state: ExecutionRunState) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state,
        long_leg: leg(HedgeLegRole::Long, "long-order"),
        short_leg: leg(HedgeLegRole::Short, "short-order"),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "submitted".into(),
        created_at_ms: 1,
        updated_at_ms: 1,
    }
}

fn leg(role: HedgeLegRole, order_id: &str) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "mock".into(),
        symbol: "BTCUSDT".into(),
        order_ids: vec![order_id.to_owned()],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Created,
        target_quantity: 1.0,
        filled_quantity: None,
        target_notional_usd: 100.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
