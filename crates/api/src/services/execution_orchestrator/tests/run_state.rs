use super::super::legs::recovery_action_for_role;
use super::super::run_model::run_state_after_second;
use super::super::*;
use super::*;

#[test]
fn run_state_requires_both_legs_filled_before_hedged() {
    let accepted = order_record(LiveOrderState::Accepted);
    let mut filled = order_record(LiveOrderState::Filled);
    filled.filled_quantity = Some(1.0);

    assert_eq!(
        run_state_after_second(&accepted, &accepted),
        ExecutionRunState::SecondLegSubmitted
    );
    assert_eq!(
        run_state_after_second(&filled, &accepted),
        ExecutionRunState::SecondLegSubmitted
    );
    assert_eq!(
        run_state_after_second(&filled, &filled),
        ExecutionRunState::Hedged
    );
}

#[test]
fn run_state_marks_second_leg_terminal_failure_as_unwind_required() {
    let mut filled = order_record(LiveOrderState::Filled);
    filled.filled_quantity = Some(1.0);
    let failed = order_record(LiveOrderState::Failed);

    assert_eq!(
        run_state_after_second(&filled, &failed),
        ExecutionRunState::UnwindRequired
    );
}

#[test]
fn short_first_records_return_in_business_role_slots() {
    let mut first_short = order_record(LiveOrderState::Filled);
    first_short.intent.side = OrderSide::Sell;
    first_short.intent.exchange = "bybit".into();
    let mut second_long = order_record(LiveOrderState::Filled);
    second_long.intent.side = OrderSide::Buy;
    second_long.intent.exchange = "okx".into();

    let (long, short) = records_by_role(HedgeLegRole::Short, Some(first_short), Some(second_long));

    assert_eq!(
        long.as_ref().map(|record| record.intent.side),
        Some(OrderSide::Buy)
    );
    assert_eq!(
        short.as_ref().map(|record| record.intent.side),
        Some(OrderSide::Sell)
    );
    assert_eq!(
        recovery_action_for_role(HedgeLegRole::Short),
        RecoveryAction::UnwindShortLeg
    );
}
