use super::super::*;
use super::fixtures::*;
use super::*;

fn submitted_run() -> ExecutionRun {
    let mut run = run("cancel-finality", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");
    run.short_leg = leg_with_order(HedgeLegRole::Short, "short");
    run
}

fn cancelled(id: &str, quantity: Option<f64>) -> OrderRecord {
    let mut record = filled_record(id, 0.0);
    record.state = LiveOrderState::Cancelled;
    record.filled_quantity = quantity;
    record
}

#[test]
fn cancellation_requires_explicit_zero_and_preserves_existing_fills() {
    let mut run = submitted_run();
    assert!(apply_order_update(&mut run, &cancelled("long", Some(0.0))));
    assert_ne!(run.state, ExecutionRunState::FailedSafe);
    assert!(apply_order_update(&mut run, &cancelled("short", None)));
    assert_ne!(run.state, ExecutionRunState::FailedSafe);
    assert!(apply_order_update(&mut run, &cancelled("short", Some(0.0))));
    assert_eq!(run.state, ExecutionRunState::FailedSafe);
    assert_eq!(run.long_leg.filled_quantity, Some(0.0));
    assert_eq!(run.short_leg.filled_quantity, Some(0.0));
    assert_eq!(run.recovery_action, None);

    let mut partial = submitted_run();
    assert!(apply_order_update(&mut partial, &cancelled("long", Some(0.4))));
    assert!(apply_order_update(&mut partial, &cancelled("short", Some(0.0))));
    assert_eq!(partial.state, ExecutionRunState::UnwindRequired);
    assert_eq!(partial.net_exposure_usd, 40.0);
    assert_eq!(partial.recovery_action, Some(RecoveryAction::UnwindLongLeg));
    // A later incomplete cancellation message cannot erase the known fill.
    assert!(apply_order_update(&mut partial, &cancelled("long", None)));
    assert_eq!(partial.long_leg.filled_quantity, Some(0.4));
    assert_eq!(partial.net_exposure_usd, 40.0);
    // Nor may a contradictory zero receipt erase it or release the execution.
    assert!(apply_order_update(&mut partial, &cancelled("long", Some(0.0))));
    assert_eq!(partial.long_leg.filled_quantity, Some(0.4));
    assert_eq!(partial.net_exposure_usd, 40.0);
    assert_eq!(partial.recovery_action, Some(RecoveryAction::ManualReview));
    assert!(apply_order_update(&mut partial, &cancelled("short", Some(0.0))));
    assert_eq!(partial.recovery_action, Some(RecoveryAction::ManualReview));
    assert!(apply_order_update(&mut partial, &cancelled("long", Some(0.4))));
    assert_eq!(partial.recovery_action, Some(RecoveryAction::UnwindLongLeg));
    assert!(partial.valuation_problem.is_none());
}

#[test]
fn balanced_partial_cancellations_are_not_unfilled_or_closed() {
    let mut run = submitted_run();
    apply_order_update(&mut run, &cancelled("long", Some(0.4)));
    apply_order_update(&mut run, &cancelled("short", Some(0.4)));
    assert_eq!(run.net_exposure_usd, 0.0);
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::ManualReview));
    assert_eq!(run.long_leg.filled_quantity, Some(0.4));
    assert_eq!(run.short_leg.filled_quantity, Some(0.4));
}

#[test]
fn cancel_finality_requires_remote_quantities_and_preserves_terminal_state() {
    let mut run = submitted_run();
    let mut ack = cancelled("long", Some(0.0));
    ack.last_update_source = OrderUpdateSource::AdapterAck;
    apply_order_update(&mut run, &ack);
    apply_order_update(&mut run, &cancelled("short", Some(0.0)));
    assert_ne!(run.state, ExecutionRunState::FailedSafe);
    assert!(!run.orders_ended_without_fills());
    let mut wrong = cancelled("long", Some(0.0));
    wrong.intent.exchange = "other".into();
    assert!(!apply_order_update(&mut run, &wrong));
    apply_order_update(&mut run, &cancelled("long", Some(0.0)));
    assert!(run.orders_ended_without_fills());
    assert_eq!(run.state, ExecutionRunState::FailedSafe);
    ack.state = LiveOrderState::Accepted;
    assert!(!apply_order_update(&mut run, &ack));
    assert!(run.orders_ended_without_fills());
    assert_eq!(run.long_leg.state, LiveOrderState::Cancelled);

    for quantity in [None, Some(-1.0), Some(f64::NAN)] {
        let mut uncertain = submitted_run();
        apply_order_update(&mut uncertain, &cancelled("long", quantity));
        apply_order_update(&mut uncertain, &cancelled("short", Some(0.0)));
        assert!(!uncertain.orders_ended_without_fills());
        assert_eq!(uncertain.state, ExecutionRunState::UnwindRequired);
        assert_eq!(uncertain.recovery_action, Some(RecoveryAction::ManualReview));
    }
}

#[test]
fn cancel_finality_late_fill_does_not_reopen_order_or_erase_exposure() {
    let mut run = submitted_run();
    apply_order_update(&mut run, &cancelled("long", None));
    apply_order_update(&mut run, &cancelled("short", Some(0.0)));
    let fill = ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 0.4, 40.0, Some(0.01));
    assert!(apply_ledger_event_update(&mut run, &fill));
    assert_eq!(run.long_leg.state, LiveOrderState::Cancelled);
    assert_eq!(run.long_leg.filled_quantity, Some(0.4));
    assert_eq!(run.net_exposure_usd, 40.0);
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::UnwindLongLeg));
}
