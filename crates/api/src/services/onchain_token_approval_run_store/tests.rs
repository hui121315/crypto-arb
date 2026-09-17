use super::*;
use serde_json::json;

pub(super) fn plan() -> Plan {
    let wallet = format!("0x{:040x}", 1);
    let token = format!("0x{:040x}", 2);
    let spender = format!("0x{:040x}", 3);
    serde_json::from_value(json!({
        "approvalId":"approval-one", "direction":"buy_onchain_sell_cex", "provider":"zeroex_swap_v2", "chain":"ethereum",
        "walletAddress":wallet,"tokenAddress":token,"tokenSymbol":"USDC","tokenDecimals":6,"spender":spender,
        "requiredAmountRaw":"1000000","currentAllowanceRaw":"0",
        "transactions":[{"kind":"evm_call","chain_id":1,"from":wallet,"to":token,"value":"0","gas":"0x10000",
            "data":format!("0x095ea7b3{:064x}{:064x}",3,1000000)}],
        "builtAtMs":1000,"validUntilMs":10000,"officialDocsUrl":"https://eips.ethereum.org/EIPS/eip-20",
        "approvalRequired":true,"submitReady":true,"blockers":[]
    })).unwrap()
}

fn response() -> Response {
    serde_json::from_value(
        json!({"runId":"run-one","approvalId":"approval-one","status":"awaiting_finality",
        "transactionIds":[],"message":"fixture","startedAtMs":1100,"updatedAtMs":1100}),
    )
    .unwrap()
}

fn seeded(path: PathBuf) -> OnchainTokenApprovalRunStore {
    let store = OnchainTokenApprovalRunStore::load_path(Some(path));
    store.create(plan(), response()).unwrap();
    store
}

#[test]
fn wallet_claims_approval_unconfirmed_hash_survives_restart_and_failure_label() {
    use crate::services::{
        onchain_execution_run_store::{test_checkpoint, OnchainExecutionRunStore},
        onchain_wallet_claims::WalletClaims,
    };
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("approval.jsonl");
    let shared = Arc::new(WalletClaims::default());
    let store = OnchainTokenApprovalRunStore::load_path(Some(path.clone()))
        .with_wallet_claims(shared.clone());
    let plan = plan();
    store.create(plan.clone(), response()).unwrap();
    let hash = format!("0x{:064x}", 10);
    store.intent("run-one", 0, &hash, 1200).unwrap();
    let mut failed = store.by_approval("approval-one").unwrap();
    failed.status = Status::Failed;
    failed.updated_at_ms = 1250;
    store.response(failed).unwrap();
    assert!(shared.check(&plan.chain, &plan.wallet_address, 50_000).is_err());
    drop(store);
    drop(shared);

    let shared = Arc::new(WalletClaims::default());
    let restored = OnchainTokenApprovalRunStore::load_path(Some(path))
        .with_wallet_claims(shared.clone());
    let mut config = AppConfig::default();
    config.storage.onchain_execution_run_ledger_path =
        Some(dir.path().join("execution.jsonl").to_string_lossy().into());
    let execution = OnchainExecutionRunStore::load(&config)
        .store
        .with_wallet_claims(shared.clone());
    let mut checkpoint = test_checkpoint();
    checkpoint.build.chain = plan.chain;
    checkpoint.build.wallet_address = plan.wallet_address;
    assert!(execution
        .append_pending(&checkpoint)
        .unwrap_err()
        .contains("代币授权"));
    restored.confirm("run-one", &hash).unwrap();
    let mut done = restored.by_approval("approval-one").unwrap();
    done.status = Status::Completed;
    done.updated_at_ms = 1400;
    restored.response(done).unwrap();
    execution.append_pending(&checkpoint).unwrap();
}

#[test]
fn approval_journal_intent_is_durable_before_broadcast_and_replay_never_releases_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let store = seeded(path.clone());
    let hash = format!("0x{:064x}", 10);
    store.intent("run-one", 0, &hash, 1200).unwrap();
    assert!(store.intent("run-one", 0, &hash, 1201).is_err());
    assert!(store
        .intent("run-one", 1, &format!("0x{:064x}", 11), 1201)
        .is_err());
    let before = std::fs::read(&path).unwrap();
    let restored = OnchainTokenApprovalRunStore::load_path(Some(path.clone()));
    let row = restored.by_approval("approval-one").unwrap();
    assert_eq!(row.status, Status::FinalityUnresolved);
    assert_eq!(row.transaction_ids, vec![hash]);
    assert_eq!(
        restored.create(plan(), response()).unwrap().run_id,
        "run-one"
    );
    for _ in 0..3 {
        restored.recent(64);
        restored.record("run-one");
    }
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn approval_journal_write_failure_and_corruption_block_new_intents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let store = seeded(path.clone());
    let before = store.record("run-one").unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(store
        .intent("run-one", 0, &format!("0x{:064x}", 10), 1200)
        .is_err());
    assert_eq!(store.record("run-one").unwrap(), before);
    let visible = store.recent(1).pop().unwrap();
    assert!(visible.problem.is_some());
    assert!(!visible.receipt_check_pending());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, b"{\"partial\":").unwrap();
    let corrupted = OnchainTokenApprovalRunStore::load_path(Some(path));
    assert!(corrupted.readiness().is_err());
    assert!(corrupted.create(plan(), response()).is_err());
}

#[test]
fn approval_journal_missing_receipts_are_bounded_without_forgetting_hashes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let store = seeded(path.clone());
    let hash = format!("0x{:064x}", 10);
    store.intent("run-one", 0, &hash, 1200).unwrap();
    let row = store.record("run-one").unwrap();
    let receipt = OnchainWalletReceipt {
        basis: basis(&row, &hash).unwrap(),
        status: ReceiptStatus::Pending,
        asset_changes_raw: vec![None],
        additional_native_change_raw: None,
        network_cost: None,
        block_ref: None,
        observed_at_ms: None,
        problem: Some("rpc timeout".into()),
    };
    for count in 1..=12 {
        store
            .receipt("run-one", receipt.clone(), count * 5000)
            .unwrap();
    }
    assert!(store.due(100000).is_empty());
    assert!(
        store
            .record("run-one")
            .unwrap()
            .response
            .fee_checks_exhausted
    );
    let replay = OnchainTokenApprovalRunStore::load_path(Some(path));
    assert!(replay.due(100000).is_empty());
    assert_eq!(
        replay.record("run-one").unwrap().response.transaction_ids,
        vec![hash]
    );
    let before = replay.record("run-one").unwrap().response.transaction_ids;
    replay.resume_checks("run-one").unwrap();
    assert_eq!(replay.due(100000).len(), 1);
    assert_eq!(
        replay.record("run-one").unwrap().response.transaction_ids,
        before
    );
    assert!(replay.intent("run-one", 0, &before[0], 100000).is_err());
}

#[test]
fn approval_journal_cannot_claim_success_or_change_plan_without_confirmation() {
    let dir = tempfile::tempdir().unwrap();
    let store = seeded(dir.path().join("runs.jsonl"));
    let mut run = response();
    run.status = Status::Completed;
    assert!(store.response(run).is_err());
    let mut wrong = plan();
    if let shared_types::OnchainUnsignedTransaction::EvmCall { to, .. } = &mut wrong.transactions[0]
    {
        *to = format!("0x{:040x}", 55);
    }
    let other = OnchainTokenApprovalRunStore::load_path(Some(dir.path().join("wrong.jsonl")));
    assert!(other.create(wrong, response()).is_err());
}
