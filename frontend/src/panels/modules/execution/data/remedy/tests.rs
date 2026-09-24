use super::*;
use shared_types::{
    ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole, LiveOrderState,
};

#[test]
fn cancel_resolution_requires_every_order_terminal_and_preserves_fill_warning() {
    let evidence = ActionEvidence {
        order_ids: vec!["a".into(), "b".into()],
        ..Default::default()
    };
    let mut a = record("a", LiveOrderState::Cancelled);
    let mut b = record("b", LiveOrderState::CancelRequested);
    assert!(settled_cancel_state(&evidence, &[a.clone(), b.clone()]).is_none());
    b.state = LiveOrderState::Failed;
    assert!(settled_cancel_state(&evidence, &[a.clone(), b.clone()]).is_none());
    b.state = LiveOrderState::Rejected;
    assert!(matches!(
        settled_cancel_state(&evidence, &[a.clone(), b.clone()]),
        Some(ActionState::Succeeded { .. })
    ));
    a.filled_quantity = Some(0.1);
    let state = settled_cancel_state(&evidence, &[a, b]).unwrap();
    assert_eq!(
        state.problem().map(|p| p.code.as_str()),
        Some("CANCEL_ORDER_HAS_FILLS")
    );
    assert!(state.label().unwrap().contains("1 笔已有成交"));
}

#[test]
fn order_finality_precedes_stale_run_for_cancel_and_position_handoff() {
    let mut run = run(ExecutionRunState::SecondLegSubmitted);
    run.long_leg.state = LiveOrderState::Accepted;
    run.short_leg.state = LiveOrderState::Accepted;
    let rows = vec![
        record("long-order", LiveOrderState::Filled),
        record("short-order", LiveOrderState::CancelRequested),
    ];
    assert!(cancelable_order_ids_with_records(&run, &rows).is_empty());
    assert!(run_orders_have_fill(&run, &rows));
    assert!(!run_orders_have_fill(
        &run,
        &[record("unrelated", LiveOrderState::Filled)]
    ));
    run.state = ExecutionRunState::Closed;
    assert!(!run_orders_have_fill(&run, &rows));
}

fn record(id: &str, state: LiveOrderState) -> OrderRecord {
    serde_json::from_value(serde_json::json!({
        "intent": { "id": id, "source": "manual", "mode": "dry_run", "exchange": "fixture", "symbol": "BTC",
            "side": "buy", "orderType": "limit", "quantity": 1, "price": 100, "reduceOnly": false,
            "timeInForce": "ioc", "postOnly": false, "marginMode": "cross", "leverage": 1,
            "clientOrderId": id, "createdAtMs": 1 },
        "state": state, "lastUpdateSource": "private_ws", "updatedAtMs": 2
    })).unwrap()
}

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
fn rejected_without_fills_does_not_offer_close_and_cancel_ids_are_unique() {
    let mut row = run(ExecutionRunState::FailedSafe);
    row.long_leg.state = LiveOrderState::Rejected;
    assert!(!run_needs_position_close(&row));
    assert!(!run_is_released(&row));
    row.long_leg.filled_quantity = Some(0.0);
    row.short_leg.state = LiveOrderState::Rejected;
    row.short_leg.filled_quantity = Some(0.0);
    assert!(run_is_released(&row));
    row.long_leg.state = LiveOrderState::Accepted;
    row.short_leg.state = LiveOrderState::Accepted;
    row.short_leg.order_ids = row.long_leg.order_ids.clone();
    assert_eq!(cancelable_order_ids(&row), vec!["long-order"]);
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
