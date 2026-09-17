use super::*;
use shared_types::OnchainChainSettlementStatus as ReceiptStatus;

pub(crate) fn pending_bridge(path: &Path) -> (OnchainCrossChainRunStore, OnchainCrossChainRun) {
    let store = OnchainCrossChainRunStore::load_path(Some(path.into()), 100);
    let template = super::super::accounting::tests::fixture();
    store.insert_build(template.build.clone(), 100).unwrap();
    let id = store
        .authorize(&template.build.build_id, "wait-test", "tester", 110)
        .unwrap()
        .run
        .run_id;
    for leg in template.legs.iter().take(2) {
        store
            .claim_leg(
                &id,
                "tester",
                leg.position,
                leg.submitted_input_amount_raw.clone().unwrap(),
                leg.minimum_output_amount_raw.clone().unwrap(),
                format!("provider-{}", leg.position),
                leg.swap_execution.clone(),
                leg.bridge_execution.clone(),
                100,
                10000,
                "104000000".into(),
                "100000000".into(),
                "300".into(),
                1500,
            )
            .unwrap();
        store
            .record_submission_intent(
                &id,
                leg.source_transaction_id.clone().unwrap(),
                "fixture".into(),
                2000,
            )
            .unwrap();
        store
            .record_wallet_receipt(
                &id,
                leg.position,
                leg.source_receipt.clone().unwrap(),
                false,
                None,
                2001,
            )
            .unwrap();
    }
    let run = store.run(&id, 2001).unwrap();
    assert_eq!(
        run.status,
        OnchainCrossChainRunStatus::AwaitingDestinationEvidence
    );
    (store, run)
}

#[test]
fn cross_chain_verification_bounds_pending_and_errors_without_erasing_assets() {
    for problem in [
        "LI.FI PENDING",
        "LI.FI DONE but amount missing",
        "HTTP timeout",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let (store, run) = pending_bridge(&dir.path().join("runs.jsonl"));
        let deadline = run.automatic_check_deadline_ms().unwrap();
        assert_eq!(deadline, 2000 + 2 * 60 * 60_000);
        let claimed = store
            .claim_reconciliation(&run.run_id, deadline - 1)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.legs[1].recovery_checks, 0);
        store
            .record_check_problem(&run.run_id, problem.into(), "fixture".into(), deadline)
            .unwrap();
        let paused = store.finish_reconciliation(&run.run_id, deadline).unwrap();
        assert_eq!(paused.status, OnchainCrossChainRunStatus::Paused);
        assert_eq!(paused.legs[1].source_receipt, run.legs[1].source_receipt);
        assert_eq!(
            paused.legs[1].actual_input_amount_raw,
            run.legs[1].actual_input_amount_raw
        );
        assert_eq!(
            paused.accounting.as_ref().unwrap().flows,
            run.accounting.as_ref().unwrap().flows
        );
        assert!(store
            .claim_reconciliation(&run.run_id, deadline + 60_000)
            .unwrap()
            .is_none());
    }
}

#[test]
fn cross_chain_verification_manual_rounds_survive_restart_and_old_deadlines() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let (store, run) = pending_bridge(&path);
    let start = run.automatic_check_deadline_ms().unwrap() + 24 * 60 * 60_000;
    store.claim_reconciliation(&run.run_id, start).unwrap();
    let restored = store
        .request_recheck(&run.run_id, "tester", 2, start + 1)
        .unwrap();
    assert_eq!(
        restored.status,
        OnchainCrossChainRunStatus::AwaitingDestinationEvidence
    );
    assert_eq!(
        restored.legs[1].status,
        OnchainCrossChainLegRunStatus::SourceConfirmed
    );
    assert!(restored.automatic_check_deadline_ms().is_none());
    for round in 1..=12 {
        let now = start + 1 + i64::from(round - 1) * 60_000;
        let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), now);
        let claimed = store
            .claim_reconciliation(&run.run_id, now)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.legs[1].recovery_checks, round);
        // A repeated request cannot reset a running recovery round.
        assert_eq!(
            store
                .request_recheck(&run.run_id, "tester", 2, now)
                .unwrap(),
            claimed
        );
        assert!(store
            .claim_reconciliation(&run.run_id, now + 59_999)
            .unwrap()
            .is_none());
        store
            .record_check_problem(
                &run.run_id,
                "provider unavailable".into(),
                "fixture".into(),
                now,
            )
            .unwrap();
        let updated = store.finish_reconciliation(&run.run_id, now).unwrap();
        assert_eq!(
            updated.status,
            if round == 12 {
                OnchainCrossChainRunStatus::Paused
            } else {
                OnchainCrossChainRunStatus::AwaitingDestinationEvidence
            }
        );
        assert_eq!(updated.legs[1].attempts, 1);
        assert_eq!(
            updated.legs[1].source_transaction_id,
            run.legs[1].source_transaction_id
        );
        assert_eq!(
            updated.legs[1].actual_input_amount_raw,
            run.legs[1].actual_input_amount_raw
        );
    }
    let store = OnchainCrossChainRunStore::load_path(Some(path), start + 1_000_000);
    let final_run = store.run(&run.run_id, start + 1_000_000).unwrap();
    assert_eq!(final_run.status, OnchainCrossChainRunStatus::Paused);
    assert_eq!(final_run.legs[1].recovery_checks, 12);
    assert!(store
        .claim_reconciliation(&run.run_id, start + 1_000_000)
        .unwrap()
        .is_none());
}

#[test]
fn cross_chain_verification_last_round_success_keeps_receipts_and_stops_at_next_step() {
    let dir = tempfile::tempdir().unwrap();
    let (store, run) = pending_bridge(&dir.path().join("runs.jsonl"));
    store
        .pause(&run.run_id, "operator review".into(), 3000)
        .unwrap();
    store
        .request_recheck(&run.run_id, "tester", 2, 4000)
        .unwrap();
    for round in 0..12 {
        store
            .claim_reconciliation(&run.run_id, 4000 + round * 60_000)
            .unwrap()
            .unwrap();
    }
    let template = super::super::accounting::tests::fixture();
    let now = 4000 + 11 * 60_000;
    let receipt = template.legs[1].destination_receipt.clone().unwrap();
    store
        .record_wallet_receipt(&run.run_id, 2, receipt.clone(), true, Some("99000000"), now)
        .unwrap();
    let completed = store.finish_reconciliation(&run.run_id, now).unwrap();
    assert_eq!(completed.status, OnchainCrossChainRunStatus::Running);
    assert_eq!(completed.active_position, None);
    assert_eq!(
        completed.legs[1].status,
        OnchainCrossChainLegRunStatus::Completed
    );
    assert_eq!(completed.legs[1].destination_receipt, Some(receipt));
    assert_eq!(
        completed.legs[1].actual_output_amount_raw.as_deref(),
        Some("99000000")
    );
    assert_eq!(completed.legs[2].attempts, 0);
    assert_eq!(
        completed.legs[2].status,
        OnchainCrossChainLegRunStatus::RequoteRequired
    );
}

#[test]
fn cross_chain_verification_claim_is_durable_even_if_query_does_not_return() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let (store, run) = pending_bridge(&path);
    store.pause(&run.run_id, "review".into(), 3000).unwrap();
    store
        .request_recheck(&run.run_id, "tester", 2, 4000)
        .unwrap();
    for round in 0..12 {
        let store = OnchainCrossChainRunStore::load_path(Some(path.clone()), 4000 + round * 60_000);
        let claimed = store
            .claim_reconciliation(&run.run_id, 4000 + round * 60_000)
            .unwrap()
            .unwrap();
        assert_eq!(claimed.legs[1].recovery_checks, (round + 1) as u8);
    }
    let store = OnchainCrossChainRunStore::load_path(Some(path), 800_000);
    let paused = store
        .claim_reconciliation(&run.run_id, 800_000)
        .unwrap()
        .unwrap();
    assert_eq!(paused.status, OnchainCrossChainRunStatus::Paused);
    assert_eq!(paused.legs[1].recovery_checks, 12);
}

#[test]
fn cross_chain_verification_partial_receipts_do_not_end_automatic_bridge_window_early() {
    let dir = tempfile::tempdir().unwrap();
    let (store, run) = pending_bridge(&dir.path().join("runs.jsonl"));
    let template = super::super::accounting::tests::fixture();
    let mut receipt = template.legs[1].destination_receipt.clone().unwrap();
    receipt.status = ReceiptStatus::Pending;
    receipt.network_cost = None;
    receipt.problem = Some("RPC fee unavailable".into());
    for round in 0..13 {
        let now = 12_001 + round * 10_000;
        store
            .claim_reconciliation(&run.run_id, now)
            .unwrap()
            .unwrap();
        store
            .record_wallet_receipt(&run.run_id, 2, receipt.clone(), true, Some("99000000"), now)
            .unwrap();
        let waiting = store.finish_reconciliation(&run.run_id, now).unwrap();
        assert_eq!(
            waiting.status,
            OnchainCrossChainRunStatus::AwaitingDestinationEvidence
        );
        assert_eq!(
            waiting.legs[1].actual_output_amount_raw.as_deref(),
            Some("99000000")
        );
    }
    let paused = store
        .claim_reconciliation(&run.run_id, run.automatic_check_deadline_ms().unwrap())
        .unwrap()
        .unwrap();
    assert_eq!(paused.status, OnchainCrossChainRunStatus::Paused);
    assert_eq!(paused.legs[1].destination_receipt, Some(receipt));
}

#[test]
fn cross_chain_verification_claims_are_exclusive_and_legacy_deadline_is_not_invented() {
    let dir = tempfile::tempdir().unwrap();
    let (store, run) = pending_bridge(&dir.path().join("runs.jsonl"));
    let claimed = std::thread::scope(|scope| {
        let jobs: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    store
                        .claim_reconciliation(&run.run_id, 20_000)
                        .unwrap()
                        .is_some()
                })
            })
            .collect();
        jobs.into_iter()
            .map(|job| job.join().unwrap())
            .filter(|claimed| *claimed)
            .count()
    });
    assert_eq!(claimed, 1);
    assert!(store
        .claim_reconciliation(&run.run_id, 20_000)
        .unwrap()
        .is_none());
    let mut legacy = run;
    legacy.legs[1].source_submitted_at_ms = None;
    assert!(legacy.automatic_check_deadline_ms().is_none());
    assert!(stop_reason(&legacy, 30_000)
        .unwrap()
        .contains("提交时间缺失"));
}
