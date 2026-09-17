use std::collections::BTreeSet;

use rust_decimal::{prelude::ToPrimitive, Decimal};
use shared_types::{OnchainComparisonConfig as Config, OnchainCrossChainBuildResponse as Build,
    OnchainCrossChainBuildRequest, OnchainCrossChainRun as Run, OnchainExecutionApprovalCost as Approval,
    OnchainExecutionCashFlow as Flow, OnchainUnsignedTransaction as Tx};

use super::{approval_allocation, replenishment_allocation, execution_submit::accounting, usd_valuation};
use crate::{state::AppState, services::onchain_execution_run_store::CrossChainCostClaim};

pub(super) fn resolve(state: &AppState, source: &Config, peer: &Config, request: &OnchainCrossChainBuildRequest, build: &mut Build, now: i64) -> Result<(), String> {
    build.approval_costs = approval_allocation::resolve_records(state, source, &request.approval_run_ids, now)?;
    for cost in &build.approval_costs { match_approval(build, cost)?; }
    let ids = &request.replenishment_run_ids;
    if ids.len() > 8 || ids.iter().collect::<BTreeSet<_>>().len() != ids.len() {
        return Err("补库费用最多选择 8 笔且不得重复".into());
    }
    if !ids.is_empty() { state.onchain_replenishment_plans().readiness()?; }
    for id in ids {
        let run = state.onchain_replenishment_plans().run(id, now).ok_or("补库记录不存在")?;
        let first = run.plan.legs.first().ok_or("补库步骤缺失")?;
        // A cross-chain cycle uses credited chain inventory, not a deposit into a CEX account.
        if run.plan.legs.iter().any(|leg| leg.direction != shared_types::OnchainTransferDirection::WithdrawToChain) {
            return Err("该补库记录包含 CEX 充值，不能整笔归入跨链钱包补库".into());
        }
        let mut config = [source, peer].into_iter().find(|c| c.chain.eq_ignore_ascii_case(&first.chain))
            .ok_or("补库不属于本次源链或目标链")?.clone();
        config.cex_venue = first.venue.clone();
        let mut cost = replenishment_allocation::from_run(&run, &config, run.plan.direction, now)?;
        let assets = cost.fees.iter().filter(|f| f.amount_exact != "0").map(|f| &f.asset).collect::<BTreeSet<_>>();
        let rates = assets.into_iter().map(|a| usd_valuation::evidence(state, source, a, now)).collect::<Result<Vec<_>,_>>()?;
        cost.build_valuation = accounting::value_flows(&cost.fees, rates, now)?;
        build.replenishment_costs.push(cost);
    }
    state.onchain_execution_run_store().check_replenishment_available(&build.replenishment_costs)?;
    apply_build_costs(source, build, now)
}

pub(crate) fn match_approval(build: &Build, cost: &Approval) -> Result<(), String> {
    let plan = &cost.plan;
    let same = |a: &str, b: &str| !a.is_empty() && a.eq_ignore_ascii_case(b);
    let matched = build.legs.iter().any(|leg| {
        if !same(&leg.from_chain, &plan.chain) || !same(&leg.from_token, &plan.token_address)
            || leg.input_decimals != plan.token_decimals { return false; }
        let tx = build.swap_executions.iter().find(|s| s.position == leg.position).map(|s| &s.transaction)
            .or_else(|| build.bridge_executions.iter().find(|b| b.position == leg.position).map(|b| &b.transaction));
        matches!(tx, Some(Tx::EvmCall {chain_id, from, allowance_spender:Some(spender), ..})
            if shared_types::onchain_chain_preset(&plan.chain).and_then(|p| p.chain_id) == Some(*chain_id)
                && same(from, &plan.wallet_address) && same(spender, &plan.spender))
    });
    if !matched { return Err("授权费用的链、钱包、输入合约或 spender 不属于本次四步合同".into()); }
    Ok(())
}

pub(crate) fn flows(build: &Build) -> Result<Vec<Flow>, String> {
    let mut keys = BTreeSet::new();
    for cost in &build.approval_costs {
        match_approval(build, cost)?;
        for key in approval_allocation::keys(cost) {
            if !keys.insert(key) { return Err("跨链授权费用重复".into()); }
        }
    }
    for cost in &build.replenishment_costs {
        for id in &cost.transfer_ids {
            if !keys.insert(format!("transfer:{id}")) { return Err("同一交易的独立费用重复归集".into()); }
        }
    }
    let mut result = approval_allocation::fees(&build.approval_costs)?;
    result.extend(replenishment_allocation::fees(&build.replenishment_costs)?);
    Ok(result)
}

pub(crate) fn total_usd(build: &Build) -> Result<f64, String> {
    flows(build)?;
    let result = approval_allocation::total_usd(&build.approval_costs)? + replenishment_allocation::total_usd(&build.replenishment_costs)?;
    if !result.is_finite() { return Err("独立费用合计溢出".into()); }
    Ok(result)
}

pub(super) fn apply_build_costs(config: &Config, build: &mut Build, now: i64) -> Result<(), String> {
    if build.approval_costs.is_empty() && build.replenishment_costs.is_empty() { return Ok(()); }
    let costs = total_usd(build)?;
    let valuation = super::cross_chain::economics::Valuation::read(config, build.quote_usd_valuation.as_ref(), config.max_age_ms, now)?;
    let raw = valuation.gas_raw(costs, config.quote_decimals)?;
    let initial = build.initial_quote_amount_raw.parse::<u128>().ok().filter(|n| *n > 0).ok_or("跨链初始金额未知")?;
    let bps = raw as f64 / initial as f64 * 10_000.0;
    let net = build.net_return_bps.filter(|n| n.is_finite()).ok_or("跨链净差未知")? - bps;
    let total = build.total_cost_bps.filter(|n| n.is_finite()).ok_or("跨链成本未知")? + bps;
    if !net.is_finite() || !total.is_finite() { return Err("独立费用折算溢出".into()); }
    build.net_return_bps = Some(net);
    build.total_cost_bps = Some(total);
    if net <= 0.0 || net < config.spread_alert.min_net_spread_bps {
        build.blockers.push("扣除已选授权与补库实付费用后，净收益不满足要求".into());
        build.submit_ready = false;
    }
    for rate in build.approval_costs.iter().flat_map(|c| &c.build_valuation.rates)
        .chain(build.replenishment_costs.iter().flat_map(|c| &c.build_valuation.rates)) {
        build.valid_until_ms = build.valid_until_ms.min(rate.observed_at_ms.saturating_add(config.max_age_ms));
    }
    Ok(())
}

pub(super) fn current_total_usd(state: &AppState, config: &Config, build: &Build, now: i64) -> Result<f64, String> {
    let fees = flows(build)?;
    if fees.is_empty() { return Ok(0.0); }
    let assets = fees.iter().map(|f| &f.asset).collect::<BTreeSet<_>>();
    let rates = assets.into_iter().map(|a| usd_valuation::evidence(state, config, a, now)).collect::<Result<Vec<_>,_>>()?;
    let value = accounting::value_flows(&fees, rates, now)?;
    let cost = -value.net_usd_exact.parse::<Decimal>().map_err(|_| "独立费用折算无效")?;
    cost.to_f64().filter(|v| v.is_finite() && *v >= 0.0 && (*v > 0.0 || cost == Decimal::ZERO)).ok_or_else(|| "独立费用折算溢出".into())
}

pub(crate) fn claim(run: &Run) -> Result<CrossChainCostClaim, String> {
    flows(&run.build)?;
    Ok(CrossChainCostClaim {
        run_id: run.run_id.clone(), build_id: run.build.build_id.clone(),
        approval_costs: run.build.approval_costs.clone(), replenishment_costs: run.build.replenishment_costs.clone(),
    })
}

pub(super) fn ensure_claimed(state: &AppState, run: &Run) -> Result<(), String> {
    if run.build.approval_costs.is_empty() && run.build.replenishment_costs.is_empty() { return Ok(()); }
    state.onchain_execution_run_store().claim_cross_chain_costs(claim(run)?)
}

#[cfg(test)]
pub(crate) mod tests;
