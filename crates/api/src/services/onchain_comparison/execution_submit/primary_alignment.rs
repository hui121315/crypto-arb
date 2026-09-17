use std::collections::{BTreeMap, BTreeSet};

use rust_decimal::{prelude::ToPrimitive, Decimal, RoundingStrategy};
use shared_types::{
    OnchainCexOrderPlan, OnchainCexSettlement, OnchainCexSettlementStatus,
    OnchainChainInputAdjustment, OnchainComparisonConfig, OnchainComparisonDirection,
    OnchainExecutionBuildResponse, OnchainQuoteConversionSequence, OrderRecord, OrderSide,
};

use super::super::{execution_build, replenishment_credit, usd_valuation};
use super::{providers, settlement};
use crate::{services::onchain_execution_build_store::ClaimedOnchainBuild, state::AppState};

pub(super) struct AlignedChain {
    pub(super) claimed: ClaimedOnchainBuild,
    pub(super) prepared: providers::PreparedChainSubmission,
}

pub(super) async fn align(
    state: &AppState,
    claimed: &ClaimedOnchainBuild,
    primary: &OnchainCexOrderPlan,
    record: &OrderRecord,
    conversions: &[OnchainCexSettlement],
    prepared: providers::PreparedChainSubmission,
) -> Result<AlignedChain, String> {
    if claimed.response.direction != OnchainComparisonDirection::BuyCexSellOnchain {
        return Ok(AlignedChain {
            claimed: claimed.clone(),
            prepared,
        });
    }
    if primary.side != OrderSide::Buy
        || record.intent.side != primary.side
        || record.intent.client_order_id != primary.client_order_id
        || !record.intent.exchange.eq_ignore_ascii_case(&primary.venue)
        || !record
            .intent
            .symbol
            .eq_ignore_ascii_case(&primary.native_symbol)
    {
        return Err("CEX 净到账核验与主买单身份不一致".into());
    }
    let receipt = settlement::confirmed_order(state, record, &primary.instrument_spec).await?;
    let input_raw = aligned_input(&claimed.response, &claimed.config, &receipt)?;
    let costs = paid_assets(&claimed.response, &claimed.config, &receipt, conversions)?;
    let reuse = input_raw == claimed.response.input_amount_raw
        && claimed.response.valid_until_ms > common::time::now_ms()
        && claimed.response.minimum_output_amount_raw.is_some();
    let chain = if reuse {
        let build = &claimed.response;
        execution_build::FirmChainContract {
            quote: onchain_monitor::ProviderQuote {
                input_address: build.input_token.clone(),
                output_address: build.output_token.clone(),
                input_amount_raw: input_raw.clone(),
                output_amount_raw: build.output_amount_raw.clone(),
                router: None,
            },
            minimum_output_amount_raw: build
                .minimum_output_amount_raw
                .clone()
                .ok_or("缺少最少到账约束")?,
            transaction: build.chain_transaction.clone(),
            official_docs_url: build.official_docs_url.clone(),
            quote_observed_at_ms: build.quote_observed_at_ms,
            valid_until_ms: build.valid_until_ms,
        }
    } else {
        execution_build::build_firm_chain_contract(
            state,
            &claimed.config,
            claimed.response.direction,
            &input_raw,
        )
        .await
        .map_err(|problem| problem.to_string())?
    };
    let now_ms = common::time::now_ms();
    let mut cost_usd = 0.0;
    for (asset, amount) in costs {
        let valuation = usd_valuation::evidence(state, &claimed.config, &asset, now_ms)?;
        let price = if amount < Decimal::ZERO {
            valuation.usd_bid
        } else {
            valuation.usd_ask
        };
        cost_usd += amount.to_f64().ok_or("实际支出超出估值精度")? * price;
    }
    let valuation = usd_valuation::quote_evidence(state, &claimed.config, common::time::now_ms())?;
    let updated = revised_build(claimed, &receipt, &input_raw, chain, cost_usd, valuation)?;
    let prepared = if reuse {
        prepared
    } else {
        providers::prepare(state, &updated.config, &updated.response.chain_transaction).await?
    };
    if updated.response.valid_until_ms <= common::time::now_ms() {
        return Err("净到账重询价在签名前后已过期，链上交易未广播".into());
    }
    Ok(AlignedChain {
        claimed: updated,
        prepared,
    })
}

fn aligned_input(
    build: &OnchainExecutionBuildResponse,
    config: &OnchainComparisonConfig,
    receipt: &OnchainCexSettlement,
) -> Result<String, String> {
    if build.direction != OnchainComparisonDirection::BuyCexSellOnchain
        || receipt.status != OnchainCexSettlementStatus::Complete
        || receipt.basis.side != OrderSide::Buy
        || !receipt
            .basis
            .venue
            .eq_ignore_ascii_case(&build.cex_order.venue)
        || !receipt
            .basis
            .symbol
            .eq_ignore_ascii_case(&build.cex_order.native_symbol)
    {
        return Err("CEX 实际买入到账未核清，链上卖单未广播".into());
    }
    let original = build
        .input_amount_raw
        .parse::<u128>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or("原链上输入数量无效")?;
    let net = amount(receipt.credit_amount.as_deref())?;
    if config.base_decimals > 28 {
        return Err("基础币精度超出净到账核算范围".into());
    }
    let truncated =
        net.round_dp_with_strategy(u32::from(config.base_decimals), RoundingStrategy::ToZero);
    let available = replenishment_credit::decimal_to_raw(
        &truncated.normalize().to_string(),
        config.base_decimals,
    )
    .filter(|v| *v > 0)
    .ok_or("CEX 净到账不足链上最小数量单位")?;
    Ok(original.min(available).to_string())
}

fn paid_assets(
    build: &OnchainExecutionBuildResponse,
    config: &OnchainComparisonConfig,
    primary: &OnchainCexSettlement,
    conversions: &[OnchainCexSettlement],
) -> Result<BTreeMap<String, Decimal>, String> {
    if primary.status != OnchainCexSettlementStatus::Complete
        || primary.basis.side != OrderSide::Buy
    {
        return Err("主买单实付明细未核清".into());
    }
    let mut costs = BTreeMap::new();
    let mut ids = BTreeSet::new();
    let quote = config.quote_token.trim().to_ascii_uppercase();
    match &build.quote_conversion_order {
        None if conversions.is_empty() && primary.basis.quote_asset == quote => {
            add(&mut costs, &quote, amount(primary.debit_amount.as_deref())?)?;
        }
        Some(plan)
            if plan.sequence == OnchainQuoteConversionSequence::BeforePrimaryCex
                && plan.from_asset == quote
                && plan.to_asset == primary.basis.quote_asset
                && !conversions.is_empty() =>
        {
            let mut credited = Decimal::ZERO;
            for receipt in conversions {
                if receipt.status != OnchainCexSettlementStatus::Complete
                    || !ids.insert(receipt.basis.order_id.clone())
                    || receipt.basis.order_id == primary.basis.order_id
                    || receipt.basis.side != plan.order.side
                    || !receipt
                        .basis
                        .symbol
                        .eq_ignore_ascii_case(&plan.order.native_symbol)
                    || !receipt.basis.venue.eq_ignore_ascii_case(&plan.order.venue)
                {
                    return Err("前置换汇实际支出缺失或含重复订单，不能继续缩量执行".into());
                }
                let (from, to) = if receipt.basis.side == OrderSide::Buy {
                    (&receipt.basis.quote_asset, &receipt.basis.base_asset)
                } else {
                    (&receipt.basis.base_asset, &receipt.basis.quote_asset)
                };
                if from != &quote || to != &primary.basis.quote_asset {
                    return Err("换汇回执资产与本次支付路径不一致".into());
                }
                add(&mut costs, from, amount(receipt.debit_amount.as_deref())?)?;
                credited = credited
                    .checked_add(amount(receipt.credit_amount.as_deref())?)
                    .ok_or("换汇到账求和溢出")?;
                add_other_fees(&mut costs, receipt)?;
            }
            if credited < amount(primary.debit_amount.as_deref())? {
                return Err("主买单实际支出超过前置换汇净到账，不能认定路径已自筹资金".into());
            }
            let retained = credited - amount(primary.debit_amount.as_deref())?;
            if retained > Decimal::ZERO {
                add(&mut costs, &primary.basis.quote_asset, -retained)?;
            }
        }
        _ => return Err("缺少同币种支付或前置换汇实付证明，不能隐式折算成本".into()),
    }
    add_other_fees(&mut costs, primary)?;
    Ok(costs)
}

fn add_other_fees(
    costs: &mut BTreeMap<String, Decimal>,
    receipt: &OnchainCexSettlement,
) -> Result<(), String> {
    for fee in &receipt.fees {
        if fee.asset == receipt.basis.base_asset || fee.asset == receipt.basis.quote_asset {
            continue;
        }
        let value = fee
            .amount
            .parse::<Decimal>()
            .map_err(|_| "实际手续费金额无效")?;
        // Unconfirmed future rebates are never added to this conservative profit gate.
        if value > Decimal::ZERO {
            add(costs, &fee.asset, value)?;
        }
    }
    Ok(())
}

fn add(costs: &mut BTreeMap<String, Decimal>, asset: &str, value: Decimal) -> Result<(), String> {
    if asset.trim().is_empty() {
        return Err("手续费或支付币种为空".into());
    }
    let total = costs.entry(asset.to_owned()).or_default();
    *total = total.checked_add(value).ok_or("实际支付成本溢出")?;
    Ok(())
}

fn amount(value: Option<&str>) -> Result<Decimal, String> {
    value
        .and_then(|v| v.parse::<Decimal>().ok())
        .filter(|v| *v > Decimal::ZERO)
        .ok_or_else(|| "实际净到账或支付金额缺失".into())
}

pub(super) fn retain_residual(
    build: &OnchainExecutionBuildResponse,
    run: &mut shared_types::OnchainExecutionSubmitResponse,
) {
    if run.status != shared_types::OnchainExecutionRunStatus::Completed {
        return;
    }
    let Some(adjustment) = &build.chain_input_adjustment else {
        return;
    };
    let residual = adjustment.residual_base_amount.parse::<Decimal>();
    if residual.as_ref().is_ok_and(|v| *v == Decimal::ZERO) {
        return;
    }
    if !residual.is_ok_and(|v| v > Decimal::ZERO)
        || !adjustment.residual_cost_estimate_usd.is_finite()
        || adjustment.residual_cost_estimate_usd < 0.0
    {
        run.status = shared_types::OnchainExecutionRunStatus::FinalityUnresolved;
        run.problem = Some("链上输入余量证据无效，不能认定资产已核平".into());
    } else {
        run.status = shared_types::OnchainExecutionRunStatus::Exposed;
        run.remaining_exposure_usd = adjustment.residual_cost_estimate_usd;
        run.message = "交易已确认，预计双端净余量待核对".into();
        run.problem = Some(format!(
            "预计双端净余量 {} {}；按 CEX 净到账减去计划链上输入计算，仍需核对链上实际扣款；美元金额按本次取得成本估值",
            adjustment.residual_base_amount, adjustment.asset
        ));
    }
    run.recovery_actions = super::recovery_actions(run.status);
}

fn revised_build(
    claimed: &ClaimedOnchainBuild,
    receipt: &OnchainCexSettlement,
    input_raw: &str,
    chain: execution_build::FirmChainContract,
    cost_usd: f64,
    valuation: shared_types::OnchainUsdValuation,
) -> Result<ClaimedOnchainBuild, String> {
    let config = &claimed.config;
    let token_matches = |left: &str, right: &str| {
        if config.chain.eq_ignore_ascii_case("solana") {
            left == right
        } else {
            left.eq_ignore_ascii_case(right)
        }
    };
    if chain.quote.input_amount_raw != input_raw
        || !token_matches(&chain.quote.input_address, &claimed.response.input_token)
        || !token_matches(&chain.quote.output_address, &claimed.response.output_token)
        || aligned_input(&claimed.response, config, receipt)? != input_raw
    {
        return Err("重询价返回的输入数量或币种与净到账调整不一致".into());
    }
    let minimum_raw = chain
        .minimum_output_amount_raw
        .parse::<u128>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or("最少到账原始数量无效")?;
    let expected_raw = chain
        .quote
        .output_amount_raw
        .parse::<u128>()
        .ok()
        .filter(|v| *v > 0)
        .ok_or("预计到账原始数量无效")?;
    if minimum_raw > expected_raw
        || valuation.asset != config.quote_token.trim().to_ascii_uppercase()
    {
        return Err("最少到账超过报价或美元估值币种不匹配".into());
    }
    let minimum =
        super::super::quote::raw_units(&chain.minimum_output_amount_raw, config.quote_decimals)
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or("重询价最少到账数量无效")?;
    let replenishment_cost =
        super::super::replenishment_allocation::total_usd(&claimed.response.replenishment_costs)?;
    let approval_cost = super::super::approval_allocation::total_usd(&claimed.response.approval_costs)?;
    let profit = minimum * valuation.usd_bid - cost_usd - config.gas_usd - replenishment_cost - approval_cost;
    let spread = profit / cost_usd * 10_000.0;
    if !cost_usd.is_finite()
        || cost_usd <= 0.0
        || !valuation.usd_bid.is_finite()
        || valuation.usd_bid <= 0.0
        || !config.gas_usd.is_finite()
        || config.gas_usd < 0.0
        || !config.spread_alert.min_net_spread_bps.is_finite()
        || !profit.is_finite()
        || profit <= 0.0
        || !spread.is_finite()
        || spread < config.spread_alert.min_net_spread_bps.max(0.0)
    {
        return Err(
            "按实际支付、实际手续费和重询价最少到账重算后，收益未达门槛；链上交易未广播".into(),
        );
    }
    let raw_decimal = input_raw
        .parse::<Decimal>()
        .map_err(|_| "链上输入超出净到账精度")?;
    let submitted = raw_decimal
        .checked_mul(Decimal::new(1, u32::from(config.base_decimals)))
        .ok_or("链上输入换算溢出")?;
    // This is cross-venue inventory change, not the balance remaining at the CEX.
    // The submitted chain input still requires reconciliation with its actual receipt.
    let residual = amount(receipt.credit_amount.as_deref())? - submitted;
    let residual_cost =
        residual.to_f64().ok_or("剩余币量溢出")? * cost_usd / receipt.basis.confirmed_quantity;
    if residual < Decimal::ZERO || !residual_cost.is_finite() || residual_cost < 0.0 {
        return Err("调整后的链上数量超过实际净到账".into());
    }
    let mut updated = claimed.clone();
    updated.response.chain_input_adjustment = Some(OnchainChainInputAdjustment {
        original_input_amount_raw: claimed
            .response
            .chain_input_adjustment
            .as_ref()
            .map(|row| row.original_input_amount_raw.clone())
            .unwrap_or_else(|| claimed.response.input_amount_raw.clone()),
        submitted_input_amount_raw: input_raw.to_owned(),
        asset: config.base_token.clone(),
        decimals: config.base_decimals,
        cex_order_id: receipt.basis.order_id.clone(),
        cex_net_received: receipt.credit_amount.clone().ok_or("缺少净到账数量")?,
        residual_base_amount: residual.normalize().to_string(),
        residual_cost_estimate_usd: residual_cost,
    });
    updated.response.input_amount_raw = input_raw.to_owned();
    updated.response.output_amount_raw = chain.quote.output_amount_raw;
    updated.response.minimum_output_amount_raw = Some(chain.minimum_output_amount_raw);
    updated.response.chain_transaction = chain.transaction;
    updated.response.official_docs_url = chain.official_docs_url;
    updated.response.quote_observed_at_ms = chain.quote_observed_at_ms;
    updated.response.valid_until_ms = chain.valid_until_ms;
    updated.response.quote_usd_valuation = Some(valuation);
    updated.response.estimated_net_profit_usd = profit;
    updated.response.estimated_net_spread_bps = spread;
    Ok(updated)
}

#[cfg(test)]
mod tests;
