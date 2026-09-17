use super::super::events::combine_errors;
use super::super::unwind::{mark_unwind_failed, unwind_intent};
use super::super::*;
use super::*;
use shared_types::{problem::codes, ApiProblem, ExecutionMode, OrderSource};

#[test]
fn unwind_uses_reduce_only_market_order() {
    let open_leg = OrderIntent {
        id: "open".into(),
        source: OrderSource::ArbitragePreview,
        strategy: None,
        mode: ExecutionMode::Live,
        exchange: "okx".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(100.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: "open-client".into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    };

    let open_record = OrderRecord {
        intent: open_leg,
        state: LiveOrderState::Filled,
        risk: None,
        identity: Default::default(),
        last_update_source: OrderUpdateSource::OrderQuery,
        exchange_order_id: Some("exchange-open".into()),
        message: None,
        filled_quantity: Some(0.4),
        filled_price: Some(100.0),
        filled_fee: Some(0.2),
        updated_at_ms: 1,
    };

    let unwind = unwind_intent(&open_record, "hedge-key");

    assert_eq!(unwind.side, OrderSide::Sell);
    assert_eq!(unwind.order_type, OrderType::Market);
    assert_eq!(unwind.quantity, 0.4);
    assert_eq!(unwind.price, Some(100.0));
    assert!(unwind.reduce_only);
}

#[test]
fn unwind_failure_keeps_run_visible_for_manual_review() {
    let mut run = ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state: ExecutionRunState::Unwinding,
        long_leg: leg_with_fee(HedgeLegRole::Long, None),
        short_leg: leg_with_fee(HedgeLegRole::Short, None),
        net_exposure_usd: 100.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: Some(RecoveryAction::UnwindLongLeg),
        status_reason: "unwinding".into(),
        created_at_ms: 1,
        updated_at_ms: 2,
    };

    let problem = mark_unwind_failed(
        &mut run,
        "反向 unwind 提交失败",
        &ApiProblem::new("UPSTREAM_HTTP", "route down"),
    );

    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::ManualReview));
    assert_eq!(problem.code, codes::HEDGE_UNWIND_SUBMIT_FAILED);
    assert_eq!(problem.message, "反向 unwind 提交失败: route down");
    assert_eq!(
        run.unwind_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_UNWIND_SUBMIT_FAILED)
    );
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("runId"))
            .and_then(|value| value.as_str()),
        Some("run-1")
    );
}

#[test]
fn unwind_failure_problem_preserves_request_id_retry_after() {
    let mut run = ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state: ExecutionRunState::Unwinding,
        long_leg: leg_with_fee(HedgeLegRole::Long, None),
        short_leg: leg_with_fee(HedgeLegRole::Short, None),
        net_exposure_usd: 100.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: Some(RecoveryAction::UnwindLongLeg),
        status_reason: "unwinding".into(),
        created_at_ms: 1,
        updated_at_ms: 2,
    };
    let submit_problem = ApiProblem::new("RATE_LIMITED", "exchange rate limited")
        .with_status(429)
        .with_request_id(Some("req-unwind-429".into()))
        .with_retry_after_ms(Some(4_000))
        .with_source("exchange");

    let problem = mark_unwind_failed(&mut run, "反向 unwind 提交失败", &submit_problem);

    assert_eq!(problem.code, codes::HEDGE_UNWIND_SUBMIT_FAILED);
    assert_eq!(problem.status, Some(429));
    assert_eq!(problem.request_id.as_deref(), Some("req-unwind-429"));
    assert_eq!(problem.retry_after_ms, Some(4_000));
    assert_eq!(problem.source.as_deref(), Some("exchange"));
    assert_eq!(
        problem
            .details
            .as_ref()
            .and_then(|details| details.get("submitProblem"))
            .and_then(|value| value.get("requestId"))
            .and_then(|value| value.as_str()),
        Some("req-unwind-429")
    );
}

#[test]
fn response_error_keeps_primary_and_unwind_failure() {
    let error = combine_errors(Some("第二腿失败".into()), Some("unwind 失败".into()));

    assert_eq!(error, Some("第二腿失败; unwind 失败".into()));
}
