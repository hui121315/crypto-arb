use super::super::*;
use crate::services::onchain_comparison::replenishment_allocation::test_cost;

fn pending() -> PendingOnchainExecution {
    let mut row = test_checkpoint();
    row.response.replenishment_costs = vec![test_cost()];
    row.build.replenishment_costs = row.response.replenishment_costs.clone();
    row
}

fn approval_pending() -> PendingOnchainExecution {
    let mut row = test_checkpoint();
    let fixture = crate::services::onchain_comparison::approval_allocation::tests::cost();
    crate::services::onchain_comparison::approval_allocation::tests::bind(&mut row, fixture);
    row
}

#[test]
fn approval_allocation_claim_is_atomic_immutable_and_survives_replay() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("execution.jsonl");
    let store = OnchainExecutionRunStore::load_path(Some(path.clone())).store;
    let row = approval_pending();
    store
        .check_approval_available(&row.response.approval_costs)
        .unwrap();
    store.append_pending(&row).unwrap();
    store.append_pending(&row).unwrap();
    let mut done = row.response.clone();
    done.status = OnchainExecutionRunStatus::Failed;
    store.append_run(&done).unwrap();
    let replay = OnchainExecutionRunStore::load_path(Some(path.clone()));
    replay.store.readiness().unwrap();
    assert_eq!(replay.runs[0].approval_costs, done.approval_costs);
    assert_eq!(
        replay
            .store
            .approval_cost_owner(&done.approval_costs[0].run.run_id),
        Some(done.run_id.clone())
    );
    assert!(replay
        .store
        .check_approval_available(&done.approval_costs)
        .is_err());
    let before = std::fs::read(&path).unwrap();
    for fault in 0..4 {
        let mut other = done.clone();
        match fault {
            0 => other.approval_costs.clear(),
            1 => other.run_id = "second".into(),
            2 => {
                other.run_id = "second".into();
                other.approval_costs[0].run.run_id = "alias".into();
            }
            _ => other.approval_costs[0].build_valuation.net_usd_exact = "0".into(),
        }
        assert!(replay.store.append_run(&other).is_err());
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
    replay.store.readiness().unwrap();
}

#[test]
fn approval_allocation_concurrent_claims_have_one_winner_and_write_failure_claims_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("execution.jsonl");
    let store = OnchainExecutionRunStore::load_path(Some(path.clone())).store;
    let row = approval_pending();
    let mut other = row.clone();
    other.response.run_id = "second".into();
    let barrier = std::sync::Barrier::new(2);
    let winners = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            barrier.wait();
            store.append_pending(&row).is_ok()
        });
        let right = scope.spawn(|| {
            barrier.wait();
            store.append_pending(&other).is_ok()
        });
        usize::from(left.join().unwrap()) + usize::from(right.join().unwrap())
    });
    assert_eq!(winners, 1);
    let restored = OnchainExecutionRunStore::load_path(Some(path));
    restored.store.readiness().unwrap();
    assert_eq!(restored.runs.len(), 1);
    let path = dir.path().join("missing.jsonl");
    let store = OnchainExecutionRunStore::load_path(Some(path.clone())).store;
    std::fs::remove_file(&path).unwrap();
    assert!(store.append_pending(&row).is_err());
    assert!(store
        .approval_cost_owner(&row.response.approval_costs[0].run.run_id)
        .is_none());
    assert!(store
        .check_approval_available(&row.response.approval_costs)
        .is_err());
}

#[test]
fn approval_allocation_checkpoint_rejects_mismatch_and_log_tampering() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("execution.jsonl");
    let store = OnchainExecutionRunStore::load_path(Some(path.clone())).store;
    let mut row = approval_pending();
    let mut bad = row.clone();
    bad.build.approval_costs.clear();
    assert!(store.append_pending(&bad).is_err());
    bad = row.clone();
    bad.config.wallet_address = format!("0x{:040x}", 99);
    assert!(store.append_pending(&bad).is_err());
    store.append_pending(&row).unwrap();
    row.response.run_id = "tampered".into();
    append_jsonl(
        &path,
        &LogEntry {
            cross_chain_cost_claim: None,
            schema_version: 1,
            response: Some(row.response),
            checkpoint: None,
        },
    )
    .unwrap();
    let replay = OnchainExecutionRunStore::load_path(Some(path));
    assert!(replay.store.readiness().is_err());
    assert!(replay.pending.is_empty());
}

#[test]
fn replenishment_allocation_claims_are_durable_immutable_and_replay_safe() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("execution.jsonl");
    let store = OnchainExecutionRunStore::load_path(Some(path.clone())).store;
    let row = pending();
    store
        .check_replenishment_available(&row.response.replenishment_costs)
        .unwrap();
    store.append_pending(&row).unwrap();
    store.append_pending(&row).unwrap();
    let mut done = row.response.clone();
    done.status = OnchainExecutionRunStatus::Failed;
    store.append_run(&done).unwrap();
    let restored = OnchainExecutionRunStore::load_path(Some(path.clone()));
    assert!(restored.store.readiness().is_ok());
    assert_eq!(
        restored.runs[0].replenishment_costs,
        row.response.replenishment_costs
    );
    assert!(restored
        .store
        .check_replenishment_available(&row.response.replenishment_costs)
        .is_err());
    let before = std::fs::read(&path).unwrap();
    let mut another = done.clone();
    another.run_id = "another-execution".into();
    assert!(restored.store.append_run(&another).is_err());
    another.replenishment_costs[0].run_id = "alias-same-transfer".into();
    assert!(restored.store.append_run(&another).is_err());
    done.replenishment_costs.clear();
    assert!(restored.store.append_run(&done).is_err());
    assert!(
        restored.store.readiness().is_ok(),
        "business conflict must not poison the journal"
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn replenishment_allocation_concurrent_claims_have_one_winner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("execution.jsonl");
    let store = OnchainExecutionRunStore::load_path(Some(path.clone())).store;
    let row = pending();
    let mut other = row.clone();
    other.response.run_id = "second".into();
    let barrier = std::sync::Barrier::new(2);
    let winners = std::thread::scope(|scope| {
        let left = scope.spawn(|| {
            barrier.wait();
            store.append_pending(&row).is_ok()
        });
        let right = scope.spawn(|| {
            barrier.wait();
            store.append_pending(&other).is_ok()
        });
        usize::from(left.join().unwrap()) + usize::from(right.join().unwrap())
    });
    assert_eq!(winners, 1);
    let replay = OnchainExecutionRunStore::load_path(Some(path));
    assert!(replay.store.readiness().is_ok());
    assert_eq!(replay.runs.len(), 1);
}

#[test]
fn replenishment_allocation_replay_stops_at_conflicting_history_and_missing_write() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("execution.jsonl");
    let store = OnchainExecutionRunStore::load_path(Some(path.clone())).store;
    let row = pending();
    store.append_pending(&row).unwrap();
    let mut other = row.response.clone();
    other.run_id = "tampered".into();
    append_jsonl(
        &path,
        &LogEntry {
            cross_chain_cost_claim: None,
            schema_version: 1,
            response: Some(other),
            checkpoint: None,
        },
    )
    .unwrap();
    let restored = OnchainExecutionRunStore::load_path(Some(path));
    assert!(restored.store.readiness().is_err());
    assert!(restored.pending.is_empty());
    let path = dir.path().join("missing.jsonl");
    let store = OnchainExecutionRunStore::load_path(Some(path.clone())).store;
    std::fs::remove_file(path).unwrap();
    assert!(store.append_pending(&row).is_err());
    assert!(store
        .check_replenishment_available(&row.response.replenishment_costs)
        .is_err());
}
