use super::super::super::*;
use super::super::fixtures::*;
use shared_types::ActionRunKind;

#[tokio::test]
async fn auto_compensation_worker_keeps_multi_candidate_runs_manual() {
    let state = test_state().await;
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-3", CloseLegStatus::Submitted));
    run.expected_leg_count = 3;
    run.submitted_order_count = 3;
    record(&state, run);
    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-1", 0.4));
    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-2", 0.3));
    let _ = project_ledger_event_update(
        &state,
        &ledger_state_event("order-3", LiveOrderState::Cancelled),
    );

    let outcome = auto_submit_compensation_once(&state).await;

    assert_eq!(outcome.scanned_run_count, 1);
    assert_eq!(outcome.eligible_run_count, 1);
    assert_eq!(outcome.submitted_count, 0);
    assert_eq!(outcome.skipped_multi_candidate_count, 1);
    let updated = state
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("close run missing"));
    assert_eq!(updated.status, CloseRunStatus::UnwindRequired);
    assert_eq!(
        updated
            .unwind_plan
            .as_ref()
            .map(|plan| plan.compensation_attempts.len()),
        Some(0)
    );
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::PortfolioCloseCompensation)
            .count(),
        0
    );
}

#[tokio::test]
async fn auto_compensation_worker_skips_live_candidates() {
    let state = test_state().await;
    state
        .trading_service()
        .select_binance_live_adapter("key".to_owned(), "secret".to_owned())
        .unwrap_or_else(|error| panic!("live adapter select failed: {error}"));
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    record(&state, run);
    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-1", 0.4));
    let _ = project_ledger_event_update(
        &state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );

    let outcome = auto_submit_compensation_once(&state).await;

    assert_eq!(outcome.scanned_run_count, 1);
    assert_eq!(outcome.eligible_run_count, 1);
    assert_eq!(outcome.submitted_count, 0);
    assert_eq!(outcome.skipped_live_count, 1);
    let updated = state
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("close run missing"));
    assert_eq!(updated.status, CloseRunStatus::UnwindRequired);
    assert_eq!(
        action_runs::recent(&state)
            .into_iter()
            .filter(|run| run.kind == ActionRunKind::PortfolioCloseCompensation)
            .count(),
        0
    );
}

#[tokio::test]
async fn partial_filled_leg_plus_cancelled_leg_unwind_plan_uses_confirmed_quantity() {
    let state = test_state().await;
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    record(&state, run);

    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-1", 0.4));
    let updated = project_ledger_event_update(
        &state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );

    assert_eq!(updated[0].status, CloseRunStatus::UnwindRequired);
    let plan = updated[0]
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("partial unwind plan missing"));
    assert_eq!(plan.compensation_candidates.len(), 1);
    assert_eq!(
        plan.compensation_candidates[0].confirmed_quantity,
        Some(0.4)
    );
    assert_eq!(plan.compensation_candidates[0].confirmed_price, Some(100.0));
    assert_eq!(plan.compensation_candidates[0].notional_usd, 40.0);
    assert_eq!(
        plan.compensation_candidates[0].notional_quality,
        ExecutionLedgerQuality::Actual
    );
    assert_eq!(
        plan.compensation_candidates[0].notional_source,
        "filled_quantity_x_filled_price"
    );
    assert!(plan.compensation_candidates[0]
        .notional_missing_fields
        .is_empty());
    assert_eq!(plan.remaining_positions.len(), 1);
    assert_eq!(
        plan.remaining_positions[0].status,
        CloseLegStatus::Cancelled
    );
    assert_eq!(plan.next_actions.len(), 1);
    assert_eq!(
        plan.next_actions[0].kind,
        CloseRunNextActionKind::SubmitCompensationOrder
    );
}

#[tokio::test]
async fn unwind_plan_marks_mark_price_notional_estimated_when_fill_price_missing() {
    let state = test_state().await;
    let mut filled_leg = close_leg("order-1", CloseLegStatus::PartiallyFilled);
    if let Some(order) = filled_leg.order.as_mut() {
        order.state = LiveOrderState::Filled;
        order.filled_quantity = Some(0.4);
        order.filled_price = None;
    }
    let mut run = close_run("close-1", filled_leg);
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    record(&state, run);

    let updated = project_ledger_event_update(
        &state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );

    assert_eq!(updated[0].status, CloseRunStatus::UnwindRequired);
    let plan = updated[0]
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("partial unwind plan missing"));
    let candidate = &plan.compensation_candidates[0];
    assert_eq!(candidate.confirmed_quantity, Some(0.4));
    assert_eq!(candidate.confirmed_price, None);
    assert_eq!(candidate.notional_usd, 40.0);
    assert_eq!(
        candidate.notional_quality,
        ExecutionLedgerQuality::Estimated
    );
    assert_eq!(candidate.notional_source, "filled_quantity_x_mark_price");
    assert_eq!(candidate.notional_missing_fields, vec!["filled_price"]);
}

#[tokio::test]
async fn ledger_order_state_filled_without_fill_evidence_stays_pending() {
    let state = test_state().await;
    record(
        &state,
        close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted)),
    );

    let updated = project_ledger_event_update(
        &state,
        &ledger_state_event("order-1", LiveOrderState::Filled),
    );

    assert_eq!(updated[0].status, CloseRunStatus::Submitted);
    assert_eq!(updated[0].legs[0].status, CloseLegStatus::Accepted);
    assert_eq!(
        updated[0].legs[0]
            .problem
            .as_ref()
            .map(|problem| problem.source.as_deref()),
        Some(Some("portfolio.close_run.finality"))
    );
    assert!(updated[0].unwind_plan.is_none());
}

#[tokio::test]
async fn close_run_store_replays_submitted_snapshot_after_restart() {
    let path = temp_path("restart");
    let mut config = test_config();
    config.storage.close_run_ledger_path = Some(path.display().to_string());
    let state = AppState::new(config.clone())
        .await
        .unwrap_or_else(|error| panic!("state init failed: {error}"));
    record(
        &state,
        close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted)),
    );

    let restored = AppState::new(config)
        .await
        .unwrap_or_else(|error| panic!("state replay failed: {error}"));

    let run = restored
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone());
    assert_eq!(
        run.as_ref().map(|run| run.status),
        Some(CloseRunStatus::Submitted)
    );
    assert_eq!(
        run.as_ref().map(|run| run.legs.len()),
        Some(1),
        "replayed close run must keep submitted leg artifact"
    );
    let _ = std::fs::remove_file(path);
}
