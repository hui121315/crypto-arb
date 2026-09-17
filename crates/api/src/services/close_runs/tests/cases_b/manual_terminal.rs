use super::super::super::project::record_manual_terminal_evidence_with_persist;
use super::super::super::*;
use super::super::fixtures::*;
use shared_types::ActionRunKind;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

#[tokio::test]
async fn manual_terminal_ack_success_commits_once_with_manual_source() {
    let path = temp_path("manual-terminal");
    let mut config = test_config();
    config.storage.close_run_ledger_path = Some(path.display().to_string());
    let state = AppState::new(config.clone())
        .await
        .unwrap_or_else(|error| panic!("state init failed: {error}"));
    seed_compensation_failed_run(&state);
    let action_run = manual_terminal_action_run(&state);
    let mut request = manual_terminal_request("pos-1", "operator confirmed account is flat");
    request.manual_handling_cost_usd = Some(2.5);
    let store_lines_before = close_run_store_line_count(&path);
    let persist_calls = Arc::new(AtomicUsize::new(0));
    let persisted_events = Arc::new(parking_lot::Mutex::new(Vec::new()));
    let calls = Arc::clone(&persist_calls);
    let events = Arc::clone(&persisted_events);

    let resolved = record_manual_terminal_evidence_with_persist(
        &state,
        "close-1",
        &request,
        &action_run,
        move |event| {
            calls.fetch_add(1, Ordering::AcqRel);
            events.lock().push(event);
            std::future::ready(Ok::<_, String>(trading::SqlLedgerPersistAck::Committed))
        },
    )
    .await
    .unwrap_or_else(|error| panic!("manual terminal record failed: {error}"));

    assert_eq!(persist_calls.load(Ordering::Acquire), 1);
    assert_persisted_manual_event(&resolved, &persisted_events);
    assert_manual_terminal_resolution(&resolved, &action_run);
    assert_eq!(
        close_run_store_line_count(&path),
        store_lines_before + 1,
        "ACK success must commit one close-run snapshot"
    );
    assert_manual_terminal_replay(&state, &config, &action_run).await;
    let _ = std::fs::remove_file(path);
}

fn assert_persisted_manual_event(
    resolved: &CloseRun,
    persisted_events: &parking_lot::Mutex<Vec<trading::SqlRunFinalityLedgerEvent>>,
) {
    let expected_event = trading::SqlRunFinalityLedgerEvent::from_close_run(
        resolved,
        OrderUpdateSource::Manual,
        None,
        None,
        resolved.updated_at_ms,
    )
    .unwrap_or_else(|error| panic!("manual finality event failed: {error}"));
    let persisted = persisted_events.lock();
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0].event_id(), expected_event.event_id());
    assert!(persisted[0]
        .event_id()
        .contains(":manually_resolved:manual:"));
}

fn assert_manual_terminal_resolution(resolved: &CloseRun, action_run: &ActionRun) {
    assert_eq!(resolved.status, CloseRunStatus::ManuallyResolved);
    assert!(resolved.problem.is_none());
    assert_eq!(
        resolved
            .cost_reconciliation
            .as_ref()
            .and_then(|cost| cost.manual_handling_usd),
        Some(2.5)
    );
    assert_eq!(
        resolved.cost_events.last().map(|event| event.source),
        Some(OrderUpdateSource::Manual)
    );
    let plan = resolved
        .unwind_plan
        .as_ref()
        .unwrap_or_else(|| panic!("manual terminal plan missing"));
    assert_eq!(
        plan.status,
        CloseRunUnwindPlanStatus::ManualTerminalRecorded
    );
    assert!(plan.next_actions.is_empty());
    let evidence = plan
        .manual_terminal_evidence
        .as_ref()
        .unwrap_or_else(|| panic!("manual terminal evidence missing"));
    assert_eq!(
        evidence.action_run_id.as_deref(),
        Some(action_run.id.as_str())
    );
    assert_eq!(evidence.actor, "api-token:operator");
    assert_eq!(evidence.reason, "operator confirmed account is flat");
    assert_eq!(evidence.evidence, vec!["ticket-42".to_owned()]);
    assert_eq!(evidence.remaining_positions.len(), 1);
}

async fn assert_manual_terminal_replay(
    state: &AppState,
    config: &common::config::AppConfig,
    action_run: &ActionRun,
) {
    let stored_action = action_runs::get(state, &action_run.id)
        .unwrap_or_else(|| panic!("manual terminal action run missing"));
    assert_eq!(stored_action.status, ActionRunStatus::Succeeded);
    let replayed = action_runs::replay_payload::<CloseRun>(&stored_action)
        .unwrap_or_else(|error| panic!("manual terminal payload missing: {error}"));
    assert_eq!(replayed.status, CloseRunStatus::ManuallyResolved);

    let restored = AppState::new(config.clone())
        .await
        .unwrap_or_else(|error| panic!("state replay failed: {error}"));
    let replayed_run = restored
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("manual terminal close run not replayed"));
    assert_eq!(replayed_run.status, CloseRunStatus::ManuallyResolved);
    assert!(
        replayed_run
            .unwind_plan
            .as_ref()
            .and_then(|plan| plan.manual_terminal_evidence.as_ref())
            .is_some(),
        "manual terminal evidence must survive CloseRunStore replay"
    );
}

#[tokio::test]
async fn manual_terminal_ack_failure_leaves_hot_and_durable_state_unmutated() {
    let path = temp_path("manual-terminal-ack-failure");
    let mut config = test_config();
    config.storage.close_run_ledger_path = Some(path.display().to_string());
    let state = AppState::new(config)
        .await
        .unwrap_or_else(|error| panic!("state init failed: {error}"));
    seed_compensation_failed_run(&state);
    let action_run = manual_terminal_action_run(&state);
    let mut request = manual_terminal_request("pos-1", "operator confirmed account is flat");
    request.manual_handling_cost_usd = Some(2.5);
    let run_before = stored_close_run(&state);
    let action_before = action_runs::get(&state, &action_run.id)
        .unwrap_or_else(|| panic!("manual terminal action run missing"));
    let store_lines_before = close_run_store_line_count(&path);
    let persist_calls = Arc::new(AtomicUsize::new(0));
    let calls = Arc::clone(&persist_calls);

    let result = record_manual_terminal_evidence_with_persist(
        &state,
        "close-1",
        &request,
        &action_run,
        move |_| {
            calls.fetch_add(1, Ordering::AcqRel);
            std::future::ready(Err::<trading::SqlLedgerPersistAck, _>(
                "forced PostgreSQL ACK failure".to_owned(),
            ))
        },
    )
    .await;
    let error = match result {
        Ok(run) => panic!("manual terminal unexpectedly succeeded: {run:?}"),
        Err(error) => error,
    };

    assert_eq!(error.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(error.code(), codes::TRADING_SQL_LEDGER_WRITE_FAILED);
    assert_eq!(persist_calls.load(Ordering::Acquire), 1);
    assert_eq!(stored_close_run(&state), run_before);
    assert_eq!(
        action_runs::get(&state, &action_run.id),
        Some(action_before),
        "ACK failure must not finish the manual action run"
    );
    assert_eq!(close_run_store_line_count(&path), store_lines_before);
    let _ = std::fs::remove_file(path);
}

#[tokio::test]
async fn manual_terminal_rejects_bad_phrase_and_stale_snapshot() {
    let state = test_state().await;
    seed_compensation_failed_run(&state);
    let action_run = action_runs::begin(
        &state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::PortfolioCloseManualTerminal,
            actor: "test".to_owned(),
            target: Some("close-1".to_owned()),
            idempotency_key: None,
            message: "manual terminal accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"));
    let mut request = manual_terminal_request("pos-1", "operator confirmed account is flat");
    request.confirmation_phrase = "wrong".to_owned();

    let bad_phrase = manual_terminal_error(&state, &request, &action_run).await;
    assert_eq!(bad_phrase.status(), StatusCode::BAD_REQUEST);

    let request = manual_terminal_request("stale", "operator confirmed account is flat");
    let stale = manual_terminal_error(&state, &request, &action_run).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(stale.code(), codes::CLOSE_RUN_STALE_SNAPSHOT);
}

async fn manual_terminal_error(
    state: &AppState,
    request: &CloseRunManualTerminalRequest,
    action_run: &ActionRun,
) -> AppError {
    match record_manual_terminal_evidence_with_persist(
        state,
        "close-1",
        request,
        action_run,
        |_| std::future::ready(Ok::<_, String>(trading::SqlLedgerPersistAck::Committed)),
    )
    .await
    {
        Ok(run) => panic!("manual terminal unexpectedly succeeded: {run:?}"),
        Err(error) => error,
    }
}

fn manual_terminal_action_run(state: &AppState) -> ActionRun {
    action_runs::begin(
        state,
        action_runs::ActionRunStart {
            kind: ActionRunKind::PortfolioCloseManualTerminal,
            actor: "api-token:operator".to_owned(),
            target: Some("close-1".to_owned()),
            idempotency_key: None,
            message: "manual terminal accepted".to_owned(),
        },
    )
    .unwrap_or_else(|error| panic!("action run begin failed: {error}"))
}

fn stored_close_run(state: &AppState) -> CloseRun {
    state
        .close_runs()
        .get("close-1")
        .map(|entry| entry.value().clone())
        .unwrap_or_else(|| panic!("close run missing"))
}

fn close_run_store_line_count(path: &std::path::Path) -> usize {
    match std::fs::read_to_string(path) {
        Ok(contents) => contents.lines().count(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => panic!("close-run store read failed: {error}"),
    }
}

fn seed_compensation_failed_run(state: &AppState) {
    let mut run = close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted));
    run.legs
        .push(close_leg("order-2", CloseLegStatus::Submitted));
    run.expected_leg_count = 2;
    run.submitted_order_count = 2;
    record(state, run);
    let _ = project_ledger_event_update(state, &ledger_fill_event("order-1", 1.0));
    let _ = project_ledger_event_update(
        state,
        &ledger_state_event("order-2", LiveOrderState::Cancelled),
    );
    let compensation = compensation_order_record("comp-order-5", LiveOrderState::Submitted);
    record_compensation_order_for_candidate(state, "close-1", 0, &compensation, None)
        .unwrap_or_else(|error| panic!("record compensation order failed: {error}"));
    let failed = project_ledger_event_update(
        state,
        &ledger_state_event(&compensation.intent.id, LiveOrderState::Cancelled),
    );
    assert_eq!(failed[0].status, CloseRunStatus::CompensationFailed);
}

fn manual_terminal_request(snapshot_version: &str, reason: &str) -> CloseRunManualTerminalRequest {
    CloseRunManualTerminalRequest {
        confirmation_phrase: CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE.to_owned(),
        snapshot_version: Some(snapshot_version.to_owned()),
        reason: reason.to_owned(),
        manual_handling_cost_usd: None,
        evidence: vec!["ticket-42".to_owned(), " ".to_owned()],
    }
}
