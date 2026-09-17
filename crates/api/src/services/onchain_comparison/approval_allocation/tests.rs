use super::*;
use serde_json::json;

pub(crate) fn cost() -> Cost {
    let plan = journal::test_plan();
    let hash = format!("0x{:064x}", 10);
    let run = serde_json::from_value(json!({
        "runId":"approval-run-1", "approvalId":plan.approval_id, "status":"completed",
        "transactionIds":[hash], "message":"confirmed", "startedAtMs":1100, "updatedAtMs":7000,
        "feeReceipts":[{
            "basis":{"chain":plan.chain,"wallet":plan.wallet_address,"transactionId":hash,"requireSender":true,
                "assets":[{"symbol":plan.token_symbol,"address":plan.token_address,"decimals":plan.token_decimals}]},
            "status":"complete", "assetChangesRaw":["0"], "additionalNativeChangeRaw":"0",
            "blockRef":"0xcanonical", "observedAtMs":6000,
            "networkCost":{"chain":plan.chain,"transactionId":hash,"payer":plan.wallet_address,"asset":"ETH",
                "executionFeeExact":"0.000021","additionalFeeExact":"0","totalFeeExact":"0.000021",
                "blockRef":"0xcanonical","source":"eth_getTransactionReceipt","observedAtMs":6000}
        }]
    })).unwrap();
    from_receipt(plan, run)
}

pub(crate) fn from_receipt(
    plan: shared_types::OnchainTokenApprovalBuildResponse,
    run: shared_types::OnchainTokenApprovalSubmitResponse,
) -> Cost {
    let now = common::time::now_ms().max(run.updated_at_ms);
    let mut cost = Cost {
        plan,
        run,
        build_valuation: shared_types::OnchainExecutionUsdValue {
            net_usd_exact: "0".into(),
            valued_at_ms: now,
            rates: vec![],
        },
    };
    let flows = receipt_fees(&cost).unwrap();
    let mut rate = usd_valuation::fixture("ETH", 2000.0, now);
    rate.usd_ask = 2100.0;
    cost.build_valuation = accounting::value_flows(&flows, vec![rate], now).unwrap();
    cost
}

pub(crate) fn bind(
    checkpoint: &mut crate::services::onchain_execution_run_store::PendingOnchainExecution,
    cost: Cost,
) {
    checkpoint.config.chain = cost.plan.chain.clone();
    checkpoint.config.wallet_address = cost.plan.wallet_address.clone();
    checkpoint.config.quote_mint = cost.plan.token_address.clone();
    checkpoint.config.quote_decimals = cost.plan.token_decimals;
    checkpoint.config.provider = cost.plan.provider.clone();
    checkpoint.build.direction = cost.plan.direction;
    checkpoint.build.chain_transaction = OnchainUnsignedTransaction::EvmCall {
        chain_id: 1,
        from: cost.plan.wallet_address.clone(),
        to: cost.plan.spender.clone(),
        data: "0xswap".into(),
        value: "0".into(),
        gas: "100000".into(),
        gas_price: None,
        max_priority_fee_per_gas: None,
        allowance_spender: Some(cost.plan.spender.clone()),
    };
    checkpoint.response.approval_costs = vec![cost];
    checkpoint.build.approval_costs = checkpoint.response.approval_costs.clone();
}

#[test]
fn approval_allocation_actual_fee_uses_native_receipts_and_conservative_ws_ask() {
    let cost = cost();
    assert_eq!(fees(&[cost.clone()]).unwrap()[0].amount_exact, "-0.000021");
    assert_eq!(cost.build_valuation.net_usd_exact, "-0.0441");
    assert_eq!(total_usd(&[cost.clone()]).unwrap(), 0.0441);
    for fault in 0..12 {
        let mut bad = cost.clone();
        match fault {
            0 => bad.run.status = Status::AwaitingFinality,
            1 => {
                bad.run.fee_receipts[0]
                    .network_cost
                    .as_mut()
                    .unwrap()
                    .total_fee_exact = None
            }
            2 => {
                bad.run.fee_receipts[0]
                    .network_cost
                    .as_mut()
                    .unwrap()
                    .total_fee_exact = Some("0".into())
            }
            3 => bad.run.fee_receipts[0].basis.wallet = format!("0x{:040x}", 99),
            4 => bad.run.fee_receipts[0].network_cost.as_mut().unwrap().asset = "USD".into(),
            5 => bad
                .run
                .transaction_ids
                .push(bad.run.transaction_ids[0].clone()),
            6 => bad.run.fee_receipts[0].asset_changes_raw = vec![Some("1".into())],
            7 => bad.run.fee_receipts[0].additional_native_change_raw = Some("-1".into()),
            8 => bad.build_valuation.rates.clear(),
            9 => bad.build_valuation.net_usd_exact = "0".into(),
            10 => bad.run.fee_receipts[0]
                .network_cost
                .as_mut()
                .unwrap()
                .source
                .clear(),
            _ => bad.run.fee_receipts.push(bad.run.fee_receipts[0].clone()),
        }
        assert!(fees(&[bad]).is_err(), "fault {fault}");
    }
    assert!(fees(&[cost.clone(), cost]).is_err());
}

#[test]
fn approval_allocation_reverted_fee_is_not_lost_and_zero_requires_explicit_receipt() {
    let mut failed = cost();
    failed.run.status = Status::Failed;
    failed.run.fee_receipts[0].status = shared_types::OnchainChainSettlementStatus::ReviewRequired;
    failed.run.fee_receipts[0].problem = Some("transaction reverted".into());
    assert_eq!(total_usd(&[failed.clone()]).unwrap(), 0.0441);
    let network = failed.run.fee_receipts[0].network_cost.as_mut().unwrap();
    network.execution_fee_exact = Some("0".into());
    network.total_fee_exact = Some("0".into());
    failed.build_valuation.rates.clear();
    failed.build_valuation.net_usd_exact = "0".into();
    assert_eq!(total_usd(&[failed]).unwrap(), 0.0);
}

#[test]
fn approval_allocation_rejects_wrong_execution_identity_and_spender() {
    let cost = cost();
    let mut row = crate::services::onchain_execution_run_store::test_checkpoint();
    bind(&mut row, cost.clone());
    matches_execution(
        &cost,
        &row.config,
        row.build.direction,
        &row.build.chain_transaction,
    )
    .unwrap();
    for fault in 0..6 {
        let mut bad = row.clone();
        match fault {
            0 => bad.config.wallet_address = format!("0x{:040x}", 99),
            1 => bad.config.chain = "base".into(),
            2 => bad.config.quote_mint = format!("0x{:040x}", 99),
            3 => bad.config.quote_decimals = 18,
            4 => bad.build.direction = OnchainComparisonDirection::BuyCexSellOnchain,
            _ => {
                if let OnchainUnsignedTransaction::EvmCall {
                    allowance_spender, ..
                } = &mut bad.build.chain_transaction
                {
                    *allowance_spender = None;
                }
            }
        }
        assert!(
            matches_execution(
                &cost,
                &bad.config,
                bad.build.direction,
                &bad.build.chain_transaction
            )
            .is_err(),
            "fault {fault}"
        );
    }
}
