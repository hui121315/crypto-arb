use super::super::super::*;
use super::super::fixtures::*;

#[tokio::test]
async fn auto_compensation_worker_submits_single_candidate_with_action_run() {
    let state = test_state().await;
    seed_single_candidate_unwind(&state);

    let outcome = auto_submit_compensation_once(&state).await;

    assert_eq!(outcome.scanned_run_count, 1);
    assert_eq!(outcome.eligible_run_count, 1);
    assert_eq!(outcome.submitted_count, 1);
    assert_eq!(outcome.submission_failure_count, 0);
    let updated = state
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("close run missing"));
    assert_eq!(updated.status, CloseRunStatus::Compensated);
    let plan = updated
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("auto compensation plan missing"));
    assert_next_action(
        plan,
        CloseRunNextActionKind::WaitForCompensationFinality,
        false,
    );
    let attempt = updated
        .unwind_plan
        .as_ref()
        .and_then(|plan| plan.compensation_attempts.first())
        .unwrap_or_else(|| panic!("auto compensation attempt missing"));
    assert!(attempt.action_run_id.is_some());
    let order = attempt
        .order
        .as_ref()
        .unwrap_or_else(|| panic!("auto compensation order missing"));
    assert_eq!(order.intent.source, OrderSource::CloseRunCompensation);
    assert_eq!(order.intent.quantity, 0.4);
    assert_eq!(attempt.status, CloseLegStatus::Filled);
    assert!(attempt.confirmed_filled_at_ms.is_some_and(|time| time > 0));
    assert_eq!(order.filled_quantity, Some(0.4));
    let action_run = action_runs::recent(&state)
        .into_iter()
        .find(|run| run.kind == ActionRunKind::PortfolioCloseCompensation)
        .unwrap_or_else(|| panic!("auto compensation action run missing"));
    assert_eq!(action_run.status, ActionRunStatus::Succeeded);
    assert_eq!(action_run.actor, "system:close-run-auto-compensation");
    let replayed = action_runs::replay_payload::<CloseRun>(&action_run)
        .unwrap_or_else(|error| panic!("auto compensation payload missing: {error}"));
    assert_eq!(replayed.status, CloseRunStatus::Compensated);
}

#[tokio::test]
async fn compensation_cost_uses_durable_slippage_event_ids() {
    let state = test_state().await;
    seed_single_candidate_unwind(&state);
    let _ = project_ledger_event_update(&state, &ledger_slippage_event("order-1", 0.4));
    let outcome = auto_submit_compensation_once(&state).await;
    assert_eq!(outcome.submitted_count, 1);
    let submitted = close_run_snapshot_for_test(&state, "after compensation submit");
    let attempt = compensation_attempt_at(&submitted, 0, "compensation");
    let compensation_order_id = attempt
        .order
        .as_ref()
        .map(|order| order.intent.id.clone())
        .unwrap_or_else(|| panic!("compensation order id missing"));

    let _ = project_ledger_event_update(&state, &ledger_fill_event(&compensation_order_id, 0.4));
    let updated =
        project_ledger_event_update(&state, &ledger_slippage_event(&compensation_order_id, 0.7));

    let cost = updated[0]
        .cost_reconciliation
        .as_ref()
        .unwrap_or_else(|| panic!("compensation cost missing"));
    assert_eq!(cost.close_slippage_usd, Some(0.4));
    assert_eq!(cost.compensation_fee_usd, Some(0.01));
    assert_eq!(cost.compensation_slippage_usd, Some(0.7));
    assert_eq!(
        cost.compensation_slippage_event_ids,
        vec![format!("slippage:{compensation_order_id}:0.7")]
    );
    assert!(cost.missing_fields.is_empty());

    let duplicate =
        project_ledger_event_update(&state, &ledger_slippage_event(&compensation_order_id, 0.7));
    assert!(duplicate.is_empty());
}

#[tokio::test]
async fn auto_compensation_worker_retries_single_failed_candidate_once() {
    let state = test_state().await;
    seed_single_candidate_unwind(&state);

    let first_outcome = auto_submit_compensation_once(&state).await;

    assert_eq!(first_outcome.submitted_count, 1);
    let submitted = close_run_snapshot_for_test(&state, "after first compensation");
    let first_attempt = compensation_attempt_at(&submitted, 0, "first");
    let first_action_run_id = first_attempt
        .action_run_id
        .clone()
        .unwrap_or_else(|| panic!("first action run id missing"));
    let first_failed_order = failed_order_for_attempt(&first_attempt, "first");

    let failed_runs = project_order_update(&state, &first_failed_order);

    assert_eq!(failed_runs[0].status, CloseRunStatus::CompensationFailed);
    let failed_plan = failed_runs[0]
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("failed compensation plan missing"));
    assert_next_action(
        failed_plan,
        CloseRunNextActionKind::SubmitCompensationOrder,
        true,
    );
    assert_next_action(
        failed_plan,
        CloseRunNextActionKind::ManualIncidentReview,
        true,
    );
    assert_eq!(
        action_run_status(&state, &first_action_run_id),
        ActionRunStatus::Failed
    );

    let retry_outcome = auto_submit_compensation_once(&state).await;

    assert_eq!(retry_outcome.submitted_count, 1);
    let retried = close_run_snapshot_for_test(&state, "after retry");
    assert_eq!(retried.status, CloseRunStatus::Compensated);
    assert_eq!(
        retried.message,
        "平仓事故补偿已完成：1 条补偿订单已确认成交"
    );
    let retry_plan = retried
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("retry compensation plan missing"));
    assert_eq!(retry_plan.compensation_attempts.len(), 2);
    assert_eq!(
        retry_plan.compensation_attempts[0].status,
        CloseLegStatus::Cancelled
    );
    assert_eq!(
        retry_plan.compensation_attempts[1].status,
        CloseLegStatus::Filled
    );
    assert!(retry_plan.compensation_attempts[1]
        .confirmed_filled_at_ms
        .is_some_and(|time| time > 0));
    assert_eq!(
        action_run_status(&state, &first_action_run_id),
        ActionRunStatus::Failed,
        "retry submit must not rewrite the failed attempt ActionRun"
    );
    let retry_action_run_id = retry_plan.compensation_attempts[1]
        .action_run_id
        .as_deref()
        .unwrap_or_else(|| panic!("retry action run id missing"));
    assert_ne!(retry_action_run_id, first_action_run_id);
    assert_eq!(
        action_run_status(&state, retry_action_run_id),
        ActionRunStatus::Succeeded
    );

    let retry_failed_order =
        failed_order_for_attempt(&retry_plan.compensation_attempts[1], "retry");

    let failed_again = project_order_update(&state, &retry_failed_order);

    assert_eq!(failed_again[0].status, CloseRunStatus::CompensationFailed);
    let terminal_plan = failed_again[0]
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("terminal compensation plan missing"));
    assert_eq!(terminal_plan.compensation_attempts.len(), 2);
    assert_next_action(
        terminal_plan,
        CloseRunNextActionKind::SubmitCompensationOrder,
        false,
    );
    assert_next_action(
        terminal_plan,
        CloseRunNextActionKind::ManualIncidentReview,
        true,
    );

    let final_outcome = auto_submit_compensation_once(&state).await;

    assert_eq!(final_outcome.submitted_count, 0);
}

#[tokio::test]
async fn compensation_confirmation_requires_complete_ledger_without_double_counting() {
    let state = test_state().await;
    seed_single_candidate_unwind(&state);
    assert_eq!(
        auto_submit_compensation_once(&state).await.submitted_count,
        1
    );
    let run = close_run_snapshot_for_test(&state, "confirmation");
    let mut original = compensation_attempt_at(&run, 0, "confirmation");
    original.confirmed_filled_at_ms = None;
    original.cost_events.clear();
    let order = original.order.as_mut().unwrap();
    order.intent.mode = ExecutionMode::Live;
    let id = order.intent.id.clone();
    let mut first = ledger_fill_event(&id, 0.1);
    first.order.identity = order.identity_snapshot();
    first.order.side = order.intent.side;
    let mut second = ledger_fill_event(&id, 0.3);
    second.order = first.order.clone();
    second.occurred_at_ms = 12;
    let mut ack = ledger_fill_event(&id, 0.4);
    ack.order = first.order.clone();
    ack.event_type = ExecutionLedgerEventType::FillSnapshot;
    ack.source = OrderUpdateSource::AdapterAck;
    let mut foreign = second.clone();
    foreign.order.exchange = "okx".to_owned();
    for events in [
        vec![],
        vec![ack],
        vec![first.clone()],
        vec![first.clone(), foreign],
    ] {
        let mut attempt = original.clone();
        confirm_compensation_from_ledger(&mut attempt, events);
        assert_eq!(attempt.confirmed_filled_at_ms, None);
        assert!(!compensation_attempt_filled(&attempt));
        assert_eq!(
            compensation_plan_status(&[], &[attempt.clone()]),
            CloseRunUnwindPlanStatus::CompensationSubmitted
        );
        assert_eq!(attempt.order.unwrap().filled_quantity, Some(0.4));
    }
    let events = vec![second, first.clone(), first];
    confirm_compensation_from_ledger(&mut original, events.clone());
    assert_eq!(original.confirmed_filled_at_ms, Some(12));
    assert!(compensation_attempt_filled(&original));
    assert_eq!(original.order.as_ref().unwrap().filled_quantity, Some(0.4));
    assert_eq!(original.cost_events.len(), 2);
    confirm_compensation_from_ledger(&mut original, events);
    assert_eq!(original.order.as_ref().unwrap().filled_quantity, Some(0.4));
    assert_eq!(original.cost_events.len(), 2);
}

fn seed_single_candidate_unwind(state: &AppState) {
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    record(state, run);
    let _ = project_ledger_event_update(state, &ledger_fill_event("order-1", 0.4));
    let _ = project_ledger_event_update(
        state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );
    state.market_data().store_orderbook(
        orderbook("binance", "MUUSDT"),
        crate::services::market_data::MarketSource::WsPush,
    );
}

fn close_run_snapshot_for_test(state: &AppState, label: &str) -> CloseRun {
    state
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("close run missing {label}"))
}

fn compensation_attempt_at(
    run: &CloseRun,
    index: usize,
    label: &str,
) -> CloseRunCompensationAttempt {
    run.unwind_plan
        .as_ref()
        .and_then(|plan| plan.compensation_attempts.get(index))
        .cloned()
        .unwrap_or_else(|| panic!("{label} compensation attempt missing"))
}

fn failed_order_for_attempt(attempt: &CloseRunCompensationAttempt, label: &str) -> OrderRecord {
    let mut order = attempt
        .order
        .clone()
        .unwrap_or_else(|| panic!("{label} compensation order missing"));
    order.state = LiveOrderState::Cancelled;
    order.filled_quantity = Some(0.0);
    order.filled_price = None;
    order.updated_at_ms += 1;
    order
}

fn assert_next_action(plan: &CloseRunUnwindPlan, kind: CloseRunNextActionKind, expected: bool) {
    assert_eq!(
        plan.next_actions.iter().any(|action| action.kind == kind),
        expected
    );
}
