use std::collections::BTreeSet;

use rust_decimal::{prelude::ToPrimitive, Decimal};
use shared_types::{
    OnchainComparisonConfig, OnchainComparisonDirection, OnchainExecutionApprovalCost as Cost,
    OnchainExecutionCashFlow as Flow, OnchainExecutionCashFlowKind as Kind,
    OnchainTokenApprovalRunStatus as Status, OnchainUnsignedTransaction,
};

use super::{execution_submit::accounting, usd_valuation};
use crate::{services::onchain_token_approval_run_store as journal, state::AppState};

pub(super) fn resolve(
    state: &AppState,
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    transaction: &OnchainUnsignedTransaction,
    ids: &[String],
    now: i64,
) -> Result<Vec<Cost>, String> {
    let costs = resolve_records(state, config, ids, now)?;
    for cost in &costs { matches_execution(cost, config, direction, transaction)?; }
    Ok(costs)
}

pub(super) fn resolve_records(state: &AppState, config: &OnchainComparisonConfig, ids: &[String], now: i64) -> Result<Vec<Cost>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    if ids.len() > 8 || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err("授权费用最多选择 8 笔，且不能重复选择".into());
    }
    state.onchain_token_approval_runs().readiness()?;
    let mut costs = Vec::new();
    for id in ids {
        let record = state
            .onchain_token_approval_runs()
            .record(id)
            .ok_or_else(|| format!("授权记录 {id} 不存在"))?;
        let mut cost = Cost {
            plan: record.plan,
            run: record.response,
            build_valuation: shared_types::OnchainExecutionUsdValue {
                net_usd_exact: "0".into(),
                valued_at_ms: now,
                rates: Vec::new(),
            },
        };
        let flows = receipt_fees(&cost)?;
        let assets = flows
            .iter()
            .filter(|flow| flow.amount_exact != "0")
            .map(|flow| &flow.asset)
            .collect::<BTreeSet<_>>();
        let rates = assets
            .into_iter()
            .map(|asset| usd_valuation::evidence(state, config, asset, now))
            .collect::<Result<Vec<_>, _>>()?;
        cost.build_valuation = accounting::value_flows(&flows, rates, now)?;
        costs.push(cost);
    }
    fees(&costs)?;
    state
        .onchain_execution_run_store()
        .check_approval_available(&costs)?;
    Ok(costs)
}

pub(crate) fn matches_execution(
    cost: &Cost,
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    transaction: &OnchainUnsignedTransaction,
) -> Result<(), String> {
    let plan = &cost.plan;
    let Some(preset) = shared_types::onchain_chain_preset(&config.chain) else {
        return Err("授权费用所属链未知".into());
    };
    let (token, decimals) = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            (&config.quote_mint, config.quote_decimals)
        }
        OnchainComparisonDirection::BuyCexSellOnchain => (&config.base_mint, config.base_decimals),
    };
    let matches_call = matches!(transaction, OnchainUnsignedTransaction::EvmCall {
        chain_id, from, allowance_spender: Some(spender), ..
    } if Some(*chain_id) == preset.chain_id
        && from.eq_ignore_ascii_case(&plan.wallet_address)
        && spender.eq_ignore_ascii_case(&plan.spender));
    if !matches_call
        || plan.direction != direction
        || !plan.chain.eq_ignore_ascii_case(&config.chain)
        || !plan.provider.eq_ignore_ascii_case(&config.provider)
        || !plan
            .wallet_address
            .eq_ignore_ascii_case(&config.wallet_address)
        || !plan.token_address.eq_ignore_ascii_case(token)
        || plan.token_decimals != decimals
    {
        return Err("授权费用的链、钱包、代币、方向或 spender 不属于当前执行".into());
    }
    Ok(())
}

// Read the frozen actual receipts again on accounting and journal replay, not UI totals.
fn receipt_fees(cost: &Cost) -> Result<Vec<Flow>, String> {
    let (plan, run) = (&cost.plan, &cost.run);
    let now = cost.build_valuation.valued_at_ms;
    if run.run_id.is_empty()
        || plan.approval_id.is_empty()
        || run.approval_id != plan.approval_id
        || !matches!(run.status, Status::Completed | Status::Failed)
        || run.updated_at_ms <= 0
        || run.updated_at_ms > now
        || plan.transactions.is_empty()
        || plan.transactions.len() > 2
        || run.transaction_ids.is_empty()
        || run.transaction_ids.len() > plan.transactions.len()
        || (run.status == Status::Completed && run.transaction_ids.len() != plan.transactions.len())
        || run.fee_receipts.len() != run.transaction_ids.len()
    {
        return Err("授权尚有待确认步骤或费用回执不全，不能归集".into());
    }
    for index in 0..plan.transactions.len() {
        journal::approval_words(plan, index)?;
    }
    let mut hashes = BTreeSet::new();
    let mut flows = Vec::new();
    for hash in &run.transaction_ids {
        if hash.len() != 66
            || !hash.starts_with("0x")
            || !hash[2..].bytes().all(|b| b.is_ascii_hexdigit())
            || !hashes.insert(hash.to_ascii_lowercase())
        {
            return Err("授权交易编号无效或重复".into());
        }
        let receipts = run
            .fee_receipts
            .iter()
            .filter(|r| r.basis.transaction_id.eq_ignore_ascii_case(hash))
            .collect::<Vec<_>>();
        let [receipt] = receipts.as_slice() else {
            return Err("授权费用缺少唯一回执".into());
        };
        let expected = shared_types::OnchainWalletReceiptBasis {
            chain: plan.chain.clone(),
            wallet: plan.wallet_address.clone(),
            transaction_id: hash.clone(),
            require_sender: true,
            assets: vec![shared_types::OnchainExecutionToken {
                symbol: plan.token_symbol.clone(),
                address: plan.token_address.clone(),
                decimals: plan.token_decimals,
            }],
        };
        if receipt.basis != expected
            || !journal::complete_cost(receipt)
            || receipt.asset_changes_raw != [Some("0".into())]
            || receipt.additional_native_change_raw.as_deref() != Some("0")
            || receipt.observed_at_ms.is_none_or(|t| t <= 0 || t > now)
            || (run.status == Status::Completed
                && receipt.status != shared_types::OnchainChainSettlementStatus::Complete)
        {
            return Err("授权实付费用未核清，或存在额外资产变化".into());
        }
        let network = receipt.network_cost.as_ref().ok_or("授权链费缺失")?;
        if network.source.is_empty()
            || network.block_ref.is_empty()
            || network.observed_at_ms <= 0
            || network.observed_at_ms > now
        {
            return Err("授权链费缺少可追溯来源或核验时间".into());
        }
        let amount = network
            .total_fee_exact
            .as_deref()
            .ok_or("授权链费未知")?
            .parse::<Decimal>()
            .map_err(|_| "授权链费数值无效")?;
        flows.push(Flow {
            location: format!(
                "chain:{}:{}",
                plan.chain.to_ascii_lowercase(),
                plan.wallet_address.to_ascii_lowercase()
            ),
            source_id: format!(
                "chain:{}:{}",
                plan.chain.to_ascii_lowercase(),
                hash.to_ascii_lowercase()
            ),
            asset: network.asset.to_ascii_uppercase(),
            amount_exact: (-amount).normalize().to_string(),
            kind: Kind::ApprovalFee,
        });
    }
    Ok(flows)
}

pub(crate) fn fees(costs: &[Cost]) -> Result<Vec<Flow>, String> {
    let mut ids = BTreeSet::new();
    let mut result = Vec::new();
    for cost in costs {
        for key in keys(cost) {
            if !ids.insert(key) {
                return Err("授权费用归属重复".into());
            }
        }
        let flows = receipt_fees(cost)?;
        if accounting::value_flows(
            &flows,
            cost.build_valuation.rates.clone(),
            cost.build_valuation.valued_at_ms,
        )? != cost.build_valuation
        {
            return Err("授权实付费用与留存的美元折算不一致".into());
        }
        result.extend(flows.into_iter().filter(|flow| flow.amount_exact != "0"));
    }
    Ok(result)
}

pub(crate) fn keys(cost: &Cost) -> impl Iterator<Item = String> + '_ {
    std::iter::once(format!("approval-run:{}", cost.run.run_id))
        .chain(std::iter::once(format!(
            "approval-plan:{}",
            cost.plan.approval_id
        )))
        .chain(cost.run.transaction_ids.iter().map(|id| {
            format!(
                "transfer:chain:{}:{}",
                cost.plan.chain.to_ascii_lowercase(),
                id.to_ascii_lowercase()
            )
        }))
}

pub(super) fn total_usd(costs: &[Cost]) -> Result<f64, String> {
    fees(costs)?;
    let mut total = Decimal::ZERO;
    for cost in costs {
        let amount = cost
            .build_valuation
            .net_usd_exact
            .parse::<Decimal>()
            .map_err(|_| "授权费折算无效")?;
        total = total.checked_sub(amount).ok_or("授权费合计溢出")?;
    }
    total
        .to_f64()
        .filter(|v| v.is_finite() && *v >= 0.0 && (*v > 0.0 || total == Decimal::ZERO))
        .ok_or_else(|| "授权费合计超出支持精度".into())
}

#[cfg(test)]
pub(crate) mod tests;
