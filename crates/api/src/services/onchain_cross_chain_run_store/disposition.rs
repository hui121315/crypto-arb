use super::*;
use rust_decimal::Decimal;
use shared_types::{
    OnchainChainSettlementStatus as ReceiptStatus, OnchainCrossChainAssetChange as Change,
    OnchainCrossChainCashFlow as Flow, OnchainCrossChainDisposition as Disposition,
    OnchainCrossChainDispositionAction as Action, OnchainCrossChainFlowKind as Kind,
    OnchainCrossChainRemainingAsset as Remaining,
};
use std::collections::BTreeMap;

pub(super) fn project(
    run: &OnchainCrossChainRun,
    flows: &[Flow],
    receipt_problems: &[String],
) -> Option<Disposition> {
    if !matches!(
        run.status,
        OnchainCrossChainRunStatus::Paused
            | OnchainCrossChainRunStatus::Failed
            | OnchainCrossChainRunStatus::Compensating
    ) && !run.legs.iter().any(|leg| leg.bridge_recovery.is_some())
    {
        return None;
    }
    let mut plan = Disposition {
        source_run_id: run.run_id.clone(),
        receipts_observed_at_ms: run
            .legs
            .iter()
            .flat_map(|leg| {
                [
                    leg.source_receipt.as_ref(),
                    leg.destination_receipt.as_ref(),
                    leg.bridge_recovery
                        .as_ref()
                        .and_then(|report| report.receipt.as_ref()),
                ]
            })
            .flatten()
            .filter_map(|receipt| receipt.observed_at_ms)
            .max(),
        original_capital: None,
        remaining_assets: vec![],
        blockers: receipt_problems.to_vec(),
        submit_ready: false,
        requires_live_authorization: true,
    };
    for leg in &run.legs {
        if leg.status == OnchainCrossChainLegRunStatus::SubmissionClaimed
            || leg.source_submitted_at_ms.is_some() && leg.source_transaction_id.is_none()
        {
            plan.blockers.push(format!(
                "第 {} 步提交结果未明，先核对原交易，不能重复处置",
                leg.position
            ));
        }
        if leg.source_transaction_id.is_some() {
            if !complete(leg.source_receipt.as_ref()) {
                plan.blockers
                    .push(format!("第 {} 步实际扣款与费用未核齐", leg.position));
            }
            if leg.bridge_execution.is_some()
                && !complete(leg.destination_receipt.as_ref())
                && !complete(
                    leg.bridge_recovery
                        .as_ref()
                        .and_then(|report| report.receipt.as_ref()),
                )
            {
                plan.blockers.push(format!(
                    "第 {} 步桥款仍可能在途，先核验到账或退款",
                    leg.position
                ));
            }
        }
    }
    match original_capital(run) {
        Ok(capital) => {
            if plan.blockers.is_empty() {
                match remaining(&capital, flows) {
                    Ok(assets) => plan.remaining_assets = assets,
                    Err(problem) => plan.blockers.push(problem),
                }
            }
            plan.original_capital = Some(capital);
        }
        Err(problem) => plan.blockers.push(problem),
    }
    plan.blockers.sort();
    plan.blockers.dedup();
    Some(plan)
}

fn complete(receipt: Option<&shared_types::OnchainWalletReceipt>) -> bool {
    receipt.is_some_and(|receipt| {
        receipt.status == ReceiptStatus::Complete && receipt.problem.is_none()
    })
}

fn original_capital(run: &OnchainCrossChainRun) -> Result<Change, String> {
    let first = run
        .legs
        .iter()
        .find(|leg| leg.position == 1)
        .ok_or("原资金路径缺失")?;
    let receipt = first
        .source_receipt
        .as_ref()
        .filter(|r| complete(Some(r)))
        .ok_or("首笔资金扣款尚未核实，不能推算本次剩余")?;
    let basis = super::receipts::basis(run, first, None)?;
    if basis != receipt.basis || first.kind != shared_types::OnchainCrossChainLegKind::SourceSwap {
        return Err("原资金身份与首笔交易合同不符".into());
    }
    let mut asset = basis.assets.first().cloned().ok_or("原资金代币缺失")?;
    let raw = &run.build.initial_quote_amount_raw;
    if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
        return Err("原投入数量不是有效最小单位".into());
    }
    let amount = super::accounting::raw_amount(raw, asset.decimals)?;
    let debit = receipt
        .asset_changes_raw
        .first()
        .and_then(Option::as_deref)
        .ok_or("首笔实际扣款缺失")?;
    let debit = super::accounting::raw_amount(debit, asset.decimals)?;
    if amount <= Decimal::ZERO || debit > Decimal::ZERO || -debit > amount {
        return Err("首笔扣款超过本次投入，不能把其他钱包资金计入剩余".into());
    }
    asset.address = super::accounting::address(&basis.chain, &asset.address);
    asset.symbol = asset.symbol.to_ascii_uppercase();
    Ok(Change {
        chain: basis.chain.clone(),
        wallet: super::accounting::address(&basis.chain, &basis.wallet),
        asset,
        amount_exact: amount.normalize().to_string(),
    })
}

fn remaining(capital: &Change, flows: &[Flow]) -> Result<Vec<Remaining>, String> {
    let parse = |value: &str| {
        Decimal::from_str_exact(value).map_err(|_| "剩余数量超出精确核算范围".to_owned())
    };
    let capital_key = super::accounting::key(capital);
    let mut rows = BTreeMap::new();
    rows.insert(
        capital_key.clone(),
        (capital.clone(), parse(&capital.amount_exact)?, true),
    );
    for flow in flows {
        let row_key = super::accounting::key(&flow.change);
        let (row, amount, trading_asset) = rows
            .entry(row_key)
            .or_insert_with(|| (flow.change.clone(), Decimal::ZERO, false));
        // Labels can change after metadata resolution; contract and precision define the asset.
        if row.asset.decimals != flow.change.asset.decimals {
            return Err("同一钱包合约出现不同精度，不能生成剩余资金建议".into());
        }
        let delta = parse(&flow.change.amount_exact)?;
        *amount = amount.checked_add(delta).ok_or("剩余资金累加溢出")?;
        *trading_asset |= matches!(flow.kind, Kind::Swap | Kind::Bridge | Kind::Recovery);
        if row.asset.symbol == "TOKEN" && flow.change.asset.symbol != "TOKEN" {
            row.asset.symbol = flow.change.asset.symbol.clone();
        }
    }
    let mut remaining = vec![];
    for (key, (mut row, amount, trading_asset)) in rows {
        // Gas paid from separate wallet reserves is a cost, not negative recoverable capital.
        if amount < Decimal::ZERO && trading_asset {
            return Err(format!(
                "{} 的本次资金收支为负，需先核对额外扣款",
                row.asset.symbol
            ));
        }
        if amount <= Decimal::ZERO {
            continue;
        }
        row.amount_exact = amount.normalize().to_string();
        let action = if key == capital_key {
            Action::Keep
        } else if row.chain != capital.chain {
            Action::QuoteBridge
        } else if key.1 != capital_key.1 {
            Action::ReviewWallet
        } else {
            Action::QuoteSwap
        };
        remaining.push(Remaining {
            change: row,
            action,
        });
    }
    Ok(remaining)
}

#[cfg(test)]
mod tests;
