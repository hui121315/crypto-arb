use super::*;
use crate::services::onchain_comparison::replenishment_credit::rpc_tests::{client, fixture};
use crate::services::onchain_token_approval_run_store::{test_plan, OnchainTokenApprovalRunStore};

fn setup(path: std::path::PathBuf) -> (OnchainTokenApprovalRunStore, ApprovalRecord, Value) {
    let store = OnchainTokenApprovalRunStore::load_path(Some(path));
    let response = serde_json::from_value(
        json!({"runId":"run-rpc","approvalId":"approval-one","status":"awaiting_finality",
        "transactionIds":[],"message":"fixture","startedAtMs":1100,"updatedAtMs":1100}),
    )
    .unwrap();
    store.create(test_plan(), response).unwrap();
    let hash = format!("0x{:064x}", 10);
    store.intent("run-rpc", 0, &hash, 1200).unwrap();
    let row = store.record("run-rpc").unwrap();
    let (owner, spender, amount) = store::approval_words(&row.plan, 0).unwrap();
    let tx = json!({"transactionHash":hash,"blockNumber":"0x64","blockHash":"0xcanonical","from":row.plan.wallet_address,
        "to":row.plan.token_address,"status":"0x1","gasUsed":"0x5208","effectiveGasPrice":"0x3b9aca00",
        "logs":[{"address":row.plan.token_address,"topics":[APPROVAL_TOPIC,owner,spender],"data":amount}]});
    (store, row, tx)
}

async fn read(row: &ApprovalRecord, tx: Value) -> OnchainWalletReceipt {
    let hash = &row.response.transaction_ids[0];
    let mut replies = vec![
        ("eth_chainId", json!("0x1")),
        ("eth_getTransactionReceipt", tx.clone()),
        ("eth_getBlockByNumber", json!({"hash":"0xcanonical"})),
        ("eth_blockNumber", json!("0x65")),
    ];
    if tx["status"] == "0x1" {
        replies.extend([
            ("eth_getTransactionByHash",json!({"hash":hash,"blockHash":"0xcanonical","from":row.plan.wallet_address,"to":row.plan.token_address,"value":"0x0","input":"0x095ea7b3"})),
            ("debug_traceTransaction",json!({"type":"CALL","from":row.plan.wallet_address,"to":row.plan.token_address,"value":"0x0","calls":[]})),
            ("eth_getTransactionReceipt",tx),
        ]);
    }
    let rpc = fixture(replies).await;
    let result = read_on_rpc(&client(), &rpc.url, row, hash).await.unwrap();
    assert!(rpc.replies.lock().unwrap().is_empty());
    result
}

#[tokio::test]
async fn approval_receipt_rpc_to_journal_preserves_exact_fee_and_restarts_without_resubmitting() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let (store, row, tx) = setup(path.clone());
    assert_eq!(
        format!(
            "0x{}",
            hex::encode(common::signing::keccak256(
                b"Approval(address,address,uint256)"
            ))
        ),
        APPROVAL_TOPIC
    );
    let receipt = read(&row, tx).await;
    assert_eq!(receipt.status, Status::Complete);
    assert_eq!(
        receipt
            .network_cost
            .as_ref()
            .unwrap()
            .total_fee_exact
            .as_deref(),
        Some("0.000021")
    );
    let recovered = OnchainTokenApprovalRunStore::load_path(Some(path.clone()));
    let result = recovered.receipt("run-rpc", receipt.clone(), 7000).unwrap();
    assert_eq!(
        result.status,
        shared_types::OnchainTokenApprovalRunStatus::Completed
    );
    let before = std::fs::read(&path).unwrap();
    recovered.receipt("run-rpc", receipt, 8000).unwrap();
    assert_eq!(before, std::fs::read(&path).unwrap());
    assert_eq!(
        OnchainTokenApprovalRunStore::load_path(Some(path))
            .record("run-rpc")
            .unwrap()
            .response,
        result
    );
    assert_eq!(
        store
            .record("run-rpc")
            .unwrap()
            .response
            .transaction_ids
            .len(),
        1
    );
    if let Ok(path) = std::env::var("CROSSLINE_APPROVAL_RECEIPT_FIXTURE") {
        std::fs::write(path, serde_json::to_vec(&result).unwrap()).unwrap();
    }
    let cost = crate::services::onchain_comparison::approval_allocation::tests::from_receipt(test_plan(), result);
    assert_eq!(cost.build_valuation.net_usd_exact, "-0.0441");
    let mut execution = crate::services::onchain_execution_run_store::test_checkpoint();
    crate::services::onchain_comparison::approval_allocation::tests::bind(&mut execution, cost);
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_execution_run_ledger_path = Some(dir.path().join("execution.jsonl").to_string_lossy().into());
    let executions = crate::services::onchain_execution_run_store::OnchainExecutionRunStore::load(&config).store;
    executions.append_pending(&execution).unwrap();
    let restored = crate::services::onchain_execution_run_store::OnchainExecutionRunStore::load(&config);
    assert!(restored.store.readiness().is_ok());
    assert!(restored.store.check_approval_available(&execution.response.approval_costs).is_err());
    assert_eq!(restored.runs[0].approval_costs, execution.response.approval_costs);
}

#[tokio::test]
async fn approval_receipt_revert_still_records_gas_and_wrong_spender_cannot_confirm_approval() {
    let dir = tempfile::tempdir().unwrap();
    let (store, row, mut tx) = setup(dir.path().join("runs.jsonl"));
    let mut failed = tx.clone();
    failed["status"] = json!("0x0");
    failed["logs"] = json!([]);
    let receipt = read(&row, failed).await;
    assert_eq!(receipt.status, Status::ReviewRequired);
    assert_eq!(
        receipt.network_cost.unwrap().total_fee_exact.as_deref(),
        Some("0.000021")
    );
    tx["logs"][0]["topics"][2] = json!(format!("0x{:064x}", 999));
    let receipt = read(&row, tx).await;
    assert_eq!(receipt.status, Status::ReviewRequired);
    let result = store.receipt("run-rpc", receipt, 7000).unwrap();
    assert_ne!(
        result.status,
        shared_types::OnchainTokenApprovalRunStatus::Completed
    );
    assert!(result.fee_receipts[0]
        .problem
        .as_ref()
        .unwrap()
        .contains("Approval"));
}

#[tokio::test]
async fn approval_receipt_missing_event_is_not_success_even_if_evm_status_is_one() {
    let dir = tempfile::tempdir().unwrap();
    let (_, row, mut tx) = setup(dir.path().join("runs.jsonl"));
    tx["logs"] = json!([]);
    let receipt = read(&row, tx).await;
    assert_eq!(receipt.status, Status::ReviewRequired);
    assert!(receipt.network_cost.is_some());
}

#[tokio::test]
async fn approval_receipt_two_transaction_plan_does_not_auto_continue_after_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two.jsonl");
    let mut plan = test_plan();
    let mut reset = plan.transactions[0].clone();
    if let shared_types::OnchainUnsignedTransaction::EvmCall { data, .. } = &mut reset {
        *data = format!("0x095ea7b3{:064x}{:064x}", 3, 0);
    }
    plan.transactions.insert(0, reset);
    let store = OnchainTokenApprovalRunStore::load_path(Some(path.clone()));
    let response = serde_json::from_value(
        json!({"runId":"run-rpc","approvalId":"approval-one","status":"awaiting_finality",
        "transactionIds":[],"message":"fixture","startedAtMs":1100,"updatedAtMs":1100}),
    )
    .unwrap();
    store.create(plan, response).unwrap();
    let hash = format!("0x{:064x}", 10);
    store.intent("run-rpc", 0, &hash, 1200).unwrap();
    let row = store.record("run-rpc").unwrap();
    let (_, _, mut tx) = setup(dir.path().join("single.jsonl"));
    tx["logs"][0]["data"] = json!(format!("0x{:064x}", 0));
    let receipt = read(&row, tx).await;
    let restored = OnchainTokenApprovalRunStore::load_path(Some(path));
    restored.receipt("run-rpc", receipt, 7000).unwrap();
    assert!(restored
        .intent("run-rpc", 1, &format!("0x{:064x}", 11), 8000)
        .is_err());
    let run = restored.record("run-rpc").unwrap().response;
    assert_eq!(run.transaction_ids, vec![hash]);
    assert_eq!(
        run.status,
        shared_types::OnchainTokenApprovalRunStatus::FinalityUnresolved
    );
    assert_eq!(run.fee_receipts.len(), 1);
}
