use super::super::{
    rpc_tests::{client, fixture},
    swap::tests::{solana_basis, solana_tx},
};
use super::*;
use crate::services::onchain_cross_chain_run_store::{receipts, OnchainCrossChainRunStore};
use serde_json::json;
use shared_types::{OnchainCrossChainRun, OnchainCrossChainRunStatus as RunStatus};

fn submitted_store(path: &std::path::Path) -> (OnchainCrossChainRunStore, OnchainCrossChainRun) {
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_cross_chain_ledger_path = Some(path.display().to_string());
    let store = OnchainCrossChainRunStore::load(&config);
    let build = serde_json::from_value(json!({
        "buildId":"receipt-build","provider":"fixture","sourceChain":"solana","peerChain":"ethereum",
        "initialQuoteAmountRaw":"100000000","finalQuoteAmountRaw":"105000000",
        "quoteObservedAtMs":10,"builtAtMs":10,"validUntilMs":1000,
        "atomic":false,"monitorOnly":false,"previewReady":true,"submitReady":true,
        "legs":[{"position":1,"kind":"source_swap","provider":"fixture","fromChain":"solana","toChain":"solana",
            "fromAsset":"USDT","toAsset":"USDC","fromToken":"mint-a","toToken":"mint-b",
            "inputAmountRaw":"100000000","expectedOutputAmountRaw":"105000000","minimumOutputAmountRaw":"104000000",
            "inputDecimals":6,"outputDecimals":6,"officialDocsUrl":SOLANA_TRANSACTION_DOCS,"observedAtMs":10}]
    })).unwrap();
    store.insert_build(build, 10).unwrap();
    let run = store
        .authorize("receipt-build", "receipt-key", "tester", 20)
        .unwrap()
        .run;
    let execution = serde_json::from_value(json!({
        "executionId":"swap","position":1,"kind":"source_swap","provider":"fixture","chain":"solana",
        "walletAddress":"wallet","inputToken":"mint-a","outputToken":"mint-b","inputAmountRaw":"100000000",
        "quotedOutputAmountRaw":"105000000","minimumOutputAmountRaw":"104000000","quoteObservedAtMs":20,
        "validUntilMs":1000,"rebuildAfterPosition":0,"officialDocsUrl":SOLANA_TRANSACTION_DOCS,
        "transaction":{"kind":"solana_versioned","transaction_base64":"fixture-only","request_id":"fixture","router":"fixture","mode":"fixture"}
    })).unwrap();
    let claimed = store
        .claim_leg(
            &run.run_id,
            "tester",
            1,
            "100000000".into(),
            "104000000".into(),
            "swap".into(),
            Some(execution),
            None,
            20,
            1000,
            "105000000".into(),
            "100000000".into(),
            "400".into(),
            30,
        )
        .unwrap()
        .run;
    assert!(claimed.legs[0].actual_input_amount_raw.is_none());
    assert_eq!(
        claimed.legs[0].submitted_input_amount_raw.as_deref(),
        Some("100000000")
    );
    let submitted = store
        .record_submission_intent(&run.run_id, "signature".into(), "fixture".into(), 40)
        .unwrap();
    (store, submitted)
}

async fn read_swap(tx: Value) -> OnchainWalletReceipt {
    let fixture = fixture(vec![
        ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
        ("getTransaction", tx),
    ])
    .await;
    let swap = solana_basis();
    let row = read_on_rpc(
        &client(),
        &fixture.url,
        &OnchainWalletReceiptBasis {
            chain: swap.chain,
            wallet: swap.wallet,
            transaction_id: swap.transaction_id,
            assets: vec![swap.assets.input, swap.assets.output],
            require_sender: true,
        },
    )
    .await
    .unwrap();
    assert!(fixture.replies.lock().unwrap().is_empty());
    row
}

#[tokio::test]
async fn cross_chain_wallet_rpc_to_journal_keeps_submitted_actual_and_replay_distinct() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let (store, run) = submitted_store(&path);
    let receipt = read_swap(solana_tx()).await;
    let now = common::time::now_ms();
    let result = store
        .record_wallet_receipt(&run.run_id, 1, receipt.clone(), false, None, now)
        .unwrap();
    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(
        result.legs[0].submitted_input_amount_raw.as_deref(),
        Some("100000000")
    );
    assert_eq!(
        result.legs[0].actual_input_amount_raw.as_deref(),
        Some("99000000")
    );
    assert_eq!(
        result.legs[0].actual_output_amount_raw.as_deref(),
        Some("105000000")
    );
    assert_eq!(
        result.legs[0]
            .source_receipt
            .as_ref()
            .unwrap()
            .network_cost
            .as_ref()
            .unwrap()
            .total_fee_exact
            .as_deref(),
        Some("0.000005")
    );
    let before = std::fs::read(&path).unwrap();
    assert!(store
        .record_wallet_receipt(&run.run_id, 1, receipt.clone(), false, None, now)
        .is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_cross_chain_ledger_path = Some(path.display().to_string());
    let restored = OnchainCrossChainRunStore::load(&config)
        .run(&run.run_id, now)
        .unwrap();
    assert_eq!(restored, result);
    if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_RECEIPT_FIXTURE") {
        std::fs::write(path, serde_json::to_vec(&result).unwrap()).unwrap();
    }
}

#[tokio::test]
async fn cross_chain_wallet_missing_fee_is_retained_then_bounded_and_readonly_recoverable() {
    let dir = tempfile::tempdir().unwrap();
    let (store, run) = submitted_store(&dir.path().join("runs.jsonl"));
    let mut tx = solana_tx();
    tx["meta"].as_object_mut().unwrap().remove("fee");
    let receipt = read_swap(tx).await;
    let now = common::time::now_ms();
    let result = store
        .record_wallet_receipt(&run.run_id, 1, receipt.clone(), false, None, now)
        .unwrap();
    assert_eq!(result.status, RunStatus::AwaitingSourceFinality);
    assert_eq!(
        result.legs[0].actual_input_amount_raw.as_deref(),
        Some("99000000")
    );
    for _ in 1..12 {
        store
            .record_wallet_receipt(
                &run.run_id,
                1,
                pending_receipt(&receipt.basis, "RPC timeout".into()),
                false,
                None,
                now,
            )
            .unwrap();
    }
    let waiting = store.run(&run.run_id, now).unwrap();
    assert_eq!(waiting.status, RunStatus::AwaitingSourceFinality);
    let deadline = waiting.automatic_check_deadline_ms().unwrap().max(now);
    let paused = store.claim_reconciliation(&run.run_id, deadline).unwrap().unwrap();
    assert_eq!(paused.status, RunStatus::Paused);
    assert_eq!(
        paused.legs[0]
            .source_receipt
            .as_ref()
            .unwrap()
            .asset_changes_raw[0]
            .as_deref(),
        Some("-99000000")
    );
    assert_eq!(paused.legs[0].attempts, 1);
    let resumed = store
        .request_recheck(&run.run_id, "tester", 1, deadline + 1)
        .unwrap();
    assert_eq!(resumed.legs[0].receipt_checks, 0);
    let claimed = store.claim_reconciliation(&run.run_id, deadline + 1).unwrap().unwrap();
    assert_eq!(claimed.legs[0].recovery_checks, 1);
    let result = store
        .record_wallet_receipt(
            &run.run_id,
            1,
            read_swap(solana_tx()).await,
            false,
            None,
            common::time::now_ms().max(deadline + 1),
        )
        .unwrap();
    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(store.finish_reconciliation(&run.run_id, common::time::now_ms().max(deadline + 1)).unwrap().status, RunStatus::Completed);
    assert_eq!(result.legs[0].attempts, 1);
}

#[tokio::test]
async fn cross_chain_wallet_failure_and_contract_mismatch_never_advance() {
    for case in ["failed", "overdraw", "underpaid", "hash", "position"] {
        let dir = tempfile::tempdir().unwrap();
        let (store, run) = submitted_store(&dir.path().join("runs.jsonl"));
        let mut tx = solana_tx();
        if case == "failed" {
            tx["meta"]["err"] = json!({"InstructionError":[0,"Custom"]});
        }
        if case == "underpaid" {
            tx["meta"]["postTokenBalances"][1]["uiTokenAmount"]["amount"] = json!("103000000");
        }
        let mut receipt = read_swap(tx).await;
        if case == "overdraw" {
            receipt.asset_changes_raw[0] = Some("-100000001".into());
        }
        if case == "hash" {
            receipt.basis.transaction_id = "other".into();
        }
        let result = store.record_wallet_receipt(
            &run.run_id,
            if case == "position" { 2 } else { 1 },
            receipt,
            false,
            None,
            common::time::now_ms(),
        );
        if matches!(case, "hash" | "position") {
            assert!(result.is_err(), "{case}");
        } else {
            let result = result.unwrap();
            assert_eq!(result.status, RunStatus::Paused, "{case}");
            assert!(result.legs[0]
                .source_receipt
                .as_ref()
                .unwrap()
                .network_cost
                .is_some());
        }
    }
}

#[tokio::test]
async fn cross_chain_wallet_bridge_destination_preserves_sponsor_fee_and_exact_credit() {
    let row = read_bridge_destination().await;
    assert_eq!(row.status, Status::Complete);
    assert_eq!(row.asset_changes_raw[0].as_deref(), Some("105000000"));
    assert_eq!(row.additional_native_change_raw.as_deref(), Some("0"));
    assert_eq!(
        row.network_cost.as_ref().unwrap().payer,
        format!("0x{:040x}", 2)
    );
    assert_ne!(row.network_cost.as_ref().unwrap().payer, row.basis.wallet);
    assert_eq!(
        row.network_cost.unwrap().total_fee_exact.as_deref(),
        Some("0.000021")
    );
}

async fn read_bridge_destination() -> OnchainWalletReceipt {
    let wallet = format!("0x{:040x}", 1);
    let sender = format!("0x{:040x}", 2);
    let token = format!("0x{:040x}", 3);
    let tx = json!({"hash":"0xtransaction","blockHash":"0xcanonical","from":sender,"to":wallet,"value":"0x0","input":"0x1234"});
    let receipt = json!({"transactionHash":"0xtransaction","blockHash":"0xcanonical","blockNumber":"0x64","from":sender,
        "status":"0x1","gasUsed":"0x5208","effectiveGasPrice":"0x3b9aca00","logs":[{"address":token,"logIndex":"0x0",
        "topics":[ERC20_TRANSFER_TOPIC,format!("0x{:064x}", 2),format!("0x{:064x}", 1)],"data":format!("0x{:064x}",105000000),"removed":false}]});
    let fixture = fixture(vec![
        ("eth_chainId", json!("0x1")),
        ("eth_getTransactionReceipt", receipt),
        ("eth_getBlockByNumber", json!({"hash":"0xcanonical"})),
        ("eth_blockNumber", json!("0x65")),
        ("eth_getTransactionByHash", tx),
        (
            "debug_traceTransaction",
            json!({"type":"CALL","from":sender,"to":wallet,"value":"0x0"}),
        ),
    ])
    .await;
    let row = read_on_rpc(
        &client(),
        &fixture.url,
        &OnchainWalletReceiptBasis {
            chain: "ethereum".into(),
            wallet: wallet.clone(),
            transaction_id: "0xtransaction".into(),
            require_sender: false,
            assets: vec![shared_types::OnchainExecutionToken {
                symbol: "USDC".into(),
                address: token,
                decimals: 6,
            }],
        },
    )
    .await
    .unwrap();
    assert!(fixture.replies.lock().unwrap().is_empty());
    row
}

#[tokio::test]
async fn cross_chain_wallet_bridge_requires_both_chain_receipts_and_preserves_mismatched_credit() {
    for provider_amount in ["105000000", "105000001"] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");
        // The template's submitted swap must not occupy the wallet used by this independent bridge.
        let (_, template) = submitted_store(&dir.path().join("template.jsonl"));
        let mut config = common::config::AppConfig::default();
        config.storage.onchain_cross_chain_ledger_path = Some(path.display().to_string());
        let store = OnchainCrossChainRunStore::load(&config);
        let mut build = template.build.clone();
        build.build_id = "bridge-build".into();
        build.legs[0].kind = shared_types::OnchainCrossChainLegKind::OutboundBridge;
        build.legs[0].to_chain = "ethereum".into();
        build.legs[0].to_token = format!("0x{:040x}", 3);
        store.insert_build(build, 10).unwrap();
        let run = store
            .authorize("bridge-build", "bridge-key", "tester", 20)
            .unwrap()
            .run;
        let execution = serde_json::from_value(json!({
            "position":1,"kind":"outbound_bridge","provider":"lifi","routeId":"bridge","transactionId":"bridge-transfer","tool":"fixture",
            "fromChainId":1151111081099710u64,"toChainId":1,"fromAddress":"wallet","toAddress":format!("0x{:040x}", 1),
            "fromToken":"mint-a","toToken":format!("0x{:040x}", 3),"fromAmountRaw":"100000000","toAmountMinRaw":"104000000",
            "quoteObservedAtMs":20,"validUntilMs":1000,"rebuildAfterPosition":0,"officialDocsUrl":"https://docs.li.fi",
            "transaction":template.legs[0].swap_execution.as_ref().unwrap().transaction
        })).unwrap();
        store
            .claim_leg(
                &run.run_id,
                "tester",
                1,
                "100000000".into(),
                "104000000".into(),
                "bridge-transfer".into(),
                None,
                Some(execution),
                20,
                1000,
                "105000000".into(),
                "100000000".into(),
                "400".into(),
                30,
            )
            .unwrap();
        let run = store
            .record_submission_intent(&run.run_id, "signature".into(), "fixture".into(), 40)
            .unwrap();
        let destination = read_bridge_destination().await;
        assert!(store
            .record_wallet_receipt(
                &run.run_id,
                1,
                destination.clone(),
                true,
                Some(provider_amount),
                common::time::now_ms()
            )
            .is_err());
        let rpc = fixture(vec![
            ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
            ("getTransaction", solana_tx()),
        ])
        .await;
        let source = read_on_rpc(
            &client(),
            &rpc.url,
            &receipts::basis(&run, &run.legs[0], None).unwrap(),
        )
        .await
        .unwrap();
        if provider_amount == "105000000" {
            let mut partial = source.clone();
            partial.status = Status::Pending;
            partial.problem = Some("source fee unavailable".into());
            partial.network_cost.as_mut().unwrap().total_fee_exact = None;
            store
                .record_wallet_receipt(&run.run_id, 1, partial, false, None, common::time::now_ms())
                .unwrap();
            let arrived = store
                .record_wallet_receipt(
                    &run.run_id,
                    1,
                    destination.clone(),
                    true,
                    Some(provider_amount),
                    common::time::now_ms(),
                )
                .unwrap();
            assert_ne!(arrived.status, RunStatus::Completed);
            assert_eq!(
                arrived.legs[0].actual_output_amount_raw.as_deref(),
                Some("105000000")
            );
            assert!(arrived.problem.as_deref().unwrap().contains("源链费用"));
        }
        let pending = store
            .record_wallet_receipt(&run.run_id, 1, source, false, None, common::time::now_ms())
            .unwrap();
        assert_eq!(pending.status, RunStatus::AwaitingDestinationEvidence);
        if provider_amount != "105000000" {
            assert!(pending.legs[0].actual_output_amount_raw.is_none());
        }
        let finished = store
            .record_wallet_receipt(
                &run.run_id,
                1,
                destination,
                true,
                Some(provider_amount),
                common::time::now_ms(),
            )
            .unwrap();
        assert_eq!(
            finished.legs[0].actual_input_amount_raw.as_deref(),
            Some("99000000")
        );
        assert_eq!(
            finished.legs[0].actual_output_amount_raw.as_deref(),
            Some("105000000")
        );
        assert!(finished.legs[0]
            .destination_receipt
            .as_ref()
            .unwrap()
            .network_cost
            .is_some());
        assert_eq!(
            finished.status,
            if provider_amount == "105000000" {
                RunStatus::Completed
            } else {
                RunStatus::Paused
            }
        );
        assert!(rpc.replies.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn cross_chain_wallet_journal_failure_does_not_advance_memory() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let (store, run) = submitted_store(&path);
    let receipt = read_swap(solana_tx()).await;
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(store
        .record_wallet_receipt(&run.run_id, 1, receipt, false, None, common::time::now_ms())
        .is_err());
    assert_eq!(store.run(&run.run_id, 50).unwrap(), run);
}

#[test]
fn cross_chain_wallet_legacy_journal_does_not_turn_planned_input_into_actual_debit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let (_, run) = submitted_store(&path);
    let mut legacy = serde_json::to_value(&run).unwrap();
    legacy["legs"][0]
        .as_object_mut()
        .unwrap()
        .remove("submittedInputAmountRaw");
    legacy["legs"][0]["actualInputAmountRaw"] = json!("100000000");
    let entry = json!({"schemaVersion":1,"run":legacy});
    std::fs::write(&path, format!("{entry}\n")).unwrap();
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_cross_chain_ledger_path = Some(path.display().to_string());
    let restored = OnchainCrossChainRunStore::load(&config)
        .run(&run.run_id, 50)
        .unwrap();
    assert_eq!(
        restored.legs[0].submitted_input_amount_raw.as_deref(),
        Some("100000000")
    );
    assert!(restored.legs[0].actual_input_amount_raw.is_none());
}

#[test]
fn cross_chain_wallet_basis_is_bound_to_durable_contract() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut run) = submitted_store(&dir.path().join("runs.jsonl"));
    assert_eq!(
        receipts::basis(&run, &run.legs[0], None).unwrap().assets[0].address,
        "mint-a"
    );
    run.build.legs[0].from_chain = "ethereum".into();
    assert!(receipts::basis(&run, &run.legs[0], None).is_err());
}

#[tokio::test]
async fn cross_chain_wallet_solana_receipt_does_not_require_recipient_owner_as_transaction_account()
{
    let mut tx = solana_tx();
    tx["transaction"]["message"]["accountKeys"][0] = json!("sponsor");
    let basis = OnchainWalletReceiptBasis {
        chain: "solana".into(),
        wallet: "wallet".into(),
        transaction_id: "signature".into(),
        require_sender: false,
        assets: vec![solana_basis().assets.output],
    };
    let rpc = fixture(vec![
        ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
        ("getTransaction", tx.clone()),
    ])
    .await;
    let receipt = read_on_rpc(&client(), &rpc.url, &basis).await.unwrap();
    assert_eq!(receipt.status, Status::Complete);
    assert_eq!(receipt.asset_changes_raw[0].as_deref(), Some("105000000"));
    assert_eq!(receipt.additional_native_change_raw.as_deref(), Some("0"));
    assert_eq!(receipt.network_cost.unwrap().payer, "sponsor");
    assert!(rpc.replies.lock().unwrap().is_empty());
    let mut tx_wrapped = tx.clone();
    for phase in ["preTokenBalances", "postTokenBalances"] {
        tx_wrapped["meta"][phase][1]["mint"] = json!(SOLANA_WRAPPED_SOL_MINT);
    }
    assert_eq!(
        solana::recipient_native_change(&tx_wrapped, "wallet"),
        Ok(105000000)
    );
    let rpc = fixture(vec![
        ("getGenesisHash", json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
        ("getTransaction", tx),
    ])
    .await;
    let mut source = basis;
    source.require_sender = true;
    assert!(read_on_rpc(&client(), &rpc.url, &source).await.is_err());
}
