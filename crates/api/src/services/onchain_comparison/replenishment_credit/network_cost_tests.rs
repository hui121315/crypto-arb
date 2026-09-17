use super::*;
use crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore;
use shared_types::{
    OnchainReplenishmentNetworkCost, OnchainReplenishmentRun, OnchainReplenishmentRunStatus,
    OnchainReplenishmentTransferStatus,
};

fn charged_receipt() -> Value {
    let mut row = receipt();
    row["from"] = json!(format!("0x{:040x}", 1));
    row["gasUsed"] = json!("0x5208");
    row["effectiveGasPrice"] = json!("0x3b9aca00");
    row
}

async fn evm_cost(
    chain: &str,
    row: Value,
    operator: Option<Value>,
) -> (
    DestinationCreditCheck,
    Option<OnchainReplenishmentNetworkCost>,
) {
    let id = shared_types::onchain_chain_preset(chain)
        .unwrap()
        .chain_id
        .unwrap();
    let mut replies = vec![
        ("eth_chainId", json!(format!("0x{id:x}"))),
        ("eth_getTransactionReceipt", row),
        ("eth_getBlockByNumber", json!({"hash":"0xcanonical"})),
        ("eth_blockNumber", json!("0x65")),
    ];
    if let Some(value) = operator {
        replies.push(("eth_call", value));
    }
    let fixture = fixture(replies).await;
    let mut cost = None;
    let credit = check_on_rpc_with_cost(
        &client(),
        &fixture.url,
        chain,
        &scope(),
        "0xtransaction",
        Some(&mut cost),
    )
    .await;
    assert!(
        fixture.replies.lock().unwrap().is_empty(),
        "{chain}: unexpected extra/missing fee RPC"
    );
    (credit, cost)
}

#[tokio::test]
async fn network_cost_evm_covers_registered_chains_and_preserves_reverted_fees() {
    for chain in [
        "ethereum",
        "arbitrum",
        "polygon",
        "bnb-smart-chain",
        "avalanche",
        "base",
        "optimism",
    ] {
        for failed in [false, true] {
            let mut row = charged_receipt();
            if failed {
                row["status"] = json!("0x0");
                row["logs"] = json!([]);
            }
            let op = matches!(chain, "base" | "optimism");
            if op {
                row["l1Fee"] = json!("0x3b9aca00");
            }
            // A separate L1 gas field on Arbitrum is already part of gasUsed.
            if chain == "arbitrum" {
                row["gasUsedForL1"] = json!("0x100");
            }
            let (credit, cost) = evm_cost(chain, row, None).await;
            assert_eq!(
                matches!(credit, DestinationCreditCheck::Rejected { .. }),
                failed
            );
            let cost = cost.unwrap();
            assert_eq!(
                cost.total_fee_exact.as_deref(),
                Some(if op { "0.000021001" } else { "0.000021" })
            );
            assert_eq!(
                cost.asset,
                shared_types::onchain_chain_preset(chain)
                    .unwrap()
                    .base_token
            );
            assert!(cost.problem.is_none());
        }
    }
}

#[tokio::test]
async fn network_cost_op_historical_oracle_and_missing_components_never_assume_zero() {
    for invalid in [false, true] {
        let mut row = charged_receipt();
        row["l1Fee"] = json!("0x3b9aca00");
        row["operatorFeeScalar"] = json!("0x10");
        row["operatorFeeConstant"] = json!("0x20");
        let answer = if invalid {
            json!("0x")
        } else {
            json!(format!("0x{:064x}", 2000))
        };
        let (credit, cost) = evm_cost("base", row, Some(answer)).await;
        assert!(matches!(credit, DestinationCreditCheck::Credited { .. }));
        let cost = cost.unwrap();
        assert_eq!(cost.execution_fee_exact.as_deref(), Some("0.000021"));
        assert_eq!(
            cost.total_fee_exact.as_deref(),
            if invalid {
                None
            } else {
                Some("0.000021001000002")
            }
        );
        assert_eq!(cost.problem.is_some(), invalid);
    }
    for field in ["l1Fee", "gasUsed", "effectiveGasPrice"] {
        let mut row = charged_receipt();
        row["l1Fee"] = json!("0x0");
        row.as_object_mut().unwrap().remove(field);
        let (_, cost) = evm_cost("optimism", row, None).await;
        assert!(cost.unwrap().total_fee_exact.is_none(), "{field}");
    }
    let mut blob = charged_receipt();
    blob["type"] = json!("0x3");
    let (_, cost) = evm_cost("ethereum", blob.clone(), None).await;
    assert!(cost.unwrap().total_fee_exact.is_none());
    blob["blobGasUsed"] = json!("0x20000");
    blob["blobGasPrice"] = json!("0x1");
    assert_eq!(
        evm_cost("ethereum", blob, None)
            .await
            .1
            .unwrap()
            .total_fee_exact
            .as_deref(),
        Some("0.000021000000131072")
    );
}

#[tokio::test]
async fn network_cost_unconfirmed_or_reorg_receipt_cannot_record_spend() {
    let fixture = fixture(vec![
        ("eth_chainId", json!("0x1")),
        ("eth_getTransactionReceipt", charged_receipt()),
        ("eth_getBlockByNumber", json!({"hash":"0xother-fork"})),
    ])
    .await;
    let mut cost = None;
    let result = check_on_rpc_with_cost(
        &client(),
        &fixture.url,
        "ethereum",
        &scope(),
        "0xtransaction",
        Some(&mut cost),
    )
    .await;
    assert!(matches!(result, DestinationCreditCheck::Pending { .. }));
    assert!(cost.is_none());
    assert!(fixture.replies.lock().unwrap().is_empty());
}

#[tokio::test]
async fn network_cost_solana_failed_transaction_preserves_actual_payer_and_fee() {
    let failure = json!({"InstructionError":[0,"InvalidArgument"]});
    for fee in [json!(5000), Value::Null, json!(0)] {
        let fixture = fixture(vec![("getGenesisHash",json!("5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d")),
            ("getSignatureStatuses",json!({"value":[{"slot":120,"err":failure,"confirmations":null,"confirmationStatus":"finalized"}]})),
            ("getTransaction",json!({"slot":120,"transaction":{"signatures":["signature"],"message":{
                "accountKeys":[{"pubkey":"payer","signer":true},"wallet"]}},"meta":{"err":failure,"fee":fee}}))]).await;
        let mut cost = None;
        let result = check_on_rpc_with_cost(
            &client(),
            &fixture.url,
            "solana",
            &scope(),
            "signature",
            Some(&mut cost),
        )
        .await;
        assert!(matches!(result, DestinationCreditCheck::Rejected { .. }));
        let cost = cost.unwrap();
        assert_eq!(cost.payer, "payer");
        assert_eq!(
            cost.total_fee_exact.as_deref(),
            match fee.as_u64() {
                Some(5000) => Some("0.000005"),
                Some(0) => Some("0"),
                _ => None,
            }
        );
        assert!(fixture.replies.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn network_cost_rpc_to_ledger_is_atomic_restart_safe_and_identity_bound() {
    let (_, cost) = evm_cost("ethereum", charged_receipt(), None).await;
    let cost = cost.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run.jsonl");
    let mut config = common::config::AppConfig::default();
    config.storage.onchain_replenishment_ledger_path = Some(path.to_string_lossy().into_owned());
    let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../shared-types/fixtures/onchain_replenishment_locked.json"
    )))
    .unwrap();
    run.status = OnchainReplenishmentRunStatus::AwaitingSourceFinality;
    run.plan.legs[0].chain = cost.chain.clone();
    run.plan.legs[0].source_address = Some(cost.payer.clone());
    run.transfers[0].transaction_id = Some(cost.transaction_id.clone());
    run.transfers[0].status = OnchainReplenishmentTransferStatus::Submitted;
    run.transfers[0].last_checked_at_ms = None;
    run.transfers[0].submission_attempted_at_ms = cost.observed_at_ms - 10;
    std::fs::write(&path, format!("{}\n", json!({"schemaVersion":1,"run":run}))).unwrap();
    let store = OnchainReplenishmentPlanStore::load(&config);
    let record = |cost: OnchainReplenishmentNetworkCost| {
        store.record_chain_source_status(
            &run.run_id,
            cost.transaction_id.clone(),
            OnchainReplenishmentTransferStatus::SourceCompleted,
            Some(2),
            cost.source.clone(),
            None,
            Some(cost.clone()),
            cost.observed_at_ms - 1,
        )
    };
    let updated = record(cost.clone()).unwrap();
    assert_eq!(updated.transfers[0].network_cost.as_ref(), Some(&cost));
    assert_eq!(
        updated.transfers[0].last_checked_at_ms,
        Some(cost.observed_at_ms)
    );
    let before = std::fs::read_to_string(&path).unwrap();
    assert_eq!(before.lines().count(), 2);
    for case in [
        "payer", "hash", "chain", "asset", "block", "time", "total", "regress",
    ] {
        let mut bad = cost.clone();
        match case {
            "payer" => bad.payer = "other-wallet".into(),
            "hash" => bad.transaction_id = "other-hash".into(),
            "chain" => bad.chain = "base".into(),
            "asset" => bad.asset = "SOL".into(),
            "block" => bad.block_ref = "other-block".into(),
            "time" => bad.observed_at_ms -= 1,
            "total" => bad.total_fee_exact = Some("-1".into()),
            "regress" => bad.total_fee_exact = None,
            _ => unreachable!(),
        }
        assert!(record(bad).is_err(), "{case}");
    }
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
    let restored = OnchainReplenishmentPlanStore::load(&config)
        .run(&run.run_id, cost.observed_at_ms + 1)
        .unwrap();
    assert_eq!(restored.transfers, updated.transfers);
}
