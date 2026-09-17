use super::*;
use rust_decimal::Decimal;
use shared_types::{
    OnchainChainSettlementStatus as ReceiptStatus, OnchainCrossChainAccounting as Accounting,
    OnchainCrossChainAssetChange as Change, OnchainCrossChainCashFlow as Flow,
    OnchainCrossChainFlowKind as Kind, OnchainExecutionAccountingStatus as Status,
    OnchainExecutionToken, OnchainExecutionUsdValue, OnchainUsdValuation, OnchainWalletReceipt,
};
use std::collections::{BTreeMap, BTreeSet};

pub(crate) fn project(run: &mut OnchainCrossChainRun) {
    let previous = run.accounting.take();
    let mut next = Accounting {
        status: Status::PendingReceipts,
        flows: vec![],
        external_flows: vec![],
        net_assets: vec![],
        usd_value: None,
        problems: vec![],
        disposition: None,
    };
    let mut seen = BTreeSet::new();
    for leg in &run.legs {
        for (destination, receipt) in [
            (false, leg.source_receipt.as_ref()),
            (true, leg.destination_receipt.as_ref()),
        ] {
            let Some(receipt) = receipt else {
                if !destination && leg.source_transaction_id.is_some()
                    || destination
                        && leg.bridge_execution.is_some()
                        && leg.destination_transaction_id.is_some()
                {
                    next.problems.push(format!(
                        "第 {} 腿{}回执未齐",
                        leg.position,
                        if destination { "目标链" } else { "源链" }
                    ));
                }
                continue;
            };
            match receipt_flows(run, leg, receipt, destination, &mut seen) {
                Ok((flows, problems)) => {
                    next.flows.extend(flows);
                    next.problems.extend(problems);
                }
                Err(problem) => next
                    .problems
                    .push(format!("第 {} 腿：{problem}", leg.position)),
            }
        }
        if let Some(recovery) = &leg.bridge_recovery {
            match recovery.receipt.as_ref().ok_or_else(|| "异常到账回执未齐".to_owned())
                .and_then(|receipt| {
                    let expected = super::recovery::basis(run, leg, recovery)?;
                    collect_receipt_flows(leg, receipt, expected, Kind::Recovery, &mut seen)
                }) {
                Ok((flows, problems)) => {
                    next.flows.extend(flows);
                    next.problems.extend(problems);
                }
                Err(problem) => next.problems.push(format!("第 {} 腿异常到账：{problem}", leg.position)),
            }
        }
    }
    next.disposition = super::disposition::project(run, &next.flows, &next.problems);
    match crate::services::onchain_comparison::cross_chain_costs::flows(&run.build) {
        Ok(flows) => {
            let duplicated = flows.iter().any(|f| f.source_id.strip_prefix("chain:")
                .and_then(|s| s.split_once(':')).is_some_and(|(chain,hash)| seen.iter()
                    .any(|(c,_,h)| c.eq_ignore_ascii_case(chain) && h.eq_ignore_ascii_case(hash))));
            if duplicated { next.problems.push("独立费用的交易已出现在四步回执中，禁止重复计费".into()); }
            else { next.external_flows = flows; }
        }
        Err(problem) => next.problems.push(problem),
    }
    match net(&next.flows) {
        Ok(rows) => next.net_assets = rows,
        Err(problem) => next.problems.push(problem),
    }
    if let Err(problem) = closed_cycle(run) {
        next.problems.push(problem);
    }
    next.problems.sort();
    next.problems.dedup();
    if next.problems.is_empty() {
        next.status = Status::PendingValuation;
        if let Some(old) = previous.as_ref().filter(|old| {
            old.status == Status::PendingValuation
                && old.flows == next.flows
                && old.external_flows == next.external_flows
                && old.net_assets == next.net_assets
        }) {
            next.problems = old.problems.clone();
        }
        if let Some(valuation) = previous
            .filter(|old| old.flows == next.flows && old.net_assets == next.net_assets && old.external_flows == next.external_flows)
            .and_then(|old| old.usd_value)
        {
            match value(&next, valuation.rates.clone(), valuation.valued_at_ms) {
                Ok(checked) if checked == valuation => {
                    next.status = Status::Valued;
                    next.usd_value = Some(checked);
                }
                _ => next
                    .problems
                    .push("已保存美元折算与回执不一致，已撤销该折算结果".into()),
            }
        }
    }
    run.accounting = Some(next);
}

pub(super) fn address(chain: &str, value: &str) -> String {
    if chain == "solana" {
        value.to_owned()
    } else if value == "0x0000000000000000000000000000000000000000" {
        shared_types::EVM_NATIVE_TOKEN_ADDRESS.to_ascii_lowercase()
    } else {
        value.to_ascii_lowercase()
    }
}

pub(super) fn key(row: &Change) -> (String, String, String) {
    (
        row.chain.clone(),
        address(&row.chain, &row.wallet),
        address(&row.chain, &row.asset.address),
    )
}

fn decimal(raw: &str) -> Result<Decimal, String> {
    Decimal::from_str_exact(raw).map_err(|_| "收支数量缺失或超出精确核算范围".into())
}

pub(super) fn raw_amount(raw: &str, decimals: u8) -> Result<Decimal, String> {
    if decimals > 28
        || raw.is_empty()
        || !raw
            .trim_start_matches('-')
            .bytes()
            .all(|b| b.is_ascii_digit())
    {
        return Err("回执最小单位或代币精度无效".into());
    }
    decimal(raw)?
        .checked_mul(Decimal::new(1, u32::from(decimals)))
        .ok_or("收支精度溢出".into())
}

fn push(
    flows: &mut Vec<Flow>,
    leg: &OnchainCrossChainLegProgress,
    receipt: &OnchainWalletReceipt,
    asset: &OnchainExecutionToken,
    amount: Decimal,
    kind: Kind,
) {
    if amount == Decimal::ZERO {
        return;
    }
    let mut asset = asset.clone();
    asset.symbol = asset.symbol.to_ascii_uppercase();
    asset.address = address(&receipt.basis.chain, &asset.address);
    flows.push(Flow {
        position: leg.position,
        transaction_id: receipt.basis.transaction_id.clone(),
        kind,
        change: Change {
            chain: receipt.basis.chain.clone(),
            wallet: address(&receipt.basis.chain, &receipt.basis.wallet),
            asset,
            amount_exact: amount.normalize().to_string(),
        },
    });
}

fn receipt_flows(
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
    receipt: &OnchainWalletReceipt,
    destination: bool,
    seen: &mut BTreeSet<(String, String, String)>,
) -> Result<(Vec<Flow>, Vec<String>), String> {
    let expected = super::receipts::basis(
        run,
        leg,
        destination.then_some(receipt.basis.transaction_id.as_str()),
    )?;
    if expected != receipt.basis
        || receipt.asset_changes_raw.len() != expected.assets.len()
        || destination
            && leg.destination_transaction_id.as_deref() != Some(expected.transaction_id.as_str())
    {
        return Err("回执的链、钱包、哈希或代币与执行合同不符".into());
    }
    let kind = if leg.bridge_execution.is_some() { Kind::Bridge } else { Kind::Swap };
    collect_receipt_flows(leg, receipt, expected, kind, seen)
}

fn collect_receipt_flows(
    leg: &OnchainCrossChainLegProgress,
    receipt: &OnchainWalletReceipt,
    expected: shared_types::OnchainWalletReceiptBasis,
    kind: Kind,
    seen: &mut BTreeSet<(String, String, String)>,
) -> Result<(Vec<Flow>, Vec<String>), String> {
    if receipt.basis != expected || receipt.asset_changes_raw.len() != expected.assets.len() {
        return Err("回执与已核验的链、钱包或合约不符".into());
    }
    if !seen.insert((
        expected.chain.clone(),
        address(&expected.chain, &expected.wallet),
        address(&expected.chain, &expected.transaction_id),
    )) {
        return Err("同一钱包的同一交易重复出现，禁止重复累计".into());
    }
    let mut flows = vec![];
    let mut problems = vec![];
    if receipt.block_ref.as_deref().is_none_or(str::is_empty) || receipt.observed_at_ms.is_none() {
        return Ok((
            flows,
            vec![format!("第 {} 腿链上回执尚未确认", leg.position)],
        ));
    }
    for (asset, raw) in expected.assets.iter().zip(&receipt.asset_changes_raw) {
        match raw.as_deref().map(|v| raw_amount(v, asset.decimals)) {
            Some(Ok(amount)) => push(&mut flows, leg, receipt, asset, amount, kind),
            _ => problems.push(format!(
                "第 {} 腿 {} 实际数量未齐",
                leg.position, asset.symbol
            )),
        }
    }
    let preset = shared_types::onchain_chain_preset(&expected.chain).ok_or("链身份未知")?;
    let native = OnchainExecutionToken {
        symbol: preset.base_token.into(),
        address: preset.base_address.into(),
        decimals: preset.base_decimals,
    };
    let native_in_pair = expected
        .assets
        .iter()
        .any(|a| address(&expected.chain, &a.address) == address(&expected.chain, &native.address));
    if let Some(raw) = &receipt.additional_native_change_raw {
        if native_in_pair && raw_amount(raw, native.decimals)? != Decimal::ZERO {
            return Err("原生币已在成交中，不能再次计入额外原生币变化".into());
        }
        push(
            &mut flows,
            leg,
            receipt,
            &native,
            raw_amount(raw, native.decimals)?,
            Kind::OtherNativeChange,
        );
    } else if !native_in_pair {
        problems.push(format!("第 {} 腿原生币额外收支未齐", leg.position));
    }
    let cost = (|| {
        let cost = receipt.network_cost.as_ref().ok_or("网络费缺失")?;
        if cost.chain != expected.chain
            || address(&expected.chain, &cost.transaction_id)
                != address(&expected.chain, &expected.transaction_id)
            || Some(cost.block_ref.as_str()) != receipt.block_ref.as_deref()
            || cost.payer.is_empty()
            || !cost.asset.eq_ignore_ascii_case(&native.symbol)
            || cost.problem.is_some()
        {
            return Err("网络费身份或证据无效".into());
        }
        let total = decimal(cost.total_fee_exact.as_deref().ok_or("网络总费缺失")?)?;
        let execution = decimal(cost.execution_fee_exact.as_deref().ok_or("执行链费缺失")?)?;
        let additional = decimal(cost.additional_fee_exact.as_deref().ok_or("附加链费缺失")?)?;
        if total < Decimal::ZERO
            || execution < Decimal::ZERO
            || additional < Decimal::ZERO
            || execution.checked_add(additional) != Some(total)
        {
            return Err("网络费分项与总额不一致".into());
        }
        if address(&expected.chain, &cost.payer) == address(&expected.chain, &expected.wallet) {
            push(&mut flows, leg, receipt, &native, -total, Kind::NetworkFee);
        }
        Ok::<(), String>(())
    })();
    if let Err(problem) = cost {
        problems.push(format!("第 {} 腿：{problem}", leg.position));
    }
    if receipt.status != ReceiptStatus::Complete {
        problems.push(format!("第 {} 腿收支仍待复核", leg.position));
    }
    Ok((flows, problems))
}

fn net(flows: &[Flow]) -> Result<Vec<Change>, String> {
    let mut assets = BTreeMap::<_, (Change, Decimal)>::new();
    for flow in flows {
        let (row, value) = assets
            .entry(key(&flow.change))
            .or_insert_with(|| (flow.change.clone(), Decimal::ZERO));
        if row.asset != flow.change.asset {
            return Err("相同合约的名称或精度矛盾，禁止合并".into());
        }
        *value = value
            .checked_add(decimal(&flow.change.amount_exact)?)
            .ok_or("净收支合计溢出")?;
    }
    Ok(assets
        .into_values()
        .filter(|(_, value)| *value != Decimal::ZERO)
        .map(|(mut row, value)| {
            row.amount_exact = value.normalize().to_string();
            row
        })
        .collect())
}

fn closed_cycle(run: &OnchainCrossChainRun) -> Result<(), String> {
    use shared_types::OnchainCrossChainLegKind::{
        OutboundBridge, ReturnBridge, SourceSwap, TargetSwap,
    };
    if run.status != OnchainCrossChainRunStatus::Completed
        || run.legs.len() != 4
        || run.build.legs.len() != 4
    {
        return Err("四腿路径尚未全部核清；以下仅为已确认的部分收支，不是最终盈亏".into());
    }
    let mut ends = vec![];
    for (index, (leg, kind)) in run
        .legs
        .iter()
        .zip([SourceSwap, OutboundBridge, TargetSwap, ReturnBridge])
        .enumerate()
    {
        if leg.position as usize != index + 1
            || leg.kind != kind
            || leg.status != OnchainCrossChainLegRunStatus::Completed
        {
            return Err("四腿执行顺序或终态不完整".into());
        }
        let source = leg
            .source_receipt
            .as_ref()
            .filter(|v| v.status == ReceiptStatus::Complete)
            .ok_or("源链回执未齐")?;
        let (destination, output_index) = if leg.bridge_execution.is_some() {
            (
                leg.destination_receipt
                    .as_ref()
                    .filter(|v| v.status == ReceiptStatus::Complete)
                    .ok_or("桥到账回执未齐")?,
                0,
            )
        } else {
            (source, 1)
        };
        let debit = source
            .asset_changes_raw
            .first()
            .and_then(|v| v.as_deref())
            .and_then(|v| v.parse::<i128>().ok())
            .ok_or("实际扣款缺失")?;
        let output = destination
            .asset_changes_raw
            .get(output_index)
            .and_then(|v| v.as_deref())
            .and_then(|v| v.parse::<i128>().ok())
            .ok_or("实际到账缺失")?;
        if debit >= 0
            || output <= 0
            || leg
                .actual_input_amount_raw
                .as_deref()
                .and_then(|v| v.parse::<u128>().ok())
                != Some(debit.unsigned_abs())
            || leg
                .actual_output_amount_raw
                .as_deref()
                .and_then(|v| v.parse::<i128>().ok())
                != Some(output)
            || leg
                .submitted_input_amount_raw
                .as_deref()
                .and_then(|v| v.parse::<u128>().ok())
                .is_none_or(|max| debit.unsigned_abs() > max)
            || leg
                .minimum_output_amount_raw
                .as_deref()
                .and_then(|v| v.parse::<i128>().ok())
                .is_none_or(|min| output < min)
        {
            return Err("回执金额与已确认收支或执行限额不一致".into());
        }
        if leg.bridge_execution.is_some()
            && leg
                .bridge_reported_output_amount_raw
                .as_deref()
                .and_then(|v| v.parse::<i128>().ok())
                != Some(output)
        {
            return Err("桥到账缺少与钱包一致的服务商数量证明".into());
        }
        let identity = |receipt: &OnchainWalletReceipt, index: usize| -> Result<_, String> {
            let token = receipt.basis.assets.get(index).ok_or("成交资产不完整")?;
            Ok((
                receipt.basis.chain.clone(),
                address(&receipt.basis.chain, &receipt.basis.wallet),
                address(&receipt.basis.chain, &token.address),
                token.decimals,
            ))
        };
        ends.push((identity(source, 0)?, identity(destination, output_index)?));
        if index > 0
            && run.legs[index - 1].actual_output_amount_raw != leg.submitted_input_amount_raw
        {
            return Err("后续投入与前一步真实到账不一致".into());
        }
    }
    for index in 0..4 {
        if ends[index].1 != ends[(index + 1) % 4].0 {
            return Err("资产未回到同一钱包及合约，不能确认为闭环".into());
        }
    }
    Ok(())
}

pub(crate) fn valuation_flows(
    accounting: &Accounting,
) -> Result<Vec<shared_types::OnchainExecutionCashFlow>, String> {
    if accounting.status == Status::PendingReceipts {
        return Err("收支回执尚未核清".into());
    }
    let mut flows = accounting
        .net_assets
        .iter()
        .map(|row| {
            let symbol = super::valuation_assets::symbol(&row.chain, &row.asset)?;
            Ok(shared_types::OnchainExecutionCashFlow {
                location: format!("chain:{}:{}", row.chain, row.wallet),
                source_id: row.asset.address.clone(),
                asset: symbol,
                amount_exact: row.amount_exact.clone(),
                kind: shared_types::OnchainExecutionCashFlowKind::Trade,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    flows.extend(accounting.external_flows.clone());
    Ok(flows)
}

pub(crate) fn value(
    accounting: &Accounting,
    rates: Vec<OnchainUsdValuation>,
    now_ms: i64,
) -> Result<OnchainExecutionUsdValue, String> {
    crate::services::onchain_comparison::value_execution_flows(
        &valuation_flows(accounting)?,
        rates,
        now_ms,
    )
}

pub(crate) fn valuation_symbols(accounting: &Accounting) -> Result<Vec<String>, String> {
    let mut net = BTreeMap::<String, Decimal>::new();
    for flow in valuation_flows(accounting)? {
        let amount = net.entry(flow.asset).or_default();
        *amount = amount
            .checked_add(decimal(&flow.amount_exact)?)
            .ok_or("折算合计溢出")?;
    }
    Ok(net
        .into_iter()
        .filter(|(_, amount)| *amount != Decimal::ZERO)
        .map(|(asset, _)| asset)
        .collect())
}

impl OnchainCrossChainRunStore {
    pub(crate) fn record_accounting_problem(
        &self,
        run_id: &str,
        expected_flows: &[Flow],
        problem: String,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(
            run_id,
            |run| {
                project(run);
                let accounting = run.accounting.as_mut().ok_or("收支核算缺失")?;
                if accounting.flows != expected_flows
                    || accounting.status != Status::PendingValuation
                {
                    return Err("收支或核算状态已变化，不覆盖当前结果".into());
                }
                accounting.problems = vec![problem];
                Ok(())
            },
            now_ms,
        )
    }
    pub(crate) fn record_accounting_value(
        &self,
        run_id: &str,
        expected_flows: &[Flow],
        rates: Vec<OnchainUsdValuation>,
        now_ms: i64,
    ) -> Result<OnchainCrossChainRun, String> {
        self.update_run(
            run_id,
            |run| {
                project(run);
                let accounting = run.accounting.as_mut().ok_or("收支核算缺失")?;
                if accounting.flows != expected_flows {
                    return Err("收支已变化，拒绝保存旧折算".into());
                }
                if accounting.status == Status::Valued {
                    return Ok(());
                }
                accounting.usd_value = Some(value(accounting, rates, now_ms)?);
                accounting.status = Status::Valued;
                accounting.problems.clear();
                Ok(())
            },
            now_ms,
        )
    }
}

#[cfg(test)]
pub(crate) mod tests;
