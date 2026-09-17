use super::close_run;
use crate::panels::modules::positions::data::{close_run_problem, close_run_retry_anchor};
use shared_types::{
    ApiProblem, CloseLeg, CloseLegStatus, CloseRunStatus, LiveOrderState, OrderUpdateSource,
    PositionSide,
};

#[test]
fn single_leg_close_surfaces_the_actionable_venue_problem() {
    let mut run = close_run(CloseRunStatus::Failed, 0, 1, 29.0);
    run.expected_leg_count = 1;
    run.legs = vec![failed_close_leg(ApiProblem::new(
        shared_types::problem::codes::UPSTREAM_API,
        "binance api error: code=-2015 msg=Invalid API-key, IP, or permissions for action",
    ))];

    let problem = close_run_problem(&run);

    assert_eq!(
        problem.code,
        shared_types::problem::codes::CREDENTIAL_PERMISSION_DENIED
    );
    assert!(problem.message.contains("-2015"));
    assert_eq!(problem.status, Some(403));
    assert_eq!(
        problem.recovery_action,
        Some(shared_types::ApiRecoveryAction::CheckPermissions)
    );
    assert_eq!(problem.request_id.as_deref(), Some("req-1"));
}

#[test]
fn definitive_not_submitted_failure_allows_an_explicit_retry() {
    let mut run = close_run(CloseRunStatus::Failed, 0, 1, 29.0);
    run.id = "close-failed-1".to_owned();
    run.expected_leg_count = 1;
    run.legs = vec![failed_close_leg(ApiProblem::new(
        shared_types::problem::codes::CREDENTIAL_PERMISSION_DENIED,
        "permission denied",
    ))];

    assert_eq!(
        close_run_retry_anchor(&run),
        Some(("close-failed-1".to_owned(), run.updated_at_ms))
    );
}

#[test]
fn ambiguous_transport_failure_keeps_the_original_idempotency_scope() {
    let mut run = close_run(CloseRunStatus::Failed, 0, 1, 29.0);
    run.expected_leg_count = 1;
    run.legs = vec![failed_close_leg(ApiProblem::new(
        shared_types::problem::codes::UPSTREAM_API,
        "order submission timed out",
    ))];

    assert_eq!(close_run_retry_anchor(&run), None);
}

#[test]
fn confirmed_absent_bitget_order_advances_the_explicit_retry_scope() {
    let mut run = close_run(CloseRunStatus::Failed, 1, 1, 7.0);
    run.id = "close-bitget-absent".to_owned();
    run.expected_leg_count = 1;
    let mut order = super::order_record("bitget-close-1", LiveOrderState::Failed);
    order.intent.exchange = "bitget".to_owned();
    order.exchange_order_id = None;
    order.last_update_source = OrderUpdateSource::Reconcile;
    order.message = Some(
        "bitget order was not found by clientOid after the 60000ms ambiguity window; submission was not retried"
            .to_owned(),
    );
    run.legs = vec![terminal_close_leg(order)];

    assert_eq!(
        close_run_retry_anchor(&run),
        Some(("close-bitget-absent".to_owned(), run.updated_at_ms))
    );
}

#[test]
fn failed_order_with_possible_fill_does_not_advance_retry_scope() {
    let mut run = close_run(CloseRunStatus::Failed, 1, 1, 7.0);
    run.expected_leg_count = 1;
    let mut order = super::order_record("bitget-close-1", LiveOrderState::Failed);
    order.intent.exchange = "bitget".to_owned();
    order.exchange_order_id = None;
    order.last_update_source = OrderUpdateSource::Reconcile;
    order.filled_quantity = Some(0.01);
    order.message = Some(
        "bitget order was not found by clientOid after the 60000ms ambiguity window; submission was not retried"
            .to_owned(),
    );
    run.legs = vec![terminal_close_leg(order)];

    assert_eq!(close_run_retry_anchor(&run), None);
}

fn failed_close_leg(problem: ApiProblem) -> CloseLeg {
    CloseLeg {
        venue: "binance".to_owned(),
        symbol: "SOL".to_owned(),
        side: PositionSide::Long,
        status: CloseLegStatus::Failed,
        quantity: 0.4,
        mark_price: 72.5,
        notional_usd: 29.0,
        order: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: Some(problem),
        pair_evidence: None,
        cost_events: Vec::new(),
    }
}

fn terminal_close_leg(order: shared_types::OrderRecord) -> CloseLeg {
    CloseLeg {
        venue: "bitget".to_owned(),
        symbol: "SOL".to_owned(),
        side: PositionSide::Long,
        status: CloseLegStatus::Failed,
        quantity: 0.1,
        mark_price: 70.0,
        notional_usd: 7.0,
        order: Some(order),
        finality_source: Some(OrderUpdateSource::Reconcile),
        confirmed_filled_at_ms: None,
        problem: Some(ApiProblem::new(
            shared_types::problem::codes::CLOSE_RUN_FAILED,
            "close order reached a non-filled terminal state",
        )),
        pair_evidence: None,
        cost_events: Vec::new(),
    }
}
