use super::*;
use serde_json::json;
use shared_types::{OnchainCrossChainLegKind as LegKind, OnchainWalletReceiptBasis};

pub(crate) fn fixture() -> OnchainCrossChainRun {
    let wallet = format!("0x{:040x}", 1);
    let other = format!("0x{:040x}", 2);
    let usdc = shared_types::onchain_known_token("ethereum", "USDC").unwrap();
    let peer_usdc = shared_types::onchain_known_token("base", "USDC").unwrap();
    let token = format!("0x{:040x}", 3);
    let peer_token = format!("0x{:040x}", 4);
    let mut run: OnchainCrossChainRun = serde_json::from_value(json!({
        "runId":"four-leg","idempotencyKey":"accounting-fixture","status":"completed",
        "authorization":{"actor":"tester","authorizedAtMs":100,"validUntilMs":10000,"confirmationVersion":"fixture"},
        "createdAtMs":100,"updatedAtMs":2000,"nextAction":"资产路径已完成",
        "build":{"buildId":"four-leg-build","provider":"fixture","sourceChain":"ethereum","peerChain":"base","legs":[],
            "initialQuoteAmountRaw":"100000000","finalQuoteAmountRaw":"110000000","bridgeFeeUsd":999,"gasUsd":999,
            "quoteObservedAtMs":100,"builtAtMs":100,"validUntilMs":10000,
            "atomic":false,"monitorOnly":false,"previewReady":true,"submitReady":true},"legs":[]
    })).unwrap();
    for (
        index,
        (kind, from_chain, to_chain, from_token, to_token, from_symbol, to_symbol, input, output),
    ) in [
        (
            LegKind::SourceSwap,
            "ethereum",
            "ethereum",
            usdc.address,
            token.as_str(),
            "USDC",
            "TKN",
            "100000000",
            "100000000",
        ),
        (
            LegKind::OutboundBridge,
            "ethereum",
            "base",
            token.as_str(),
            peer_token.as_str(),
            "TKN",
            "TKN",
            "100000000",
            "99000000",
        ),
        (
            LegKind::TargetSwap,
            "base",
            "base",
            peer_token.as_str(),
            peer_usdc.address,
            "TKN",
            "USDC",
            "99000000",
            "105000000",
        ),
        (
            LegKind::ReturnBridge,
            "base",
            "ethereum",
            peer_usdc.address,
            usdc.address,
            "USDC",
            "USDC",
            "105000000",
            "104000000",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let position = index as u8 + 1;
        let bridge = matches!(kind, LegKind::OutboundBridge | LegKind::ReturnBridge);
        let source_hash = format!("0x{:064x}", position * 10);
        let dest_hash = if bridge {
            format!("0x{:064x}", position * 10 + 1)
        } else {
            source_hash.clone()
        };
        let from_id = shared_types::onchain_chain_preset(from_chain)
            .unwrap()
            .chain_id
            .unwrap();
        let to_id = shared_types::onchain_chain_preset(to_chain)
            .unwrap()
            .chain_id
            .unwrap();
        run.build.legs.push(serde_json::from_value(json!({
            "position":position,"kind":kind,"provider":"fixture","fromChain":from_chain,"toChain":to_chain,
            "fromAsset":from_symbol,"toAsset":to_symbol,"fromToken":from_token,"toToken":to_token,
            "inputAmountRaw":input,"expectedOutputAmountRaw":output,"minimumOutputAmountRaw":output,
            "inputDecimals":6,"outputDecimals":6,"officialDocsUrl":"https://docs.li.fi","observedAtMs":100
        })).unwrap());
        let transaction = json!({"kind":"evm_call","chain_id":from_id,"from":wallet,"to":other,"data":"0x1234","value":"0x0","gas":"0x5208"});
        let execution = if bridge {
            json!({"position":position,"kind":kind,"provider":"fixture","routeId":"fixture","transactionId":format!("provider-{position}"),
            "tool":"fixture","fromChainId":from_id,"toChainId":to_id,"fromAddress":wallet,"toAddress":wallet,
            "fromToken":from_token,"toToken":to_token,"fromAmountRaw":input,"toAmountMinRaw":output,
            "quoteObservedAtMs":100,"validUntilMs":10000,"rebuildAfterPosition":position - 1,"officialDocsUrl":"https://docs.li.fi","transaction":transaction})
        } else {
            json!({"executionId":format!("provider-{position}"),"position":position,"kind":kind,"provider":"fixture","chain":from_chain,
            "walletAddress":wallet,"inputToken":from_token,"outputToken":to_token,"inputAmountRaw":input,"quotedOutputAmountRaw":output,
            "minimumOutputAmountRaw":output,"quoteObservedAtMs":100,"validUntilMs":10000,"rebuildAfterPosition":position - 1,
            "officialDocsUrl":"https://docs.li.fi","transaction":transaction})
        };
        let mut leg: OnchainCrossChainLegProgress = serde_json::from_value(json!({
            "position":position,"kind":kind,"clientActionId":format!("action-{position}"),"status":"completed","attempts":1,
            "plannedInputAmountRaw":input,"submittedInputAmountRaw":input,"actualInputAmountRaw":input,"actualOutputAmountRaw":output,
            "minimumOutputAmountRaw":output,"sourceTransactionId":source_hash,"destinationTransactionId":dest_hash,
            "bridgeReportedOutputAmountRaw":if bridge { Some(output) } else { None }
        })).unwrap();
        if bridge {
            leg.bridge_execution = Some(serde_json::from_value(execution).unwrap());
        } else {
            leg.swap_execution = Some(serde_json::from_value(execution).unwrap());
        }
        let make_receipt = |basis: OnchainWalletReceiptBasis,
                            changes: Vec<Option<String>>,
                            sponsor: bool| {
            let chain = basis.chain.clone();
            let transaction_id = basis.transaction_id.clone();
            OnchainWalletReceipt { basis, status: ReceiptStatus::Complete, asset_changes_raw: changes,
                additional_native_change_raw: Some(if position == 1 {"-2000000000000"} else {"0"}.into()),
                network_cost: Some(serde_json::from_value(json!({"chain":chain,"transactionId":transaction_id,"blockRef":"0xblock",
                    "payer":if sponsor {&other} else {&wallet},"asset":"ETH","executionFeeExact":"0.00001","additionalFeeExact":"0",
                    "totalFeeExact":"0.00001","source":"receipt","observedAtMs":1000})).unwrap()),
                block_ref:Some("0xblock".into()),observed_at_ms:Some(1000),problem:None }
        };
        let mut changes = vec![Some(format!("-{input}"))];
        if !bridge {
            changes.push(Some(output.into()));
        }
        leg.source_receipt = Some(make_receipt(
            super::super::receipts::basis(&run, &leg, None).unwrap(),
            changes,
            false,
        ));
        if bridge {
            leg.destination_receipt = Some(make_receipt(
                super::super::receipts::basis(&run, &leg, Some(&dest_hash)).unwrap(),
                vec![Some(output.into())],
                true,
            ));
        }
        run.legs.push(leg);
    }
    run
}

fn rates(now: i64) -> Vec<OnchainUsdValuation> {
    [("ETH", 2000.0, 2001.0), ("USDC", 0.9, 0.91)]
        .into_iter()
        .map(|(asset, bid, ask)| OnchainUsdValuation {
            asset: asset.into(),
            venue: "kraken".into(),
            symbol: format!("{asset}/USD"),
            source: "ws_push".into(),
            usd_bid: bid,
            usd_ask: ask,
            observed_at_ms: now,
        })
        .collect()
}

fn completed_store(path: &std::path::Path) -> (OnchainCrossChainRunStore, OnchainCrossChainRun) {
    let store = OnchainCrossChainRunStore::load_path(Some(path.into()), 100);
    let template = fixture();
    store.insert_build(template.build.clone(), 100).unwrap();
    let run = store
        .authorize(
            &template.build.build_id,
            "four-leg-accounting",
            "tester",
            110,
        )
        .unwrap()
        .run;
    for leg in &template.legs {
        store
            .claim_leg(
                &run.run_id,
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
                &run.run_id,
                leg.source_transaction_id.clone().unwrap(),
                "fixture".into(),
                1501,
            )
            .unwrap();
        store
            .record_wallet_receipt(
                &run.run_id,
                leg.position,
                leg.source_receipt.clone().unwrap(),
                false,
                None,
                1502,
            )
            .unwrap();
        if let Some(receipt) = &leg.destination_receipt {
            store
                .record_wallet_receipt(
                    &run.run_id,
                    leg.position,
                    receipt.clone(),
                    true,
                    leg.bridge_reported_output_amount_raw.as_deref(),
                    1503,
                )
                .unwrap();
        }
    }
    let run = store.run(&run.run_id, 1600).unwrap();
    (store, run)
}

#[test]
fn cross_chain_accounting_four_legs_to_journal_and_usd_value_does_not_double_charge_bridge_or_sponsor(
) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let (store, run) = completed_store(&path);
    let a = run.accounting.as_ref().unwrap();
    assert_eq!(run.status, OnchainCrossChainRunStatus::Completed);
    assert_eq!(a.status, Status::PendingValuation, "{:?}", a.problems);
    assert_eq!(
        a.net_assets
            .iter()
            .find(|row| row.asset.symbol == "USDC")
            .unwrap()
            .amount_exact,
        "4"
    );
    assert_eq!(
        a.flows
            .iter()
            .filter(|flow| flow.kind == Kind::NetworkFee)
            .count(),
        4
    );
    assert_eq!(
        a.flows
            .iter()
            .filter(|flow| flow.kind == Kind::Bridge)
            .count(),
        4
    );
    assert_eq!(a.net_assets.len(), 3);
    let valued = store
        .record_accounting_value(&run.run_id, &a.flows, rates(2000), 2000)
        .unwrap();
    assert_eq!(
        valued
            .accounting
            .as_ref()
            .unwrap()
            .usd_value
            .as_ref()
            .unwrap()
            .net_usd_exact,
        "3.515958"
    );
    let before = std::fs::read(&path).unwrap();
    for _ in 0..3 {
        store.runs(10, 2100);
        store.run(&run.run_id, 2100);
        assert_eq!(
            store
                .record_accounting_value(&run.run_id, &a.flows, rates(2100), 2100)
                .unwrap(),
            valued
        );
    }
    assert_eq!(before, std::fs::read(&path).unwrap());
    let restored = OnchainCrossChainRunStore::load_path(Some(path), 2200)
        .run(&run.run_id, 2200)
        .unwrap();
    assert_eq!(restored, valued);
    if let Ok(path) = std::env::var("CROSSLINE_CROSS_CHAIN_ACCOUNTING_FIXTURE") {
        std::fs::write(path, serde_json::to_vec(&restored).unwrap()).unwrap();
    }
}

#[test]
fn cross_chain_accounting_missing_cost_and_unfinished_bridge_keep_partial_native_flows_without_final_profit(
) {
    for case in [
        "missing-fee",
        "in-flight",
        "duplicated",
        "amount",
        "provider",
        "wallet",
    ] {
        let mut run = fixture();
        match case {
            "missing-fee" => {
                run.legs[0]
                    .source_receipt
                    .as_mut()
                    .unwrap()
                    .network_cost
                    .as_mut()
                    .unwrap()
                    .total_fee_exact = None
            }
            "in-flight" => {
                run.status = OnchainCrossChainRunStatus::AwaitingDestinationEvidence;
                run.legs[3].destination_receipt = None;
            }
            "duplicated" => {
                let mut second = run.legs[0].clone();
                second.position = 3;
                run.legs[2] = second;
            }
            "amount" => run.legs[0].actual_input_amount_raw = Some("1".into()),
            "provider" => run.legs[1].bridge_reported_output_amount_raw = None,
            _ => {
                run.legs[3].bridge_execution.as_mut().unwrap().to_address = "other-wallet".into();
            }
        }
        project(&mut run);
        let a = run.accounting.unwrap();
        assert_eq!(a.status, Status::PendingReceipts, "{case}");
        assert!(a.usd_value.is_none(), "{case}");
        assert!(!a.problems.is_empty(), "{case}");
        assert!(!a.flows.is_empty(), "{case}");
    }
}

#[test]
fn cross_chain_accounting_only_values_exact_issuer_contracts_and_never_stablecoins_at_par() {
    let mut run = fixture();
    project(&mut run);
    let a = run.accounting.unwrap();
    for case in ["rest", "stale", "future", "peg", "wrong-market", "coverage"] {
        let mut rates = rates(2000);
        match case {
            "rest" => rates[0].source = "rest_baseline".into(),
            "stale" => rates[0].observed_at_ms = 0,
            "future" => rates[0].observed_at_ms = 32002,
            "peg" => rates[1].source = "same_currency".into(),
            "wrong-market" => rates[1].symbol = "USDT/USD".into(),
            _ => {
                rates.pop();
            }
        }
        assert!(
            value(&a, rates, if case == "stale" { 32001 } else { 2000 }).is_err(),
            "{case}"
        );
    }
    let mut collision = a.clone();
    let mut fake = collision
        .net_assets
        .iter()
        .find(|r| r.asset.symbol == "USDC")
        .unwrap()
        .clone();
    fake.asset.address = format!("0x{:040x}", 999);
    fake.amount_exact = "-4".into();
    collision.net_assets.push(fake);
    assert!(
        valuation_symbols(&collision).is_err(),
        "same symbol must not cancel a different contract"
    );
    let mut usdt = a.net_assets[0].asset.clone();
    usdt.symbol = "USDT".into();
    usdt.address = "0xdac17f958d2ee523a2206206994597c13d831ec7".into();
    usdt.decimals = 6;
    assert_eq!(
        super::super::valuation_assets::symbol("ethereum", &usdt).unwrap(),
        "USDT"
    );
    usdt.symbol = "USD".into();
    assert!(super::super::valuation_assets::symbol("ethereum", &usdt).is_err());
}

#[test]
fn cross_chain_accounting_values_losses_and_preserves_leftover_contracts() {
    let mut run = fixture();
    let receipt = run.legs[0].source_receipt.as_mut().unwrap();
    let cost = receipt.network_cost.as_mut().unwrap();
    cost.execution_fee_exact = Some("1".into());
    cost.total_fee_exact = Some("1".into());
    project(&mut run);
    assert!(value(run.accounting.as_ref().unwrap(), rates(2000), 2000)
        .unwrap()
        .net_usd_exact
        .starts_with('-'));
    let mut leftover = fixture();
    let leg = &mut leftover.legs[1];
    leg.actual_input_amount_raw = Some("99000000".into());
    leg.source_receipt.as_mut().unwrap().asset_changes_raw[0] = Some("-99000000".into());
    project(&mut leftover);
    let a = leftover.accounting.unwrap();
    assert_eq!(a.status, Status::PendingValuation);
    assert_eq!(
        a.net_assets
            .iter()
            .find(|r| r.asset.symbol == "TKN")
            .unwrap()
            .amount_exact,
        "1"
    );
    assert!(valuation_symbols(&a).is_err());
}

#[test]
fn cross_chain_accounting_journal_failure_and_stale_projection_do_not_publish_a_value() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("runs.jsonl");
    let (store, run) = completed_store(&path);
    let a = run.accounting.as_ref().unwrap();
    assert!(store
        .record_accounting_value(&run.run_id, &[], rates(2000), 2000)
        .is_err());
    store
        .record_accounting_problem(&run.run_id, &a.flows, "waiting for WS".into(), 2000)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    store
        .record_accounting_problem(&run.run_id, &a.flows, "waiting for WS".into(), 2100)
        .unwrap();
    assert_eq!(before, std::fs::read(&path).unwrap());
    let snapshot = store.run(&run.run_id, 2200).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(store
        .record_accounting_value(&run.run_id, &a.flows, rates(2200), 2200)
        .is_err());
    assert_eq!(store.run(&run.run_id, 2200).unwrap(), snapshot);
}

#[test]
fn cross_chain_accounting_replay_recalculates_tampered_usd_and_bounded_refresh_stops() {
    let mut run = fixture();
    project(&mut run);
    let a = run.accounting.as_mut().unwrap();
    a.usd_value = Some(value(a, rates(2000), 2000).unwrap());
    a.status = Status::Valued;
    project(&mut run);
    assert_eq!(run.accounting.as_ref().unwrap().status, Status::Valued);
    run.accounting
        .as_mut()
        .unwrap()
        .usd_value
        .as_mut()
        .unwrap()
        .net_usd_exact = "99999".into();
    project(&mut run);
    assert!(run.accounting.as_ref().unwrap().usd_value.is_none());
    assert!(run.accounting_refresh_due(1000));
    assert!(!run.accounting_refresh_due(999));
    assert!(!run.accounting_refresh_due(601001));
}
