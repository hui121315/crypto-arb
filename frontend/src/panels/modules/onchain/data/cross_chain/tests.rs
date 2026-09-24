use super::*;
use serde_json::json;

fn run(id: &str, status: RunStatus, updated: i64) -> OnchainCrossChainRun {
    serde_json::from_value(json!({
        "runId": id, "idempotencyKey": format!("key-{id}"), "status": status,
        "authorization": {"actor":"test", "authorizedAtMs":100, "validUntilMs":1000, "confirmationVersion":"v1"},
        "activePosition": null, "createdAtMs":100, "updatedAtMs":updated, "nextAction":"等待下一步", "problem":null,
        "build": {"buildId":"build-1", "provider":"lifi", "sourceChain":"ethereum", "peerChain":"base",
            "legs":[], "initialQuoteAmountRaw":"1000000", "finalQuoteAmountRaw":"1010000",
            "quoteObservedAtMs":100, "builtAtMs":100, "validUntilMs":500,
            "atomic":false, "monitorOnly":false, "previewReady":true, "submitReady":true},
        "legs":[{"position":1,"kind":"source_swap","clientActionId":"leg-1",
            "status":"requote_required","attempts":0,"plannedInputAmountRaw":"1000000"},
            {"position":2,"kind":"outbound_bridge","clientActionId":"leg-2",
            "status":"requote_required","attempts":0,"plannedInputAmountRaw":"1000000"}]
    })).expect("cross-chain run fixture")
}

#[test]
fn cross_chain_recovery_failure_blocks_actions_but_keeps_history_visible() {
    let mut state = CrossChainRecovery::default();
    state.accept_snapshot(vec![run("r1", RunStatus::Running, 200)], 200);
    let request = OnchainCrossChainSubmitRequest { run_id: "r1".into(), expected_position: 1 };
    assert!(state.can_submit(&request, 200));
    state.recovery_problem = Some("恢复日志第 3 行损坏".into());
    assert!(!state.can_submit(&request, 200));
    assert!(!state.can_build(200));
    assert!(!state.can_authorize("new", 200));
    assert_eq!(state.selected().unwrap().run_id, "r1");
    state.accept_snapshot(Vec::new(), 300);
    assert!(!state.can_submit(&request, 300));
    assert!(state.recovery_problem.is_some());
    state.recovery_problem = None;
    state.read_problem = Some("network timeout".into());
    assert!(!state.can_submit(&request, 300));
    state.accept_snapshot(Vec::new(), 300);
    assert!(state.can_submit(&request, 300));
}

#[test]
fn cross_chain_reload_prefers_unresolved_run_over_latest_completed() {
    let mut state = CrossChainRecovery::default();
    assert!(!state.can_build(200));
    state.accept_snapshot(
        vec![
            run("done", RunStatus::Completed, 400),
            run("pending", RunStatus::Paused, 300),
        ],
        500,
    );
    assert_eq!(state.selected().unwrap().run_id, "pending");
    assert!(!state.can_build(500));
}

#[test]
fn cross_chain_timeout_keeps_evidence_and_recovers_without_resubmitting() {
    let mut state = CrossChainRecovery::default();
    state.accept_snapshot(vec![run("r1", RunStatus::Running, 200)], 200);
    state.pending_submission = Some("r1".into());
    state.problem = Some("network timeout".into());
    assert!(state.needs_poll());
    let request = OnchainCrossChainSubmitRequest {
        run_id: "r1".into(),
        expected_position: 1,
    };
    assert!(!state.can_submit(&request, 250));
    state.accept_snapshot(vec![run("r1", RunStatus::Running, 200)], 260);
    assert!(state.pending_submission.is_some());
    assert!(!state.can_submit(&request, 260));
    state.accept_snapshot(vec![run("r1", RunStatus::Running, 199)], 270);
    assert!(state.pending_submission.is_some());
    let mut latest = run("r1", RunStatus::AwaitingDestinationEvidence, 300);
    latest.active_position = Some(1);
    latest.legs[0].source_transaction_id = Some("0xconfirmed".into());
    state.accept_snapshot(vec![latest], 350);
    assert!(state.pending_submission.is_none());
    assert_eq!(
        state.selected().unwrap().legs[0]
            .source_transaction_id
            .as_deref(),
        Some("0xconfirmed")
    );
    assert!(!state.can_submit(&request, 350));
    state.accept_snapshot(vec![run("r1", RunStatus::Running, 200)], 400);
    assert_eq!(
        state.selected().unwrap().status,
        RunStatus::AwaitingDestinationEvidence
    );
}

#[test]
fn cross_chain_old_step_does_not_advance_to_the_next_step() {
    let mut latest = run("r1", RunStatus::Running, 500);
    latest.legs[0].status = LegStatus::Completed;
    latest.legs[0].actual_output_amount_raw = Some("1234500".into());
    let mut state = CrossChainRecovery::default();
    state.accept_snapshot(vec![latest], 500);
    assert!(!state.can_submit(
        &OnchainCrossChainSubmitRequest {
            run_id: "r1".into(),
            expected_position: 1
        },
        500
    ));
    assert!(state.can_submit(
        &OnchainCrossChainSubmitRequest {
            run_id: "r1".into(),
            expected_position: 2
        },
        500
    ));
    assert!(!state.needs_poll());
}

#[test]
fn cross_chain_ambiguous_authorization_can_only_retry_the_same_key() {
    let mut state = CrossChainRecovery {
        loaded: true,
        pending_authorization: Some("key-r1".into()),
        ..Default::default()
    };
    assert!(!state.can_build(200));
    assert!(state.can_authorize("key-r1", 200));
    assert!(!state.can_authorize("different-key", 200));
    state.accept_snapshot(
        vec![run("r1", RunStatus::AuthorizedAwaitingSubmit, 200)],
        300,
    );
    assert!(state.pending_authorization.is_none());
    assert_eq!(
        next_submit_position(state.selected().unwrap(), 999),
        Some(1)
    );
    assert_eq!(next_submit_position(state.selected().unwrap(), 1000), None);
    assert!(state.can_build(1000));
}

#[test]
fn cross_chain_recheck_is_scoped_to_the_paused_transaction_and_never_submits() {
    let mut row = run("paused", RunStatus::Paused, 500);
    row.active_position = Some(1);
    row.legs[0].status = LegStatus::Paused;
    let mut state = CrossChainRecovery::default();
    state.accept_snapshot(vec![row.clone()], 500);
    let request = OnchainCrossChainRecheckRequest {
        run_id: "paused".into(),
        expected_position: 1,
    };
    assert!(!state.can_recheck(&request));
    row.legs[0].source_transaction_id = Some("0xexisting".into());
    state.accept_snapshot(vec![row.clone()], 500);
    assert!(state.can_recheck(&request));
    assert!(!state.can_submit(
        &OnchainCrossChainSubmitRequest {
            run_id: "paused".into(),
            expected_position: 1
        },
        500
    ));
    assert!(!state.can_recheck(&OnchainCrossChainRecheckRequest {
        run_id: "paused".into(),
        expected_position: 2
    }));
    row.status = RunStatus::AwaitingSourceFinality;
    row.legs[0].status = LegStatus::Submitted;
    row.updated_at_ms = 501;
    state.accept_run(row, true);
    assert!(!state.can_recheck(&request));
    assert!(state.needs_poll());
    assert!(!state.can_build(501));
}
