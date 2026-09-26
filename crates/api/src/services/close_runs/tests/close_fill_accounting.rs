use super::super::*;
use super::fixtures::*;

fn apply(run: &mut CloseRun, event: &ExecutionLedgerEvent) -> bool {
    let update = CloseLedgerUpdate::from_event(event);
    apply_close_ledger_event(run, event, update.as_ref())
}

fn snapshot(id: &str, qty: f64) -> ExecutionLedgerEvent {
    let mut event = ledger_fill_event(id, qty);
    event.event_id = format!("snapshot:{id}:{qty}");
    event.event_type = ExecutionLedgerEventType::FillSnapshot;
    event.source = OrderUpdateSource::OrderQuery;
    event
}

#[test]
fn cumulative_reply_and_individual_fills_are_not_added_twice_after_reload() {
    let mut run = close_run("mixed", close_leg("mixed-order", CloseLegStatus::Submitted));
    let first = ledger_fill_event("mixed-order", 0.4);
    assert!(apply(&mut run, &snapshot("mixed-order", 0.4)));
    assert!(apply(&mut run, &first));
    assert_eq!(
        run.legs[0].order.as_ref().unwrap().filled_quantity,
        Some(0.4)
    );
    assert_eq!(run.status, CloseRunStatus::Submitted);
    let saved = serde_json::to_vec(&run).unwrap();
    let mut run: CloseRun = serde_json::from_slice(&saved).unwrap();
    assert!(!apply(&mut run, &first));
    let mut second = ledger_fill_event("mixed-order", 0.6);
    second.occurred_at_ms = 20;
    if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut second.payload {
        fill.average_price = 110.0;
        fill.quote_value = 66.0;
    }
    assert!(apply(&mut run, &second));
    let order = run.legs[0].order.as_ref().unwrap();
    assert_eq!(order.filled_quantity, Some(1.0));
    assert_eq!(order.filled_price, Some(106.0));
    assert_eq!(order.filled_fee, Some(0.02));
    assert_eq!(run.legs[0].cost_events.len(), 3);
    assert_eq!(run.legs[0].ledger_fills.as_ref().unwrap().last_fill_at_ms, Some(20));
    assert_eq!(
        run.cost_reconciliation.as_ref().unwrap().close_fee_usd,
        Some(0.02)
    );
    assert_eq!(run.status, CloseRunStatus::Succeeded);
    assert!(run.has_complete_fills());
    assert!(!apply(&mut run, &second));
}

#[test]
fn partial_fill_time_survives_cancel_reload_and_later_order_queries() {
    let mut run = close_run("cancel-time", close_leg("cancel-time-order", CloseLegStatus::Submitted));
    let mut fill = ledger_fill_event("cancel-time-order", 0.4);
    fill.occurred_at_ms = 20;
    assert!(apply(&mut run, &fill));
    let mut cancelled = run.legs[0].order.clone().unwrap();
    cancelled.state = LiveOrderState::Cancelled;
    cancelled.last_update_source = OrderUpdateSource::OrderQuery;
    cancelled.updated_at_ms = 1_000;
    apply_order_update(&mut run, &cancelled);
    let saved = serde_json::to_vec(&run).unwrap();
    let mut run: CloseRun = serde_json::from_slice(&saved).unwrap();
    assert_eq!(run.legs[0].ledger_fills.as_ref().unwrap().last_fill_at_ms, Some(20));
    cancelled.updated_at_ms = 2_000;
    apply_order_update(&mut run, &cancelled);
    assert!(!apply(&mut run, &fill));
    assert_eq!(run.legs[0].ledger_fills.as_ref().unwrap().last_fill_at_ms, Some(20));
    assert_eq!(run.legs[0].status, CloseLegStatus::Cancelled);
    assert!(!run.has_complete_fills());
}

#[test]
fn local_cancellation_cannot_borrow_remote_fill_proof() {
    let mut run = close_run("cancel-proof", close_leg("cancel-proof-order", CloseLegStatus::Submitted));
    let fill = ledger_fill_event("cancel-proof-order", 0.4);
    assert!(apply(&mut run, &fill));
    let mut cancelled = run.legs[0].order.clone().unwrap();
    cancelled.state = LiveOrderState::Cancelled;
    cancelled.last_update_source = OrderUpdateSource::Internal;
    cancelled.updated_at_ms += 10;
    apply_order_update(&mut run, &cancelled);
    assert_eq!(run.legs[0].finality_source, Some(OrderUpdateSource::Internal));
    let late_fill = ledger_fill_event("cancel-proof-order", 0.1);
    assert!(apply(&mut run, &late_fill));
    assert_eq!(run.legs[0].finality_source, Some(OrderUpdateSource::Internal));
    assert_eq!(run.legs[0].order.as_ref().unwrap().filled_quantity, Some(0.5));
    cancelled.last_update_source = OrderUpdateSource::OrderQuery;
    cancelled.updated_at_ms += 10;
    cancelled.filled_quantity = Some(0.5);
    apply_order_update(&mut run, &cancelled);
    assert_eq!(run.legs[0].finality_source, Some(OrderUpdateSource::OrderQuery));
    assert!(!run.has_complete_fills());
}

#[test]
fn compensation_uses_the_same_independent_fill_totals_and_deduplication() {
    let order = compensation_order_record("mixed", LiveOrderState::Submitted);
    let id = order.intent.id.clone();
    let mut attempt = CloseRunCompensationAttempt {
        action_run_id: None,
        venue: "binance".into(),
        symbol: "MUUSDT".into(),
        side: PositionSide::Long,
        compensation_order_side: OrderSide::Buy,
        target_quantity: 1.0,
        status: CloseLegStatus::Submitted,
        order: Some(order),
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: None,
        ledger_fills: None,
        cost_events: Vec::new(),
        submitted_at_ms: 1,
        updated_at_ms: 2,
    };
    for mut event in [
        snapshot(&id, 0.4),
        ledger_fill_event(&id, 0.4),
        ledger_fill_event(&id, 0.6),
    ] {
        event.order.side = OrderSide::Buy;
        let update = CloseLedgerUpdate::from_event(&event).unwrap();
        assert!(apply_ledger_event_to_compensation_attempt(
            &mut attempt,
            &event,
            &update
        ));
        if event.event_type == ExecutionLedgerEventType::FillEvent {
            let saved = serde_json::to_vec(&attempt).unwrap();
            attempt = serde_json::from_slice(&saved).unwrap();
            assert!(!apply_ledger_event_to_compensation_attempt(
                &mut attempt,
                &event,
                &update
            ));
        }
    }
    assert_eq!(attempt.confirmed_filled_quantity(), Some(1.0));
    assert_eq!(attempt.unfilled_quantity(), Some(0.0));
    assert_eq!(attempt.cost_events.len(), 3);
}

#[test]
fn older_snapshots_and_late_acceptance_do_not_erase_confirmed_fills() {
    let mut run = close_run("late", close_leg("late-order", CloseLegStatus::Submitted));
    let mut full = ledger_fill_event("late-order", 1.0);
    full.occurred_at_ms = 30;
    assert!(apply(&mut run, &full));
    let mut old = snapshot("late-order", 1.0);
    if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut old.payload {
        fill.average_price = 99.0;
    }
    apply(&mut run, &old);
    let mut accepted = order_record("late-order", LiveOrderState::Accepted);
    accepted.updated_at_ms = 40;
    apply_order_update(&mut run, &accepted);
    assert_eq!(
        run.legs[0].order.as_ref().unwrap().filled_price,
        Some(100.0)
    );
    assert_eq!(
        run.legs[0].order.as_ref().unwrap().filled_quantity,
        Some(1.0)
    );
    assert_eq!(run.status, CloseRunStatus::Succeeded);
    assert!(run.has_complete_fills());

    accepted.last_update_source = OrderUpdateSource::AdapterAck;
    accepted.filled_quantity = Some(9.0);
    accepted.filled_price = Some(99.0);
    run.legs[0].order.as_mut().unwrap().intent.mode = ExecutionMode::Live;
    accepted.intent.mode = ExecutionMode::Live;
    apply_order_update(&mut run, &accepted);
    assert_eq!(
        run.legs[0].order.as_ref().unwrap().filled_quantity,
        Some(1.0)
    );
}

#[test]
fn local_cancel_keeps_confirmed_quantity_without_ignoring_the_control_update() {
    let mut previous = order_record("cancel-local", LiveOrderState::PartiallyFilled);
    previous.last_update_source = OrderUpdateSource::AdapterAck;
    previous.filled_quantity = Some(0.4);
    previous.filled_price = Some(100.0);
    for state in [LiveOrderState::CancelRequested, LiveOrderState::Cancelled] {
        let mut incoming = previous.clone();
        incoming.state = state;
        incoming.last_update_source = OrderUpdateSource::Internal;
        incoming.filled_quantity = None;
        incoming.updated_at_ms += 1;
        previous = merge_close_order(Some(&previous), &incoming);
        assert_eq!(previous.state, state);
        assert_eq!(previous.filled_quantity, Some(0.4));
    }
}

#[test]
fn live_ack_estimates_and_excess_quantity_never_prove_complete_close() {
    let mut run = close_run(
        "unproven",
        close_leg("unproven-order", CloseLegStatus::Submitted),
    );
    run.legs[0].order.as_mut().unwrap().intent.mode = ExecutionMode::Live;
    let mut ack = snapshot("unproven-order", 1.0);
    ack.source = OrderUpdateSource::AdapterAck;
    assert!(!apply(&mut run, &ack));
    let mut estimated = ledger_fill_event("unproven-order", 1.0);
    if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut estimated.payload {
        fill.quality = ExecutionLedgerQuality::Estimated;
    }
    assert!(!apply(&mut run, &estimated));
    assert!(!run.has_complete_fills());
    assert!(apply(&mut run, &ledger_fill_event("unproven-order", 1.5)));
    assert!(!run.has_complete_fills());
    assert_ne!(run.status, CloseRunStatus::Succeeded);
    assert_eq!(run.exposure_estimate_usd(), None);
}

#[test]
fn unresolved_zero_is_unknown_but_confirmed_close_can_display_zero() {
    let mut run = close_run("zero", close_leg("zero-order", CloseLegStatus::Submitted));
    assert_eq!(run.exposure_estimate_usd(), None);
    run.naked_exposure_usd = 10.0;
    assert_eq!(run.exposure_estimate_usd(), Some(10.0));
    assert!(apply(&mut run, &ledger_fill_event("zero-order", 1.0)));
    assert_eq!(run.exposure_estimate_usd(), Some(0.0));
    run.finality_problem = Some(ApiProblem::new("TEST_PENDING", "awaiting order query"));
    assert_eq!(run.exposure_estimate_usd(), None);
    run.finality_problem = None;
    run.legs[0].order.as_mut().unwrap().filled_quantity = None;
    assert_eq!(run.exposure_estimate_usd(), None);
}
