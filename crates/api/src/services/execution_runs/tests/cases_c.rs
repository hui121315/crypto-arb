use super::super::*;
use super::fixtures::*;
use super::*;

#[test]
fn ledger_filled_state_without_fill_snapshot_requires_manual_review() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    let event = ledger_state_event_row(
        &run,
        HedgeLegRole::Long,
        "ex-long",
        ExecutionLedgerEventType::OrderState,
        LiveOrderState::Filled,
    );

    assert!(apply_ledger_event_update(&mut run, &event));

    assert_eq!(run.long_leg.state, LiveOrderState::Accepted);
    assert_eq!(run.long_leg.filled_quantity, None);
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::ManualReview));
    assert_eq!(
        run.valuation_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_EXECUTION_VALUATION_MISSING)
    );
}

#[test]
fn stale_submitted_ledger_state_cannot_downgrade_confirmed_fill() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    let fill_event =
        ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 1.0, 100.0, Some(0.1));

    assert!(apply_ledger_event_update(&mut run, &fill_event));
    assert_eq!(run.long_leg.state, LiveOrderState::Filled);
    let confirmed_at_ms = run.long_leg.confirmed_filled_at_ms;

    let stale_state = ledger_state_event_row(
        &run,
        HedgeLegRole::Long,
        "ex-long",
        ExecutionLedgerEventType::OrderState,
        LiveOrderState::Submitted,
    );
    assert!(apply_ledger_event_update(&mut run, &stale_state));

    assert_eq!(run.long_leg.state, LiveOrderState::Filled);
    assert_eq!(run.long_leg.confirmed_filled_at_ms, confirmed_at_ms);
    assert_eq!(run.long_leg.filled_quantity, Some(1.0));
    assert_eq!(run.long_leg.filled_notional_usd, Some(100.0));
    assert_eq!(run.valuation_problem, None);
}

#[test]
fn filled_state_after_fill_snapshot_reuses_existing_valuation() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    let fill_event =
        ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 1.0, 100.0, Some(0.1));

    assert!(apply_ledger_event_update(&mut run, &fill_event));
    let filled_state = ledger_state_event_row(
        &run,
        HedgeLegRole::Long,
        "ex-long",
        ExecutionLedgerEventType::OrderState,
        LiveOrderState::Filled,
    );
    assert!(apply_ledger_event_update(&mut run, &filled_state));

    assert_eq!(run.long_leg.state, LiveOrderState::Filled);
    assert_eq!(run.long_leg.filled_quantity, Some(1.0));
    assert_eq!(run.long_leg.filled_notional_usd, Some(100.0));
    assert_eq!(run.valuation_problem, None);
}

#[test]
fn missing_fill_price_requires_manual_valuation_review() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");

    let mut record = filled_record("long", 0.0);
    record.filled_price = None;
    record.intent.price = None;

    assert!(apply_order_update(&mut run, &record));

    assert_eq!(run.long_leg.filled_quantity, Some(1.0));
    assert_eq!(run.long_leg.filled_notional_usd, None);
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::ManualReview));
    assert_eq!(
        run.valuation_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_EXECUTION_VALUATION_MISSING)
    );
    assert_eq!(run.net_exposure_usd, 0.0);
}

#[test]
fn filled_state_without_fill_quantity_does_not_infer_from_intent() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");

    let mut record = filled_record("long", 0.0);
    record.filled_quantity = None;
    record.filled_price = Some(100.0);

    assert!(apply_order_update(&mut run, &record));

    assert_eq!(run.long_leg.state, LiveOrderState::Filled);
    assert_eq!(run.long_leg.filled_quantity, None);
    assert_eq!(run.long_leg.filled_notional_usd, None);
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::ManualReview));
    assert_eq!(
        run.valuation_problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("reason"))
            .and_then(serde_json::Value::as_str),
        Some("missing_filled_quantity")
    );
}

#[test]
fn ledger_fill_without_fee_preserves_prior_fee() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.long_leg.filled_fee = Some(0.1);
    let event = ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 0.2, 20.0, None);
    let fill = ledger_fill_event(&event).ok_or("fill payload missing")?;

    assert!(apply_ledger_fill_update(&mut run, &event, fill));

    assert_eq!(run.long_leg.filled_fee, Some(0.1));
    Ok(())
}

#[test]
fn reduce_only_recovery_fill_closes_run_exposure() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::Unwinding;
    run.recovery_action = Some(RecoveryAction::UnwindLongLeg);
    run.net_exposure_usd = 100.0;
    run.unwind_problem = Some(ApiProblem::new(
        codes::HEDGE_UNWIND_SUBMIT_FAILED,
        "previous failure",
    ));
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long-unwind");

    assert!(apply_order_update(
        &mut run,
        &filled_record("long-unwind", 0.0).reduce_only()
    ));

    assert_eq!(run.state, ExecutionRunState::Closed);
    assert_eq!(run.net_exposure_usd, 0.0);
    assert_eq!(run.recovery_action, None);
    assert_eq!(run.unwind_problem, None);
}

#[test]
fn reduce_only_ledger_fill_records_unwind_fee_without_polluting_open_leg(
) -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::Unwinding;
    run.recovery_action = Some(RecoveryAction::UnwindLongLeg);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.long_leg.state = LiveOrderState::Filled;
    run.long_leg.filled_quantity = Some(1.0);
    run.long_leg.filled_notional_usd = Some(100.0);
    run.long_leg.filled_fee = Some(0.1);
    run.cost_reconciliation = Some(cost());
    let mut event = ledger_fill_event_row(
        &run,
        HedgeLegRole::Long,
        "ex-long-unwind",
        1.0,
        100.0,
        Some(0.4),
    );
    event.order.reduce_only = Some(true);
    let fill = ledger_fill_event(&event).ok_or("fill payload missing")?;

    assert!(apply_ledger_fill_update(&mut run, &event, fill));

    assert_eq!(run.long_leg.filled_quantity, Some(1.0));
    assert_eq!(run.long_leg.filled_notional_usd, Some(100.0));
    assert_eq!(run.long_leg.filled_fee, Some(0.1));
    let cost = run.cost_reconciliation.ok_or("cost missing")?;
    assert_close_option(cost.actual_unwind_fee_usd, 0.4)?;
    assert_eq!(cost.actual_unwind_cost_usd, None);
    assert_eq!(cost.unwind_event_ids, [event.event_id]);
    assert!(cost
        .missing_fields
        .iter()
        .any(|field| field == "actualUnwindSlippageUsd"));
    Ok(())
}

#[test]
fn reduce_only_ledger_fill_and_slippage_compute_unwind_total_cost() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::Unwinding;
    run.recovery_action = Some(RecoveryAction::UnwindLongLeg);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.short_leg = leg_with_order(HedgeLegRole::Short, "ex-short");
    run.long_leg.state = LiveOrderState::Filled;
    run.long_leg.filled_quantity = Some(1.0);
    run.long_leg.filled_notional_usd = Some(100.0);
    run.long_leg.filled_fee = Some(0.1);
    run.short_leg.state = LiveOrderState::Filled;
    run.short_leg.filled_quantity = Some(1.0);
    run.short_leg.filled_notional_usd = Some(100.0);
    run.short_leg.filled_fee = Some(0.2);
    run.cost_reconciliation = Some(cost());
    let mut fill_event = ledger_fill_event_row(
        &run,
        HedgeLegRole::Long,
        "ex-long-unwind",
        1.0,
        100.0,
        Some(0.4),
    );
    fill_event.order.reduce_only = Some(true);
    let mut slippage_event =
        ledger_slippage_event_row(&run, HedgeLegRole::Long, "ex-long-unwind", 0.7);
    slippage_event.order.reduce_only = Some(true);

    assert!(apply_ledger_event_update(&mut run, &fill_event));
    assert!(apply_ledger_event_update(&mut run, &slippage_event));
    assert!(!apply_ledger_event_update(&mut run, &slippage_event));

    let cost = run.cost_reconciliation.ok_or("cost missing")?;
    assert_close_option(cost.filled_fee_usd, 0.3)?;
    assert_close_option(cost.actual_open_cost_usd, 0.3)?;
    assert_close_option(cost.actual_unwind_fee_usd, 0.4)?;
    assert_close_option(cost.actual_unwind_slippage_usd, 0.7)?;
    assert_close_option(cost.actual_unwind_cost_usd, 1.1)?;
    assert_close_option(cost.actual_cost_usd, 1.4)?;
    assert_close_option(cost.cost_delta_usd, 0.4)?;
    assert_eq!(
        cost.unwind_event_ids,
        [fill_event.event_id, slippage_event.event_id]
    );
    assert!(cost.missing_fields.is_empty());
    Ok(())
}

#[test]
fn reduce_only_recovery_failure_requires_manual_review() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::Unwinding;
    run.recovery_action = Some(RecoveryAction::UnwindLongLeg);
    run.net_exposure_usd = 100.0;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long-unwind");

    assert!(apply_order_update(
        &mut run,
        &failed_record("long-unwind").reduce_only()
    ));

    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::ManualReview));
    assert!(run.status_reason.contains("人工复核"));
    assert_eq!(
        run.unwind_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_UNWIND_FINALITY_FAILED)
    );
    assert_eq!(
        run.unwind_problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("orderId"))
            .and_then(|value| value.as_str()),
        Some("long-unwind")
    );
}

#[test]
fn finality_problem_updates_matching_run_order() {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");
    let problem = ApiProblem::new(codes::HEDGE_ORDER_FINALITY_FAILED, "order query failed")
        .with_status(502)
        .with_source("run_finality");

    assert!(apply_finality_problem(&mut run, "long", &problem, 9));

    assert_eq!(
        run.finality_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_ORDER_FINALITY_FAILED)
    );
    assert_eq!(run.finality_checked_at_ms, Some(9));
    assert_eq!(run.updated_at_ms, 9);
}

#[test]
fn order_query_success_clears_finality_problem() {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");
    run.finality_problem = Some(ApiProblem::new(
        codes::HEDGE_ORDER_FINALITY_FAILED,
        "previous query failed",
    ));
    run.finality_checked_at_ms = Some(4);
    let mut record = filled_record("long", 0.0);
    record.last_update_source = OrderUpdateSource::OrderQuery;
    record.updated_at_ms = 9;

    assert!(apply_order_update(&mut run, &record));

    assert_eq!(run.finality_problem, None);
    assert_eq!(run.finality_checked_at_ms, Some(9));
}

#[test]
fn unrelated_order_update_is_ignored() {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");

    assert!(!apply_order_update(&mut run, &filled_record("other", 0.0)));
}

#[test]
fn exchange_order_id_collision_cannot_cross_execution_run_identity() {
    let mut run = run("hedge", 1);
    let mut original = filled_record("original", 0.0);
    original.exchange_order_id = Some("reused-exchange-id".into());
    run.long_leg = leg_with_order(HedgeLegRole::Long, "original");
    run.long_leg.identity = Some(original.identity_snapshot());
    register_order_ids(&mut run.long_leg, &original);
    let mut newer = filled_record("newer", 0.0);
    newer.exchange_order_id = Some("reused-exchange-id".into());

    assert!(!leg_matches_order(&run.long_leg, &newer));
    assert!(!apply_order_update(&mut run, &newer));
}
