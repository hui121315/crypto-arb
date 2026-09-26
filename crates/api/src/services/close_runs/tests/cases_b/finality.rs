use super::super::super::*;
use super::super::fixtures::*;
use shared_types::ActionRunKind;

#[tokio::test]
async fn compensation_cancel_retains_partial_fills_and_only_zero_fill_allows_full_retry() {
    for filled in [
        None,
        Some(0.0),
        Some(0.4),
        Some(1.0),
        Some(f64::NAN),
        Some(-1.0),
    ] {
        let state = test_state().await;
        let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
        run.legs
            .push(close_leg("order-2", CloseLegStatus::Submitted));
        run.expected_leg_count = 2;
        record(&state, run);
        project_ledger_event_update(&state, &ledger_fill_event("order-1", 1.0));
        project_ledger_event_update(
            &state,
            &ledger_state_event("order-2", LiveOrderState::Cancelled),
        );
        let mut order = compensation_order_record("close-1", LiveOrderState::PartiallyFilled);
        order.filled_quantity = filled;
        let run =
            record_compensation_order_for_candidate(&state, "close-1", 0, &order, None).unwrap();
        let plan = run.unwind_plan.unwrap();
        assert_eq!(
            plan.compensation_attempts[0].cancellable_order_id(),
            Some(order.intent.id.as_str())
        );
        assert!(plan
            .next_actions
            .iter()
            .any(|action| action.kind == CloseRunNextActionKind::CancelCompensationOrder));

        order.state = LiveOrderState::CancelRequested;
        let pending = project_order_update(&state, &order).remove(0);
        assert!(pending.unwind_plan.unwrap().compensation_attempts[0]
            .cancellable_order_id()
            .is_none());
        order.state = LiveOrderState::Cancelled;
        let terminal = project_order_update(&state, &order).remove(0);
        let plan = terminal.unwind_plan.as_ref().unwrap();
        assert_eq!(terminal.status, CloseRunStatus::CompensationFailed);
        assert_eq!(
            plan.compensation_attempts[0].confirmed_filled_quantity(),
            filled.filter(|qty| qty.is_finite() && *qty >= 0.0)
        );
        let retry = filled == Some(0.0);
        assert_eq!(
            plan.next_actions
                .iter()
                .any(|action| action.kind == CloseRunNextActionKind::SubmitCompensationOrder),
            retry
        );
        assert_eq!(validate_compensation_state(&terminal).is_ok(), retry);
        assert_eq!(
            validate_compensation_candidate_state(&terminal, 0).is_ok(),
            retry
        );
        assert!(plan.compensation_attempts[0]
            .cancellable_order_id()
            .is_none());
    }
}

#[tokio::test]
async fn unwind_required_refreshes_action_run_payload_with_plan() {
    let state = test_state().await;
    let action_run = action_runs::begin(
        &state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::PortfolioClosePair,
            actor: "test".to_owned(),
            target: Some("binance:MUUSDT".to_owned()),
            idempotency_key: None,
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    run.action_run_id = Some(action_run.id.clone());
    record(&state, run);

    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-1", 1.0));
    let updated = project_ledger_event_update(
        &state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );

    assert_eq!(updated[0].status, CloseRunStatus::UnwindRequired);
    let stored = action_runs::get(&state, &action_run.id)
        .unwrap_or_else(|| panic!("close action run missing"));
    assert_eq!(stored.status, ActionRunStatus::Failed);
    assert_eq!(
        stored.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::CLOSE_RUN_UNWIND_REQUIRED)
    );
    let replayed = action_runs::replay_payload::<CloseRun>(&stored)
        .unwrap_or_else(|error| panic!("close action run payload missing: {error}"));
    assert_eq!(replayed.status, CloseRunStatus::UnwindRequired);
    assert!(replayed.unwind_plan.is_some());
    assert_eq!(
        replayed
            .problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::CLOSE_RUN_UNWIND_REQUIRED)
    );
}

#[tokio::test]
async fn compensation_order_finality_refreshes_close_run_and_action_payload() {
    let state = test_state().await;
    let action_run = action_runs::begin(
        &state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::PortfolioClosePair,
            actor: "test".to_owned(),
            target: Some("binance:MUUSDT".to_owned()),
            idempotency_key: None,
            message: "accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    run.action_run_id = Some(action_run.id.clone());
    record(&state, run);

    let _ = project_ledger_event_update(&state, &ledger_fill_event("order-1", 1.0));
    let _ = project_ledger_event_update(
        &state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );
    let compensation = compensation_order_record("comp-order-3", LiveOrderState::Submitted);
    let submitted = record_compensation_order_for_candidate(
        &state,
        "close-1",
        0,
        &compensation,
        Some(action_run.id.clone()),
    )
    .unwrap_or_else(|error| panic!("record compensation order failed: {error}"));

    assert_eq!(submitted.status, CloseRunStatus::CompensationSubmitted);
    assert_eq!(
        submitted.unwind_plan.as_ref().map(|plan| plan.status),
        Some(CloseRunUnwindPlanStatus::CompensationSubmitted)
    );
    assert_eq!(
        submitted
            .unwind_plan
            .as_ref()
            .map(|plan| plan.compensation_attempts.len()),
        Some(1)
    );
    assert_eq!(
        submitted.unwind_plan.as_ref().map(|plan| {
            plan.next_actions
                .iter()
                .map(|action| action.kind)
                .collect::<Vec<_>>()
        }),
        Some(vec![
            CloseRunNextActionKind::CancelCompensationOrder,
            CloseRunNextActionKind::WaitForCompensationFinality,
        ])
    );
    let stored = action_runs::get(&state, &action_run.id)
        .unwrap_or_else(|| panic!("close action run missing"));
    assert_eq!(stored.status, ActionRunStatus::Accepted);

    let compensated =
        project_ledger_event_update(&state, &ledger_fill_event(&compensation.intent.id, 1.0));

    assert_eq!(compensated[0].status, CloseRunStatus::Compensated);
    assert_eq!(compensated[0].naked_exposure_usd, 0.0);
    assert_eq!(
        compensated[0].unwind_plan.as_ref().map(|plan| plan.status),
        Some(CloseRunUnwindPlanStatus::Compensated)
    );
    assert_eq!(
        compensated[0]
            .unwind_plan
            .as_ref()
            .map(|plan| plan.remaining_positions.len()),
        Some(0)
    );
    assert_eq!(
        compensated[0]
            .unwind_plan
            .as_ref()
            .map(|plan| plan.next_actions.len()),
        Some(0)
    );
    let stored = action_runs::get(&state, &action_run.id)
        .unwrap_or_else(|| panic!("close action run missing after compensation"));
    assert_eq!(stored.status, ActionRunStatus::Succeeded);
    assert!(stored.problem.is_none());
    let replayed = action_runs::replay_payload::<CloseRun>(&stored)
        .unwrap_or_else(|error| panic!("close action run payload missing: {error}"));
    assert_eq!(replayed.status, CloseRunStatus::Compensated);
}
