use crate::panels::modules::positions::data::{
    close_run_action_state, latest_position_close_attempt_anchor, should_recover_close_run,
};
use shared_types::{
    CloseLeg, CloseLegStatus, CloseRunScope, CloseRunStatus, PositionRow, PositionSide,
};

use super::super::support::{close_run_with_id, portfolio_snapshot, row};

#[test]
fn older_close_run_cannot_overwrite_a_newer_attempt_state() {
    let mut current_run = close_run_with_id("close-current", CloseRunStatus::Submitted, 20);
    current_run.idempotency_key = Some("close-current-key".to_owned());
    let current = close_run_action_state("平仓", &current_run);
    let mut previous_run = close_run_with_id("close-previous", CloseRunStatus::Succeeded, 10);
    previous_run.idempotency_key = Some("close-previous-key".to_owned());

    assert!(!should_recover_close_run(&current, &previous_run));
}

#[test]
fn same_close_attempt_accepts_a_newer_finality_update() {
    let mut submitted = close_run_with_id("close-current", CloseRunStatus::Submitted, 20);
    submitted.idempotency_key = Some("close-current-key".to_owned());
    let current = close_run_action_state("平仓", &submitted);
    let mut succeeded = close_run_with_id("close-current", CloseRunStatus::Succeeded, 30);
    succeeded.idempotency_key = Some("close-current-key".to_owned());

    assert!(should_recover_close_run(&current, &succeeded));
}

#[test]
fn completed_close_advances_reopened_position_but_in_flight_close_does_not() {
    let reopened = row("binance", "SOLUSDT", PositionSide::Long, None);
    let mut completed = close_run_with_id("close-completed", CloseRunStatus::Succeeded, 10);
    completed.scope = CloseRunScope::Single;
    completed.expected_leg_count = 1;
    completed.legs = vec![close_leg(&reopened, CloseLegStatus::Filled)];
    let mut in_flight = close_run_with_id("close-in-flight", CloseRunStatus::Submitted, 20);
    in_flight.scope = CloseRunScope::Single;
    in_flight.expected_leg_count = 1;
    in_flight.legs = vec![close_leg(&reopened, CloseLegStatus::Submitted)];
    let mut snapshot = portfolio_snapshot("positions-v1");
    snapshot.positions = vec![reopened.clone()];
    snapshot.recent_close_runs = vec![in_flight, completed];

    assert_eq!(
        latest_position_close_attempt_anchor(&snapshot, &reopened, 1),
        Some(("close-completed".to_owned(), 10))
    );
}

fn close_leg(row: &PositionRow, status: CloseLegStatus) -> CloseLeg {
    CloseLeg {
        venue: row.venue.clone(),
        symbol: row.symbol.clone(),
        side: row.side,
        status,
        quantity: row.quantity,
        mark_price: row.mark_price,
        notional_usd: row.quantity * row.mark_price,
        order: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: None,
        pair_evidence: None,
        cost_events: Vec::new(),
    }
}
