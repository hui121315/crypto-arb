use super::super::recovery::tests::{pending_bridge, received};
use super::*;

#[test]
fn disposition_refund_uses_remaining_capital_not_net_loss_and_replays() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("disposition.jsonl");
    let (store, run) = pending_bridge(&path);
    let done = store
        .record_bridge_recovery(&run.run_id, 2, received(&run), 3000)
        .unwrap();
    let accounting = done.accounting.as_ref().unwrap();
    let plan = accounting.disposition.as_ref().unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    assert_eq!(plan.remaining_assets.len(), 1);
    assert_eq!(plan.remaining_assets[0].change.amount_exact, "98");
    assert_eq!(plan.remaining_assets[0].action, Action::Keep);
    assert_eq!(plan.original_capital.as_ref().unwrap().amount_exact, "100");
    assert!(!plan.submit_ready);
    assert!(plan.requires_live_authorization);
    assert!(accounting
        .net_assets
        .iter()
        .any(|asset| asset.amount_exact == "-2"));
    let restored = OnchainCrossChainRunStore::load_path(Some(path), 4000);
    assert_eq!(
        restored.run(&run.run_id, 4000).unwrap().accounting,
        done.accounting
    );
    // Projections from old or injected JSON are replaced by the verified journal receipts.
    let mut altered = done;
    altered
        .accounting
        .as_mut()
        .unwrap()
        .disposition
        .as_mut()
        .unwrap()
        .remaining_assets[0]
        .change
        .amount_exact = "99999".into();
    super::super::accounting::project(&mut altered);
    assert_eq!(
        altered
            .accounting
            .unwrap()
            .disposition
            .unwrap()
            .remaining_assets[0]
            .change
            .amount_exact,
        "98"
    );
}

#[test]
fn disposition_requires_pending_source_bridge_refund_and_fee_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut run) = pending_bridge(&dir.path().join("pending.jsonl"));
    run.status = OnchainCrossChainRunStatus::Paused;
    super::super::accounting::project(&mut run);
    let plan = run
        .accounting
        .as_ref()
        .unwrap()
        .disposition
        .as_ref()
        .unwrap();
    assert!(plan.remaining_assets.is_empty());
    assert!(plan.blockers.iter().any(|p| p.contains("在途")));
    let report = received(&run);
    run.legs[1].bridge_recovery = Some(report);
    run.legs[1].source_receipt.as_mut().unwrap().status = ReceiptStatus::Pending;
    super::super::accounting::project(&mut run);
    assert!(!run
        .accounting
        .as_ref()
        .unwrap()
        .disposition
        .as_ref()
        .unwrap()
        .blockers
        .is_empty());
    run.legs[1].source_receipt.as_mut().unwrap().status = ReceiptStatus::Complete;
    run.legs[1]
        .bridge_recovery
        .as_mut()
        .unwrap()
        .receipt
        .as_mut()
        .unwrap()
        .network_cost = None;
    super::super::accounting::project(&mut run);
    let plan = run
        .accounting
        .as_ref()
        .unwrap()
        .disposition
        .as_ref()
        .unwrap();
    assert!(plan.remaining_assets.is_empty());
    assert!(plan.blockers.iter().any(|p| p.contains("网络费")));
    run.legs[1].source_transaction_id = None;
    run.legs[1].status = OnchainCrossChainLegRunStatus::SubmissionClaimed;
    super::super::accounting::project(&mut run);
    assert!(run
        .accounting
        .unwrap()
        .disposition
        .unwrap()
        .blockers
        .iter()
        .any(|p| p.contains("提交结果未明")));
}

#[test]
fn disposition_partial_target_asset_requires_explicit_bridge_back() {
    let dir = tempfile::tempdir().unwrap();
    let (_, mut run) = pending_bridge(&dir.path().join("partial.jsonl"));
    let mut report = received(&run);
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
    let basis = super::super::recovery::basis(&run, &run.legs[1], &report).unwrap();
    let receipt = report.receipt.as_mut().unwrap();
    receipt.basis = basis;
    receipt.network_cost.as_mut().unwrap().chain = "base".into();
    run.legs[1].bridge_recovery = Some(report);
    run.status = OnchainCrossChainRunStatus::Paused;
    super::super::accounting::project(&mut run);
    let plan = run.accounting.unwrap().disposition.unwrap();
    assert!(plan.blockers.is_empty(), "{:?}", plan.blockers);
    assert_eq!(plan.remaining_assets.len(), 1);
    assert_eq!(plan.remaining_assets[0].change.chain, "base");
    assert_eq!(plan.remaining_assets[0].change.amount_exact, "98");
    assert_eq!(plan.remaining_assets[0].action, Action::QuoteBridge);
    assert_eq!(plan.original_capital.unwrap().chain, "ethereum");
}

fn capital(chain: &str, contract: &str) -> Change {
    Change {
        chain: chain.into(),
        wallet: "0xABC".into(),
        asset: shared_types::OnchainExecutionToken {
            symbol: "USDC".into(),
            address: contract.into(),
            decimals: 6,
        },
        amount_exact: "100".into(),
    }
}

fn flow(change: &Change, amount: &str, kind: Kind) -> Flow {
    let mut change = change.clone();
    change.amount_exact = amount.into();
    Flow {
        position: 1,
        transaction_id: "fixture".into(),
        kind,
        change,
    }
}

#[test]
fn disposition_deducts_later_spend_and_does_not_merge_quotes_or_wallets() {
    let capital = capital("ethereum", "0xUSDC");
    let mut usdt = capital.clone();
    usdt.asset.address = "0xUSDT".into();
    usdt.asset.symbol = "USDT".into();
    let mut other_wallet = usdt.clone();
    other_wallet.wallet = "0xother".into();
    let flows = vec![
        flow(&capital, "-100", Kind::Swap),
        flow(&usdt, "99", Kind::Swap),
        flow(&usdt, "-90", Kind::Bridge),
        flow(&other_wallet, "89", Kind::Recovery),
    ];
    let rows = remaining(&capital, &flows).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(rows
        .iter()
        .any(|r| r.change.amount_exact == "9" && r.action == Action::QuoteSwap));
    assert!(rows
        .iter()
        .any(|r| r.change.amount_exact == "89" && r.action == Action::ReviewWallet));
    let mut flows = flows;
    flows.push(flow(&usdt, "-10", Kind::Swap));
    assert!(remaining(&capital, &flows)
        .unwrap_err()
        .contains("收支为负"));
}

#[test]
fn disposition_counts_native_gas_once_and_rejects_precision_conflicts() {
    let mut capital = capital("ethereum", shared_types::EVM_NATIVE_TOKEN_ADDRESS);
    capital.asset.symbol = "ETH".into();
    capital.asset.decimals = 18;
    let flows = vec![
        flow(&capital, "-100", Kind::Swap),
        flow(&capital, "98", Kind::Recovery),
        flow(&capital, "-0.000021", Kind::NetworkFee),
    ];
    assert_eq!(
        remaining(&capital, &flows).unwrap()[0].change.amount_exact,
        "97.999979"
    );
    let mut altered = flows;
    altered[1].change.asset.decimals = 6;
    assert!(remaining(&capital, &altered)
        .unwrap_err()
        .contains("不同精度"));
}

#[test]
fn disposition_contract_identity_is_evm_case_insensitive_but_solana_exact() {
    let capital = capital("solana", "Abcd");
    let mut other = capital.clone();
    other.asset.address = "abcd".into();
    let rows = remaining(
        &capital,
        &[
            flow(&capital, "-100", Kind::Swap),
            flow(&other, "98", Kind::Recovery),
        ],
    )
    .unwrap();
    assert_eq!(rows[0].action, Action::QuoteSwap);
    let mut evm = capital.clone();
    evm.chain = "ethereum".into();
    let mut alias = evm.clone();
    alias.wallet = "0xabc".into();
    alias.asset.address = "abcd".into();
    alias.asset.symbol = "TOKEN".into();
    let rows = remaining(
        &evm,
        &[
            flow(&evm, "-100", Kind::Swap),
            flow(&alias, "98", Kind::Recovery),
        ],
    )
    .unwrap();
    assert_eq!(rows[0].action, Action::Keep);
    assert_eq!(rows[0].change.amount_exact, "98");
}
