use super::*;
use shared_types::{
    OnchainCrossChainFlowKind as Kind, OnchainTokenIdentity, OnchainTokenResolution,
};

pub(crate) fn pending_bridge(path: &Path) -> (OnchainCrossChainRunStore, OnchainCrossChainRun) {
    super::super::verification::tests::pending_bridge(path)
}

pub(crate) fn refund_report(run: &OnchainCrossChainRun) -> OnchainCrossChainRecovery {
    let leg = &run.legs[1];
    let bridge = leg.bridge_execution.as_ref().unwrap();
    let token = shared_types::onchain_known_token("ethereum", "USDC").unwrap();
    OnchainCrossChainRecovery {
        provider_status: "FAILED".into(),
        substatus: Some("REFUNDED".into()),
        message: None,
        provider_transaction_id: Some(bridge.transaction_id.clone()),
        sending_transaction_id: leg.source_transaction_id.clone(),
        sending_chain_id: Some(bridge.from_chain_id),
        receiving_transaction_id: Some(format!("0x{:064x}", 999)),
        receiving_chain_id: Some(1),
        receiving_token_chain_id: Some(1),
        receiving_token: Some(token.address.into()),
        reported_amount_raw: Some("99000000".into()),
        reported_receiver: Some(bridge.from_address.clone()),
        observed_at_ms: 2100,
        official_docs_url:
            "https://docs.li.fi/introduction/user-flows-and-examples/status-tracking".into(),
        token_resolution: Some(OnchainTokenResolution::complete(OnchainTokenIdentity {
            chain: "ethereum".into(),
            address: token.address.into(),
            symbol: "USDC".into(),
            name: None,
            decimals: 6,
            source: "ethereum_rpc".into(),
            evidence_url: "https://eips.ethereum.org/EIPS/eip-20".into(),
            verified: false,
            native: false,
            observed_at_ms: 2100,
        })),
        receipt: None,
        problem: None,
    }
}

pub(crate) fn received(run: &OnchainCrossChainRun) -> OnchainCrossChainRecovery {
    let mut report = refund_report(run);
    let mut receipt = run.legs[1].source_receipt.clone().unwrap();
    receipt.basis = basis(run, &run.legs[1], &report).unwrap();
    receipt.asset_changes_raw = vec![Some("98000000".into())];
    let fee = receipt.network_cost.as_mut().unwrap();
    fee.transaction_id = receipt.basis.transaction_id.clone();
    report.receipt = Some(receipt);
    report
}

#[test]
fn bridge_recovery_refund_keeps_real_asset_fee_and_stops_original_cycle() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("refund.jsonl");
    let (store, run) = pending_bridge(&path);
    let report = received(&run);
    let result = store
        .record_bridge_recovery(&run.run_id, 2, report.clone(), 3000)
        .unwrap();
    assert_eq!(result.status, OnchainCrossChainRunStatus::Paused);
    assert!(result.legs[1].actual_output_amount_raw.is_none());
    assert_eq!(result.legs[1].source_receipt, run.legs[1].source_receipt);
    assert_eq!(result.legs[1].bridge_recovery, Some(report.clone()));
    let accounting = result.accounting.as_ref().unwrap();
    assert!(accounting.usd_value.is_none());
    let refund = accounting
        .flows
        .iter()
        .find(|flow| flow.kind == Kind::Recovery)
        .unwrap();
    assert_eq!(refund.change.asset.symbol, "USDC");
    assert_eq!(refund.change.chain, "ethereum");
    assert_eq!(refund.change.amount_exact, "98");
    assert_eq!(
        accounting
            .net_assets
            .iter()
            .find(|row| row.asset.symbol == "USDC")
            .unwrap()
            .amount_exact,
        "-2"
    );
    assert_eq!(
        accounting
            .flows
            .iter()
            .filter(|flow| flow.kind == Kind::NetworkFee)
            .count(),
        3
    );
    assert_eq!(result.legs[2].attempts, 0);
    let restored = OnchainCrossChainRunStore::load_path(Some(path), 4000);
    assert_eq!(restored.run(&run.run_id, 4000).unwrap(), result);
    let resumed = restored
        .request_recheck(&run.run_id, "tester", 2, 5000)
        .unwrap();
    let template = super::super::accounting::tests::fixture();
    assert!(restored
        .record_wallet_receipt(
            &run.run_id,
            2,
            template.legs[1].destination_receipt.clone().unwrap(),
            true,
            Some("99000000"),
            5100
        )
        .is_err());
    let repeated = restored
        .record_bridge_recovery(&run.run_id, 2, report, 5200)
        .unwrap();
    assert_eq!(repeated.accounting, result.accounting);
    assert_eq!(resumed.legs[1].attempts, 1);
}

#[test]
fn bridge_recovery_requires_original_identity_and_does_not_guess_refund_location() {
    let dir = tempfile::tempdir().unwrap();
    let (_, run) = pending_bridge(&dir.path().join("scope.jsonl"));
    let mutations: [fn(&mut OnchainCrossChainRecovery); 9] = [
        |r| r.provider_transaction_id = Some("other-transfer".into()),
        |r| r.sending_transaction_id = Some("0xwrong".into()),
        |r| r.sending_chain_id = Some(999),
        |r| r.receiving_chain_id = None,
        |r| r.receiving_chain_id = Some(999),
        |r| r.receiving_token_chain_id = None,
        |r| r.receiving_transaction_id = None,
        |r| r.receiving_token = None,
        |r| r.reported_receiver = Some("0xwrong-wallet".into()),
    ];
    for change in mutations {
        let mut report = refund_report(&run);
        change(&mut report);
        assert!(scope(&run, &run.legs[1], &report).is_err());
    }
    let mut report = refund_report(&run);
    report.token_resolution.as_mut().unwrap().decimals = 18;
    assert!(basis(&run, &run.legs[1], &report).is_err());
    report = refund_report(&run);
    report.receiving_transaction_id = run.legs[1].source_transaction_id.clone();
    assert!(scope(&run, &run.legs[1], &report)
        .unwrap_err()
        .contains("不重复"));
}

#[test]
fn bridge_recovery_partial_target_credit_is_not_assumed_to_be_original_token() {
    let dir = tempfile::tempdir().unwrap();
    let (_, run) = pending_bridge(&dir.path().join("partial.jsonl"));
    let mut report = refund_report(&run);
    let token = shared_types::onchain_known_token("base", "USDC").unwrap();
    report.provider_status = "DONE".into();
    report.substatus = Some("PARTIAL".into());
    report.receiving_chain_id = Some(8453);
    report.receiving_token_chain_id = Some(8453);
    report.receiving_token = Some(token.address.into());
    let resolution = report.token_resolution.as_mut().unwrap();
    resolution.chain = "base".into();
    resolution.address = token.address.into();
    let identity = resolution.identity.as_mut().unwrap();
    identity.chain = "base".into();
    identity.address = token.address.into();
    let basis = basis(&run, &run.legs[1], &report).unwrap();
    assert_eq!(basis.chain, "base");
    assert_eq!(basis.assets[0].symbol, "USDC");
    assert_ne!(
        basis.assets[0].address,
        run.legs[1].bridge_execution.as_ref().unwrap().to_token
    );
    assert!(!basis.require_sender);
}

#[test]
fn bridge_recovery_rpc_timeout_preserves_known_credit_then_confirms_missing_fees() {
    let dir = tempfile::tempdir().unwrap();
    let (store, run) = pending_bridge(&dir.path().join("partial-receipt.jsonl"));
    let complete = received(&run);
    let mut report = complete.clone();
    let receipt = report.receipt.as_mut().unwrap();
    receipt.status = ReceiptStatus::Pending;
    receipt.network_cost = None;
    receipt.problem = Some("fee unavailable".into());
    let first = store
        .record_bridge_recovery(&run.run_id, 2, report.clone(), 3000)
        .unwrap();
    assert_eq!(
        first.status,
        OnchainCrossChainRunStatus::AwaitingDestinationEvidence
    );
    let receipt = report.receipt.as_mut().unwrap();
    receipt.block_ref = None;
    receipt.observed_at_ms = None;
    receipt.asset_changes_raw = vec![None];
    receipt.additional_native_change_raw = None;
    receipt.problem = Some("RPC timeout".into());
    let retry = store
        .record_bridge_recovery(&run.run_id, 2, report, 4000)
        .unwrap();
    assert_eq!(
        retry.legs[1]
            .bridge_recovery
            .as_ref()
            .unwrap()
            .receipt
            .as_ref()
            .unwrap()
            .asset_changes_raw[0]
            .as_deref(),
        Some("98000000")
    );
    let done = store
        .record_bridge_recovery(&run.run_id, 2, complete, 5000)
        .unwrap();
    assert_eq!(done.status, OnchainCrossChainRunStatus::Paused);
    assert_eq!(
        done.accounting
            .as_ref()
            .unwrap()
            .flows
            .iter()
            .filter(|flow| flow.kind == Kind::Recovery)
            .count(),
        1
    );
}

#[test]
fn bridge_recovery_confirmed_receipt_cannot_be_overwritten_in_memory_or_journal() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("immutable.jsonl");
    let (store, run) = pending_bridge(&path);
    let report = received(&run);
    let confirmed = store
        .record_bridge_recovery(&run.run_id, 2, report.clone(), 3000)
        .unwrap();
    let resumed = store
        .request_recheck(&run.run_id, "tester", 2, 4000)
        .unwrap();
    let mut tampered = report;
    tampered.receipt.as_mut().unwrap().asset_changes_raw[0] = Some("1000000000".into());
    assert!(store
        .record_bridge_recovery(&run.run_id, 2, tampered.clone(), 5000)
        .is_err());
    assert_eq!(store.run(&run.run_id, 5000).unwrap(), resumed);
    let mut changed = resumed.clone();
    changed.legs[1].bridge_recovery = Some(tampered);
    append_jsonl(
        &path,
        &LogEntry {
            schema_version: SCHEMA_VERSION,
            build: None,
            run: Some(changed),
            recovery_plan: None,
        },
    )
    .unwrap();
    let restored = OnchainCrossChainRunStore::load_path(Some(path), 6000);
    assert!(restored.readiness().is_err());
    assert_eq!(
        restored.run(&run.run_id, 6000).unwrap().legs[1].bridge_recovery,
        confirmed.legs[1].bridge_recovery
    );
}
