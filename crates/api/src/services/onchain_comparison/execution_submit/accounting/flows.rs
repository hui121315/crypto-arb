use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::Decimal;
use shared_types::{
    OnchainCexSettlementStatus, OnchainChainSettlementStatus, OnchainExecutionAssetChange,
    OnchainExecutionCashFlow as Flow, OnchainExecutionCashFlowKind as Kind,
    OnchainExecutionLegKind, OnchainExecutionLegResult, OnchainExecutionLegStatus as LegStatus,
    OnchainExecutionRunStatus, OnchainExecutionSubmitResponse, OrderSide,
};

pub(super) fn collect(run: &OnchainExecutionSubmitResponse) -> Result<Vec<Flow>, String> {
    if run.legs.is_empty()
        || matches!(
            run.status,
            OnchainExecutionRunStatus::Executing
                | OnchainExecutionRunStatus::AwaitingChainFinality
                | OnchainExecutionRunStatus::FinalityUnresolved
        )
    {
        return Err("执行腿或终态尚未齐全，不能计算成交净变动".into());
    }
    let mut flows = Vec::new();
    let mut seen = BTreeSet::new();
    for (kind, expected) in [
        (
            OnchainExecutionLegKind::PrimaryCex,
            run.cex_order_id.as_deref(),
        ),
        (
            OnchainExecutionLegKind::Chain,
            run.chain_transaction_id.as_deref(),
        ),
        (
            OnchainExecutionLegKind::Compensation,
            run.compensation_order_id.as_deref(),
        ),
    ] {
        if let Some(id) = expected {
            let count = run
                .legs
                .iter()
                .filter(|leg| {
                    leg.kind == kind
                        && leg.order_id.as_deref().or(leg.transaction_id.as_deref()) == Some(id)
                })
                .count();
            if count != 1 {
                return Err("本次执行引用的订单或链上交易明细不完整或重复".into());
            }
        }
    }
    for leg in &run.legs {
        if leg.kind == OnchainExecutionLegKind::Chain {
            chain(run, leg, &mut seen, &mut flows)?;
        } else {
            cex(run, leg, &mut seen, &mut flows)?;
        }
    }
    flows.extend(super::super::super::replenishment_allocation::fees(
        &run.replenishment_costs,
    )?);
    for fee in super::super::super::approval_allocation::fees(&run.approval_costs)? {
        if !seen.insert(fee.source_id.clone()) {
            return Err("授权交易已计入本次执行，不能重复扣链费".into());
        }
        flows.push(fee);
    }
    Ok(flows)
}

fn cex(
    run: &OnchainExecutionSubmitResponse,
    leg: &OnchainExecutionLegResult,
    seen: &mut BTreeSet<String>,
    flows: &mut Vec<Flow>,
) -> Result<(), String> {
    let id = leg
        .order_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .ok_or("CEX 订单号缺失")?;
    let location = format!("cex:{}", leg.venue.to_ascii_lowercase());
    if !seen.insert(format!("{location}:{id}")) {
        return Err("重复 CEX 订单不能重复计入收支".into());
    }
    if leg.kind == OnchainExecutionLegKind::PrimaryCex && run.cex_order_id.as_deref() != Some(id) {
        return Err("主订单收支与本次执行不匹配".into());
    }
    if leg.filled_quantity == Some(0.0)
        && matches!(
            leg.status,
            LegStatus::Cancelled | LegStatus::Rejected | LegStatus::Failed
        )
    {
        return Ok(());
    }
    let receipt = leg
        .settlement
        .as_ref()
        .filter(|row| {
            row.status == OnchainCexSettlementStatus::Complete
                && row.basis.order_id == id
                && row.basis.venue.eq_ignore_ascii_case(&leg.venue)
                && leg.symbol.as_deref() == Some(row.basis.symbol.as_str())
                && leg.filled_quantity == Some(row.basis.confirmed_quantity)
                && matches!(
                    leg.status,
                    LegStatus::Filled | LegStatus::Cancelled | LegStatus::Compensated
                )
        })
        .ok_or_else(|| format!("{} 订单 {} 的实际成交或手续费未核清", leg.venue, id))?;
    let base = asset(&receipt.basis.base_asset)?;
    let quote = asset(&receipt.basis.quote_asset)?;
    if base == quote || receipt.fill_event_ids.is_empty() {
        return Err("CEX 成交资产或逐笔编号缺失".into());
    }
    let gross_base = amount(receipt.gross_base_amount.as_deref())?;
    let gross_quote = amount(receipt.gross_quote_amount.as_deref())?;
    if gross_base <= Decimal::ZERO || gross_quote <= Decimal::ZERO {
        return Err("已成交 CEX 数量或金额无效".into());
    }
    let mut fees = BTreeMap::new();
    for fee in &receipt.fees {
        if fees
            .insert(asset(&fee.asset)?, amount(Some(&fee.amount))?)
            .is_some()
        {
            return Err("同一扣费资产重复出现，不能重复扣费".into());
        }
    }
    let (from, to, debit, credit) = match receipt.basis.side {
        OrderSide::Buy => (&quote, &base, gross_quote, gross_base),
        OrderSide::Sell => (&base, &quote, gross_base, gross_quote),
    };
    if debit.checked_add(*fees.get(from).unwrap_or(&Decimal::ZERO))
        != Some(amount(receipt.debit_amount.as_deref())?)
        || credit.checked_sub(*fees.get(to).unwrap_or(&Decimal::ZERO))
            != Some(amount(receipt.credit_amount.as_deref())?)
    {
        return Err("CEX 毛成交、净到账与实扣费用矛盾".into());
    }
    // Use gross cash legs plus each native fee once, never net legs plus the same fee again.
    push(flows, &location, id, from, -debit, Kind::Trade)?;
    push(flows, &location, id, to, credit, Kind::Trade)?;
    for (asset, fee) in fees {
        push(flows, &location, id, &asset, -fee, Kind::Fee)?;
    }
    Ok(())
}

fn chain(
    run: &OnchainExecutionSubmitResponse,
    leg: &OnchainExecutionLegResult,
    seen: &mut BTreeSet<String>,
    flows: &mut Vec<Flow>,
) -> Result<(), String> {
    let receipt = leg.chain_settlement.as_ref().ok_or("链上实际收支未取得")?;
    let basis = &receipt.basis;
    let is_solana = basis.chain.eq_ignore_ascii_case("solana");
    let same_address = |a: &str, b: &str| {
        if is_solana {
            a == b
        } else {
            a.eq_ignore_ascii_case(b)
        }
    };
    let id = basis.transaction_id.as_str();
    if id.is_empty()
        || basis.wallet.is_empty()
        || leg.transaction_id.as_deref() != Some(id)
        || run.chain_transaction_id.as_deref() != Some(id)
        || !leg.venue.eq_ignore_ascii_case(&basis.chain)
        || !seen.insert(format!("chain:{}:{id}", basis.chain))
    {
        return Err("链上收支哈希、链或钱包不匹配，或交易被重复计入".into());
    }
    let failed = matches!(leg.status, LegStatus::Rejected | LegStatus::Failed)
        && receipt.status == OnchainChainSettlementStatus::ReviewRequired
        && receipt.input_amount_raw.as_deref() == Some("0")
        && receipt.output_amount_raw.as_deref() == Some("0");
    if !failed
        && (leg.status != LegStatus::Confirmed
            || receipt.status != OnchainChainSettlementStatus::Complete)
    {
        return Err("链上实际扣款、到账或网络费用仍待核算".into());
    }
    let cost = receipt
        .network_cost
        .as_ref()
        .filter(|cost| {
            cost.problem.is_none()
                && cost.transaction_id == id
                && cost.chain.eq_ignore_ascii_case(&basis.chain)
                && receipt.block_ref.as_deref() == Some(cost.block_ref.as_str())
                && !cost.payer.is_empty()
        })
        .ok_or("实际网络费尚未取得或与本次交易不一致")?;
    let native = shared_types::onchain_chain_preset(&basis.chain).ok_or("链上原生币身份未知")?;
    if !cost.asset.eq_ignore_ascii_case(native.base_token) {
        return Err("网络费扣费币种与所选链不匹配".into());
    }
    let fee = amount(cost.total_fee_exact.as_deref())?;
    if fee < Decimal::ZERO {
        return Err("网络费不能为负数".into());
    }
    if amount(cost.execution_fee_exact.as_deref())?
        .checked_add(amount(cost.additional_fee_exact.as_deref())?)
        != Some(fee)
    {
        return Err("网络费明细与实扣总额不一致".into());
    }
    if basis.assets.input.symbol.eq_ignore_ascii_case("USD")
        || basis.assets.output.symbol.eq_ignore_ascii_case("USD")
    {
        return Err("链上同名 USD 代币不能直接计作法币美元".into());
    }
    let location = format!("chain:{}:{}", basis.chain, basis.wallet);
    push(
        flows,
        &location,
        id,
        &basis.assets.input.symbol,
        -raw(
            receipt.input_amount_raw.as_deref(),
            basis.assets.input.decimals,
        )?,
        Kind::Trade,
    )?;
    push(
        flows,
        &location,
        id,
        &basis.assets.output.symbol,
        raw(
            receipt.output_amount_raw.as_deref(),
            basis.assets.output.decimals,
        )?,
        Kind::Trade,
    )?;
    if let Some(extra) = &receipt.additional_native_change_raw {
        let (negative, unsigned) = extra
            .strip_prefix('-')
            .map(|v| (true, v))
            .unwrap_or((false, extra));
        let extra = raw(Some(unsigned), if is_solana { 9 } else { 18 })?;
        push(
            flows,
            &location,
            id,
            native.base_token,
            if negative { -extra } else { extra },
            Kind::OtherNativeChange,
        )?;
    }
    // A sponsored Solana transaction is not a fee debit from this wallet.
    if same_address(&cost.payer, &basis.wallet) {
        push(flows, &location, id, &cost.asset, -fee, Kind::Fee)?;
    }
    Ok(())
}

fn push(
    flows: &mut Vec<Flow>,
    location: &str,
    source_id: &str,
    currency: &str,
    value: Decimal,
    kind: Kind,
) -> Result<(), String> {
    let asset = asset(currency)?;
    if value != Decimal::ZERO {
        flows.push(Flow {
            location: location.into(),
            source_id: source_id.into(),
            asset,
            amount_exact: value.normalize().to_string(),
            kind,
        });
    }
    Ok(())
}

pub(super) fn net(flows: &[Flow]) -> Result<Vec<OnchainExecutionAssetChange>, String> {
    let mut net = BTreeMap::<String, Decimal>::new();
    for flow in flows {
        let entry = net.entry(asset(&flow.asset)?).or_default();
        *entry = entry
            .checked_add(amount(Some(&flow.amount_exact))?)
            .ok_or("净资产合计超出支持精度")?;
    }
    Ok(net
        .into_iter()
        .filter(|(_, v)| *v != Decimal::ZERO)
        .map(|(asset, value)| OnchainExecutionAssetChange {
            asset,
            amount_exact: value.normalize().to_string(),
        })
        .collect())
}

pub(super) fn amount(value: Option<&str>) -> Result<Decimal, String> {
    value
        .and_then(|v| Decimal::from_str_exact(v).ok())
        .ok_or_else(|| "实际收支数量缺失或超出支持精度".into())
}

fn raw(value: Option<&str>, decimals: u8) -> Result<Decimal, String> {
    let value = value
        .filter(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
        .ok_or("链上最小单位数量无效")?;
    if decimals > 28 {
        return Err("链上精度超出收支核算范围".into());
    }
    amount(Some(value))?
        .checked_mul(Decimal::new(1, u32::from(decimals)))
        .ok_or_else(|| "链上数量超出支持精度".into())
}

fn asset(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        return Err("收支币种缺失".into());
    }
    Ok(value.to_ascii_uppercase())
}
