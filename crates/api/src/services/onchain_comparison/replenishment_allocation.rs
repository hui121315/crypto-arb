use std::collections::BTreeSet;

use rust_decimal::{prelude::ToPrimitive, Decimal};
use shared_types::{
    OnchainComparisonConfig, OnchainComparisonDirection, OnchainExecutionCashFlow as Flow,
    OnchainExecutionCashFlowKind as Kind, OnchainExecutionReplenishmentCost as Cost,
    OnchainReplenishmentRun as Run, OnchainReplenishmentRunStatus as Status,
    OnchainReplenishmentTransferStatus as TransferStatus, OnchainTransferDirection as Direction,
};

use super::{execution_submit::accounting, usd_valuation};
use crate::state::AppState;

pub(super) fn resolve(
    state: &AppState,
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    ids: &[String],
    now: i64,
) -> Result<Vec<Cost>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    if ids.len() > 8 || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err("补库费用最多选择 8 笔，且不能重复选择".into());
    }
    state.onchain_replenishment_plans().readiness()?;
    let mut costs = Vec::new();
    for id in ids {
        let run = state
            .onchain_replenishment_plans()
            .run(id, now)
            .ok_or_else(|| format!("补库记录 {id} 不存在"))?;
        let mut cost = from_run(&run, config, direction, now)?;
        let assets = cost
            .fees
            .iter()
            .filter(|f| f.amount_exact != "0")
            .map(|f| &f.asset)
            .collect::<BTreeSet<_>>();
        let rates = assets
            .into_iter()
            .map(|asset| usd_valuation::evidence(state, config, asset, now))
            .collect::<Result<Vec<_>, _>>()?;
        cost.build_valuation = accounting::value_flows(&cost.fees, rates, now)?;
        costs.push(cost);
    }
    fees(&costs)?;
    state
        .onchain_execution_run_store()
        .check_replenishment_available(&costs)?;
    Ok(costs)
}

pub(super) fn from_run(
    run: &Run,
    config: &OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    now: i64,
) -> Result<Cost, String> {
    if run.status != Status::Completed
        || run.plan.direction != direction
        || run.plan.legs.is_empty()
        || run.transfers.len() != run.plan.legs.len()
        || run.problem.is_some()
        || run.run_id.trim().is_empty()
        || run.plan.plan_id.trim().is_empty()
        || run.updated_at_ms <= 0
        || run.updated_at_ms > now
    {
        return Err("补库尚未完整到账，或不属于当前套利方向，不能归集费用".into());
    }
    let mut cost = Cost {
        run_id: run.run_id.clone(),
        plan_id: run.plan.plan_id.clone(),
        completed_at_ms: run.updated_at_ms,
        transfer_ids: Vec::new(),
        fees: Vec::new(),
        evidence_sources: Vec::new(),
        build_valuation: shared_types::OnchainExecutionUsdValue {
            net_usd_exact: "0".into(),
            valued_at_ms: now,
            rates: Vec::new(),
        },
    };
    for (index, leg) in run.plan.legs.iter().enumerate() {
        let transfer = &run.transfers[index];
        let (mint, decimals) = if leg.asset.eq_ignore_ascii_case(&config.base_token) {
            (&config.base_mint, config.base_decimals)
        } else if leg.asset.eq_ignore_ascii_case(&config.quote_token) {
            (&config.quote_mint, config.quote_decimals)
        } else {
            return Err("补库资产不属于当前链上交易对".into());
        };
        let wallet = match leg.direction {
            Direction::WithdrawToChain => leg.destination.address.as_deref(),
            Direction::DepositToCex => leg.source_address.as_deref(),
        };
        if !shared_types::venue_names_equal(&leg.venue, &config.cex_venue)
            || !leg.chain.eq_ignore_ascii_case(&config.chain)
            || wallet.is_none_or(|a| !same_address(&config.chain, a, &config.wallet_address))
            || leg
                .asset_address
                .as_ref()
                .is_none_or(|a| !same_address(&config.chain, a, mint))
            || leg.asset_decimals != Some(decimals)
            || leg
                .network_evidence
                .network
                .as_deref()
                .is_none_or(|n| n.trim().is_empty())
            || leg.asset.eq_ignore_ascii_case("USD")
            || transfer.leg_index as usize != index
            || transfer.status != TransferStatus::DestinationCredited
            || transfer.problem.is_some()
            || transfer.withdrawal_unlocked == Some(false)
        {
            return Err("补库场所、链、钱包、合约、精度或到账状态与当前执行不一致".into());
        }
        let sent = number(leg.transfer_amount_exact.as_deref())?;
        let credited = number(transfer.credited_amount_exact.as_deref())?;
        if sent <= Decimal::ZERO || credited != sent {
            return Err("补库到账量与已确认转账量不一致，扣费或额外入账需先核清".into());
        }
        let tx = transfer
            .transaction_id
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or("补库缺少链上交易编号")?;
        let chain_id = format!(
            "chain:{}:{}",
            leg.chain.to_ascii_lowercase(),
            if leg.chain.eq_ignore_ascii_case("solana") {
                tx.to_owned()
            } else {
                tx.to_ascii_lowercase()
            }
        );
        cost.transfer_ids.push(chain_id.clone());
        cost.evidence_sources.push(
            transfer
                .evidence_source
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or("补库到账来源缺失")?,
        );
        match leg.direction {
            Direction::WithdrawToChain => {
                let receipt = transfer
                    .withdrawal_cost
                    .as_ref()
                    .filter(|r| {
                        r.confirmed
                            && r.asset.eq_ignore_ascii_case(&leg.asset)
                            && !r.source.is_empty()
                            && r.observed_at_ms > 0
                            && r.observed_at_ms <= now
                    })
                    .ok_or("实扣提币费未确认，不能用预计费用代替")?;
                number(Some(&receipt.reported_amount_exact))?;
                let id = transfer
                    .provider_transfer_id
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .ok_or("提币编号缺失")?;
                let id = format!("cex:{}:withdrawal:{id}", leg.venue.to_ascii_lowercase());
                cost.transfer_ids.push(id.clone());
                charge(
                    &mut cost,
                    &format!("cex:{}", leg.venue.to_ascii_lowercase()),
                    &id,
                    &leg.asset,
                    Some(&receipt.fee_exact),
                )?;
                cost.evidence_sources.push(receipt.source.clone());
                // The venue paid its withdrawal transaction gas; do not debit it from the user again.
            }
            Direction::DepositToCex => {
                let fee = number(transfer.deposit_fee_exact.as_deref())?;
                if fee != Decimal::ZERO
                    || number(transfer.reported_deposit_amount_exact.as_deref())? != credited
                {
                    return Err("充值费与净到账口径未核清，不能确认完整补库费用".into());
                }
                charge(
                    &mut cost,
                    &format!("cex:{}", leg.venue.to_ascii_lowercase()),
                    &format!("{chain_id}:deposit"),
                    &leg.asset,
                    Some("0"),
                )?;
                let receipt = transfer.network_cost.as_ref().ok_or("补库实扣链费缺失")?;
                let native =
                    shared_types::onchain_chain_preset(&leg.chain).ok_or("链上原生币未知")?;
                let execution = number(receipt.execution_fee_exact.as_deref())?;
                let additional = number(receipt.additional_fee_exact.as_deref())?;
                let total = number(receipt.total_fee_exact.as_deref())?;
                if !receipt.chain.eq_ignore_ascii_case(&leg.chain)
                    || !same_address(&leg.chain, &receipt.transaction_id, tx)
                    || !same_address(&leg.chain, &receipt.payer, &config.wallet_address)
                    || !receipt.asset.eq_ignore_ascii_case(native.base_token)
                    || receipt.block_ref.is_empty()
                    || receipt.source.is_empty()
                    || receipt.problem.is_some()
                    || receipt.observed_at_ms <= 0
                    || receipt.observed_at_ms > now
                    || execution.checked_add(additional) != Some(total)
                {
                    return Err("补库链费的交易、付款钱包、原生币或分项合计未核清".into());
                }
                charge(
                    &mut cost,
                    &format!(
                        "chain:{}:{}",
                        leg.chain.to_ascii_lowercase(),
                        config.wallet_address
                    ),
                    &chain_id,
                    &receipt.asset,
                    receipt.total_fee_exact.as_deref(),
                )?;
                cost.evidence_sources.push(receipt.source.clone());
            }
        }
    }
    Ok(cost)
}

fn charge(
    cost: &mut Cost,
    location: &str,
    id: &str,
    asset: &str,
    amount: Option<&str>,
) -> Result<(), String> {
    let fee = number(amount)?;
    cost.fees.push(Flow {
        location: location.into(),
        source_id: id.into(),
        asset: asset.trim().to_ascii_uppercase(),
        amount_exact: (-fee).normalize().to_string(),
        kind: Kind::ReplenishmentFee,
    });
    Ok(())
}

fn number(value: Option<&str>) -> Result<Decimal, String> {
    value
        .and_then(|s| s.parse::<Decimal>().ok())
        .filter(|n| *n >= Decimal::ZERO)
        .ok_or_else(|| "补库实际费用或数量缺失/无效，不能按零计算".into())
}

fn same_address(chain: &str, a: &str, b: &str) -> bool {
    !a.is_empty()
        && !b.is_empty()
        && if chain.eq_ignore_ascii_case("solana") {
            a == b
        } else {
            a.eq_ignore_ascii_case(b)
        }
}

/// Validates frozen server evidence both when replaying the journal and computing totals.
pub(crate) fn fees(costs: &[Cost]) -> Result<Vec<Flow>, String> {
    let mut runs = BTreeSet::new();
    let mut transfers = BTreeSet::new();
    let mut flow_ids = BTreeSet::new();
    let mut result = Vec::new();
    for cost in costs {
        if cost.run_id.is_empty()
            || cost.plan_id.is_empty()
            || !runs.insert(&cost.run_id)
            || cost.completed_at_ms <= 0
            || cost.completed_at_ms > cost.build_valuation.valued_at_ms
            || cost.transfer_ids.is_empty()
            || cost.fees.is_empty()
            || cost.evidence_sources.is_empty()
            || cost.evidence_sources.iter().any(|s| s.is_empty())
            || cost
                .transfer_ids
                .iter()
                .any(|s| s.is_empty() || !transfers.insert(s))
        {
            return Err("补库费用归属缺失或重复".into());
        }
        for fee in &cost.fees {
            if fee.kind != Kind::ReplenishmentFee
                || fee.asset.is_empty()
                || fee.location.is_empty()
                || fee.source_id.is_empty()
                || !flow_ids.insert((&fee.location, &fee.source_id, &fee.asset))
                || fee
                    .amount_exact
                    .parse::<Decimal>()
                    .ok()
                    .is_none_or(|n| n > Decimal::ZERO)
            {
                return Err("补库费用明细无效或重复".into());
            }
        }
        if accounting::value_flows(
            &cost.fees,
            cost.build_valuation.rates.clone(),
            cost.build_valuation.valued_at_ms,
        )? != cost.build_valuation
        {
            return Err("补库费用与留存的美元折算不一致".into());
        }
        result.extend(cost.fees.iter().filter(|f| f.amount_exact != "0").cloned());
    }
    Ok(result)
}

pub(super) fn total_usd(costs: &[Cost]) -> Result<f64, String> {
    fees(costs)?;
    let mut total = Decimal::ZERO;
    for cost in costs {
        let value = cost
            .build_valuation
            .net_usd_exact
            .parse::<Decimal>()
            .map_err(|_| "补库费用折算无效")?;
        total = total.checked_sub(value).ok_or("补库费用合计溢出")?;
    }
    total
        .to_f64()
        .filter(|v| v.is_finite() && *v >= 0.0 && (*v > 0.0 || total == Decimal::ZERO))
        .ok_or_else(|| "补库费用合计超出支持精度".into())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn test_cost() -> Cost {
    tests::cost()
}
