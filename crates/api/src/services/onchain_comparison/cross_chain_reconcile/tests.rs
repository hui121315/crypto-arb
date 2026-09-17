use super::*;
use serde_json::json;

fn run_fixture() -> OnchainCrossChainRun {
    let route = json!({"position":2,"kind":"outbound_bridge","provider":"lifi",
        "fromChain":"base","toChain":"arbitrum","fromAsset":"USDC","toAsset":"USDC",
        "fromToken":"0xsource-token","toToken":"0xtoken","inputAmountRaw":"100",
        "expectedOutputAmountRaw":"100","minimumOutputAmountRaw":"98",
        "inputDecimals":6,"outputDecimals":6,"officialDocsUrl":LIFI_STATUS_DOCS,"observedAtMs":1});
    let build = json!({"buildId":"build","provider":"lifi","sourceChain":"base","peerChain":"arbitrum",
            "initialQuoteAmountRaw":"100","finalQuoteAmountRaw":"101",
            "quoteObservedAtMs":1,"builtAtMs":1,"validUntilMs":1000,
            "atomic":false,"monitorOnly":false,"previewReady":true,"submitReady":true,
            "legs":[route]});
    let transaction = json!({"kind":"evm_call","chain_id":8453,"from":"0xsender",
        "to":"0xbridge","data":"0x1234","value":"0x0","gas":"0x5208"});
    let execution = json!({"position":2,"kind":"outbound_bridge","provider":"lifi",
                "routeId":"route","transactionId":"transfer-id","tool":"across",
                "fromChainId":8453,"toChainId":42161,"fromAddress":"0xsender","toAddress":"0xwallet",
                "fromToken":"0xsource-token","toToken":"0xtoken","fromAmountRaw":"100","toAmountMinRaw":"98",
                "quoteObservedAtMs":1,"validUntilMs":1000,"rebuildAfterPosition":1,"officialDocsUrl":LIFI_STATUS_DOCS,
                "transaction":transaction});
    serde_json::from_value(json!({
        "runId":"run", "idempotencyKey":"key", "status":"awaiting_destination_evidence",
        "authorization":{"actor":"tester","authorizedAtMs":1,"validUntilMs":1000,"confirmationVersion":"v1"},
        "activePosition":2,"createdAtMs":1,"updatedAtMs":2,"nextAction":"verify","build":build,
        "legs":[{"position":2,"kind":"outbound_bridge","clientActionId":"action",
            "status":"source_confirmed","attempts":1,"plannedInputAmountRaw":"100",
            "sourceTransactionId":"0xsource","minimumOutputAmountRaw":"98","bridgeExecution":execution}]
    })).unwrap()
}

fn evidence() -> LifiTransferEvidence {
    LifiTransferEvidence {
        state: LifiTransferState::Completed,
        provider_status: "DONE".into(),
        substatus: Some("COMPLETED".into()),
        message: None,
        transaction_id: Some("transfer-id".into()),
        sending_tx_hash: Some("0xsource".into()),
        receiving_tx_hash: Some("0xdestination".into()),
        receiving_amount_raw: Some("100".into()),
        sending_chain_id: Some(8453),
        receiving_chain_id: Some(42161),
        receiving_token: Some("0xtoken".into()),
        receiving_token_chain_id: Some(42161),
        to_address: Some("0xwallet".into()),
        observed_at_ms: 2,
        official_docs_url: LIFI_STATUS_DOCS.into(),
    }
}

#[test]
fn bridge_credit_is_bound_to_durable_destination_not_current_configuration() {
    let run = run_fixture();
    let result = bridge::credit_request(&run, &run.legs[0], &evidence())
        .unwrap()
        .unwrap();
    assert_eq!(result.chain, "arbitrum");
    assert_eq!(result.wallet, "0xwallet");
    assert_eq!(result.assets[0].address, "0xtoken");
    assert_eq!(result.transaction_id, "0xdestination");
    assert_eq!(run.legs[0].minimum_output_amount_raw.as_deref(), Some("98"));
}

#[test]
fn bridge_success_cannot_override_transaction_chain_wallet_or_token_identity() {
    let run = run_fixture();
    let mutations: [fn(&mut LifiTransferEvidence); 7] = [
        |value| value.sending_tx_hash = Some("0xother-source".into()),
        |value| value.transaction_id = Some("other-transfer".into()),
        |value| value.sending_chain_id = Some(1),
        |value| value.receiving_chain_id = Some(1),
        |value| value.receiving_token_chain_id = Some(1),
        |value| value.to_address = Some("0xother-wallet".into()),
        |value| value.receiving_token = Some("0xother-token".into()),
    ];
    for change in mutations {
        let mut value = evidence();
        change(&mut value);
        assert!(bridge::credit_request(&run, &run.legs[0], &value).is_err());
    }
    let mut incompatible = run.clone();
    incompatible.build.legs[0].to_chain = "optimism".into();
    assert!(bridge::credit_request(&incompatible, &incompatible.legs[0], &evidence()).is_err());
}

#[test]
fn bridge_missing_identity_remains_pending() {
    let run = run_fixture();
    let mutations: [fn(&mut LifiTransferEvidence); 3] = [
        |value| value.sending_tx_hash = None,
        |value| value.receiving_chain_id = None,
        |value| value.receiving_tx_hash = None,
    ];
    for change in mutations {
        let mut value = evidence();
        change(&mut value);
        assert!(bridge::credit_request(&run, &run.legs[0], &value)
            .unwrap()
            .is_none());
    }
}

#[test]
fn completed_leg_notifications_do_not_share_the_same_deduplication_id() {
    let mut run = run_fixture();
    run.status = OnchainCrossChainRunStatus::Running;
    run.active_position = None;
    run.legs[0].status = OnchainCrossChainLegRunStatus::Completed;
    let first = webhook_event_id(&run);
    run.legs[0].position = 3;
    let next = webhook_event_id(&run);
    assert_ne!(first, next);
    assert_eq!(next, webhook_event_id(&run));
}

#[test]
fn cross_chain_manual_pause_notification_is_deduped_per_recovery_attempt() {
    let mut run = run_fixture();
    run.status = OnchainCrossChainRunStatus::Paused;
    let automatic = webhook_event_id(&run);
    run.legs[0].recovery_started_at_ms = Some(1000);
    let manual = webhook_event_id(&run);
    assert_ne!(automatic, manual);
    assert_eq!(manual, webhook_event_id(&run));
    run.legs[0].recovery_checks = 12;
    assert_eq!(manual, webhook_event_id(&run));
    run.legs[0].recovery_started_at_ms = Some(2000);
    assert_ne!(manual, webhook_event_id(&run));
}

#[tokio::test]
async fn bridge_recovery_http_rpc_to_journal_uses_net_credit_and_does_not_charge_sponsor() {
    use crate::services::onchain_cross_chain_run_store::recovery::{basis, tests::{pending_bridge, refund_report}};
    use crate::services::onchain_comparison::replenishment_credit::{rpc_tests::{fixture, client}, wallet};
    use shared_types::{OnchainChainSettlementStatus as ReceiptStatus, OnchainCrossChainFlowKind as Kind};
    let dir = tempfile::tempdir().unwrap();
    let (store, run) = pending_bridge(&dir.path().join("http-refund.jsonl"));
    let mut report = refund_report(&run);
    let basis = basis(&run, &run.legs[1], &report).unwrap();
    let sponsor = format!("0x{:040x}", 2);
    let topic = |address: &str| format!("0x{:0>64}", address.trim_start_matches("0x"));
    let fixture = fixture(vec![
        ("eth_chainId", json!("0x1")),
        ("eth_getTransactionReceipt", json!({"transactionHash":basis.transaction_id,"from":sponsor,
            "to":basis.assets[0].address,"status":"0x1","blockNumber":"0x64","blockHash":"0xcanonical",
            "gasUsed":"0x5208","effectiveGasPrice":"0x3b9aca00","logs":[{
                "address":basis.assets[0].address,"logIndex":"0x0","removed":false,
                "transactionHash":basis.transaction_id,"blockHash":"0xcanonical",
                "topics":["0xddf252ad1be2c89b69c2b068fc378daa952ba7f163c4a11628f55a4df523b3ef",topic(&sponsor),topic(&basis.wallet)],
                "data":format!("0x{:064x}", 98_000_000u128)}]})),
        ("eth_getBlockByNumber", json!({"hash":"0xcanonical"})),
        ("eth_blockNumber", json!("0x65")),
        ("eth_getTransactionByHash", json!({"hash":basis.transaction_id,"blockHash":"0xcanonical",
            "from":sponsor,"to":basis.assets[0].address,"value":"0x0","input":"0x1234"})),
        ("debug_traceTransaction", json!({"type":"CALL","from":sponsor,"to":basis.assets[0].address,"value":"0x0"})),
    ]).await;
    let receipt = wallet::read_on_rpc(&client(), &fixture.url, &basis).await.unwrap();
    assert!(fixture.replies.lock().unwrap().is_empty());
    assert_eq!(receipt.status, ReceiptStatus::Complete);
    assert_eq!(receipt.asset_changes_raw[0].as_deref(), Some("98000000"));
    assert_eq!(receipt.network_cost.as_ref().unwrap().total_fee_exact.as_deref(), Some("0.000021"));
    report.receipt = Some(receipt);
    let updated = store.record_bridge_recovery(&run.run_id, 2, report, common::time::now_ms()).unwrap();
    let accounting = updated.accounting.unwrap();
    assert_eq!(accounting.flows.iter().find(|flow| flow.kind == Kind::Recovery).unwrap().change.amount_exact, "98");
    assert_eq!(accounting.flows.iter().filter(|flow| flow.kind == Kind::NetworkFee).count(), 2);
    assert!(accounting.usd_value.is_none());
    assert_eq!(updated.status, OnchainCrossChainRunStatus::Paused);
}
