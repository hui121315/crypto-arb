#![allow(clippy::panic)]
use super::super::*;
use super::fixtures::*;

#[tokio::test]
async fn projects_filled_order_into_close_run_success() {
    let state = test_state().await;
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.action_run_id = Some("act-1".to_owned());
    record(&state, run);

    let mut record = order_record("order-1", LiveOrderState::Filled);
    record.filled_quantity = Some(1.0);
    record.filled_price = Some(100.0);
    record.last_update_source = OrderUpdateSource::PrivateWs;

    let updated = project_order_update(&state, &record);

    assert_eq!(updated.len(), 1);
    let run = &updated[0];
    assert_eq!(run.status, CloseRunStatus::Succeeded);
    assert_eq!(run.legs[0].status, CloseLegStatus::Filled);
    assert_eq!(
        run.legs[0].finality_source,
        Some(OrderUpdateSource::PrivateWs)
    );
    assert_eq!(run.legs[0].confirmed_filled_at_ms, None);
}

#[test]
fn exchange_order_id_collision_cannot_cross_close_run_identity() {
    let mut original = order_record("original", LiveOrderState::Submitted);
    original.exchange_order_id = Some("reused-exchange-id".into());
    let mut newer = order_record("newer", LiveOrderState::Filled);
    newer.exchange_order_id = Some("reused-exchange-id".into());

    assert!(!order_matches_record(&original, &newer));

    let mut ledger = ledger_fill_event("newer", 1.0);
    ledger.order.identity.exchange_order_id = Some("reused-exchange-id".into());
    assert!(!order_matches_ledger_event(&original, &ledger));
}

#[tokio::test]
async fn keeps_filled_without_fill_evidence_pending_with_problem() {
    let state = test_state().await;
    record(
        &state,
        close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted)),
    );

    let updated = project_order_update(&state, &order_record("order-1", LiveOrderState::Filled));

    assert_eq!(updated[0].status, CloseRunStatus::Submitted);
    assert_eq!(updated[0].legs[0].status, CloseLegStatus::Accepted);
    assert_eq!(
        updated[0].legs[0]
            .problem
            .as_ref()
            .map(|problem| problem.source.as_deref()),
        Some(Some("portfolio.close_run.finality"))
    );
}

#[tokio::test]
async fn projects_cancelled_order_into_close_run_failure() {
    let state = test_state().await;
    record(
        &state,
        close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted)),
    );

    let updated = project_order_update(&state, &order_record("order-1", LiveOrderState::Cancelled));

    assert_eq!(updated[0].status, CloseRunStatus::Failed);
    assert_eq!(updated[0].legs[0].status, CloseLegStatus::Cancelled);
    assert_eq!(updated[0].naked_exposure_usd, 100.0);
    assert!(updated[0].problem.is_some());
}

#[tokio::test]
async fn record_returns_terminal_projection_for_a_reused_order() {
    let state = test_state().await;
    let mut leg = close_leg("order-1", CloseLegStatus::Submitted);
    leg.order = Some(order_record("order-1", LiveOrderState::Failed));

    let projected = record(&state, close_run("close-1", leg));

    assert_eq!(projected.status, CloseRunStatus::Failed);
    assert_eq!(projected.legs[0].status, CloseLegStatus::Failed);
    assert_eq!(projected.naked_exposure_usd, 100.0);
    assert!(projected.problem.is_some());
}

#[tokio::test]
async fn finality_problem_updates_matching_close_run_order() {
    let state = test_state().await;
    record(
        &state,
        close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted)),
    );
    let problem = ApiProblem::new(codes::HEDGE_ORDER_FINALITY_FAILED, "order query failed");

    let updated = project_finality_problem(&state, "order-1", &problem, 42);

    assert_eq!(updated.len(), 1);
    assert_eq!(
        updated[0]
            .finality_problem
            .as_ref()
            .map(|problem| problem.code.as_str()),
        Some(codes::HEDGE_ORDER_FINALITY_FAILED)
    );
    assert_eq!(updated[0].finality_checked_at_ms, Some(42));
    let stored = state
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("close run missing"));
    assert!(stored.finality_problem.is_some());
}

#[tokio::test]
async fn order_query_success_clears_close_run_finality_problem() {
    let state = test_state().await;
    record(
        &state,
        close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted)),
    );
    let problem = ApiProblem::new(codes::HEDGE_ORDER_FINALITY_FAILED, "order query failed");
    let _ = project_finality_problem(&state, "order-1", &problem, 42);
    let mut record = order_record("order-1", LiveOrderState::Accepted);
    record.last_update_source = OrderUpdateSource::OrderQuery;
    record.updated_at_ms = 50;

    let updated = project_order_update(&state, &record);

    assert_eq!(updated.len(), 1);
    assert!(updated[0].finality_problem.is_none());
    assert_eq!(updated[0].finality_checked_at_ms, Some(50));
}

#[tokio::test]
async fn ledger_fill_event_projects_close_run_to_success() {
    let state = test_state().await;
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.action_run_id = Some("act-1".to_owned());
    record(&state, run);

    let updated = project_ledger_event_update(&state, &ledger_fill_event("order-1", 1.0));

    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].status, CloseRunStatus::Succeeded);
    assert_eq!(updated[0].legs[0].status, CloseLegStatus::Filled);
    assert_eq!(updated[0].legs[0].confirmed_filled_at_ms, Some(10));
    assert_eq!(
        updated[0].legs[0]
            .order
            .as_ref()
            .and_then(|order| order.filled_price),
        Some(100.0)
    );
}
