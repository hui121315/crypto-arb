use rust_decimal::{prelude::ToPrimitive, Decimal};
use shared_types::{
    InstrumentSpec, OnchainCexOrderPlan, OnchainCexSettlement, OnchainCexSettlementStatus,
    OnchainComparisonConfig, OnchainQuoteConversionOrderPlan, OnchainQuoteConversionSequence,
    OrderRecord, OrderSide,
};

use super::ConversionRun;
use crate::state::AppState;

#[derive(Clone, Copy)]
pub(super) struct Limits {
    max_debit: Decimal,
    required_credit: Decimal,
    fee: Decimal,
}

pub(super) fn before_primary(
    config: &OnchainComparisonConfig,
    primary: &OnchainCexOrderPlan,
    conversion: &OnchainQuoteConversionOrderPlan,
) -> Result<Limits, String> {
    if primary.side != OrderSide::Buy
        || conversion.sequence != OnchainQuoteConversionSequence::BeforePrimaryCex
        || !primary.venue.eq_ignore_ascii_case(&conversion.order.venue)
        || primary.instrument_spec.quote_asset.as_deref() != Some(&conversion.to_asset)
    {
        return Err("换汇到账资产与 CEX 主买单支付资产不一致".into());
    }
    let fee = fee(config)?;
    Ok(Limits {
        max_debit: product(
            positive(conversion.planned_from_amount)?,
            Decimal::ONE + fee,
        )?,
        required_credit: product(
            positive(primary.estimated_quote_amount)?,
            Decimal::ONE + fee,
        )?,
        fee,
    })
}

pub(super) async fn after_chain(
    state: &AppState,
    config: &OnchainComparisonConfig,
    primary: &OrderRecord,
    spec: &InstrumentSpec,
    conversion: &OnchainQuoteConversionOrderPlan,
) -> Result<Limits, String> {
    let receipt = super::super::settlement::confirmed_order(state, primary, spec).await?;
    limits_from_primary(config, conversion, &receipt)
}

fn limits_from_primary(
    config: &OnchainComparisonConfig,
    conversion: &OnchainQuoteConversionOrderPlan,
    receipt: &OnchainCexSettlement,
) -> Result<Limits, String> {
    let fee = fee(config)?;
    let planned_cap = product(
        positive(conversion.planned_from_amount)?,
        Decimal::ONE + fee,
    )?;
    let expected_side = match conversion.sequence {
        OnchainQuoteConversionSequence::BeforePrimaryCex => OrderSide::Buy,
        OnchainQuoteConversionSequence::AfterPrimaryCex => OrderSide::Sell,
    };
    let quote = if expected_side == OrderSide::Buy {
        &conversion.to_asset
    } else {
        &conversion.from_asset
    };
    if receipt.status != OnchainCexSettlementStatus::Complete
        || receipt.basis.side != expected_side
        || !receipt
            .basis
            .venue
            .eq_ignore_ascii_case(&conversion.order.venue)
        || &receipt.basis.quote_asset != quote
    {
        return Err("主单扣费到账未核清，不能据此继续换汇".into());
    }
    Ok(match conversion.sequence {
        OnchainQuoteConversionSequence::BeforePrimaryCex => Limits {
            max_debit: planned_cap,
            required_credit: amount(receipt.debit_amount.as_deref())?,
            fee,
        },
        OnchainQuoteConversionSequence::AfterPrimaryCex => Limits {
            max_debit: planned_cap.min(amount(receipt.credit_amount.as_deref())?),
            required_credit: product(positive(conversion.planned_to_amount)?, Decimal::ONE - fee)?,
            fee,
        },
    })
}

pub(super) async fn refresh(
    state: &AppState,
    original: &OnchainQuoteConversionOrderPlan,
    run: &mut ConversionRun,
    limits: Limits,
) -> bool {
    run.complete = false;
    if run.receipt_unresolved {
        return false;
    }
    let result = async {
        let mut debit = Decimal::ZERO;
        let mut credit = Decimal::ZERO;
        for attempt in &mut run.attempts {
            if attempt.record.filled_quantity == Some(0.0) {
                continue;
            }
            if attempt.settlement.is_none() {
                attempt.settlement = Some(
                    super::super::settlement::confirmed_order(
                        state,
                        &attempt.record,
                        &attempt.plan.instrument_spec,
                    )
                    .await?,
                );
            }
            let receipt = attempt.settlement.as_ref().ok_or("换汇到账明细缺失")?;
            if receipt.basis.order_id != attempt.record.intent.id {
                return Err("换汇到账明细与原订单不一致".to_owned());
            }
            let (from, to) = receipt_amounts(original, receipt)?;
            debit = debit.checked_add(from).ok_or("换汇累计支出溢出")?;
            credit = credit.checked_add(to).ok_or("换汇累计到账溢出")?;
        }
        run.accounted_from = debit;
        run.accounted_to = credit;
        if debit > limits.max_debit {
            return Err(format!(
                "换汇实际支出 {} {} 超过本次资金上限 {}，已停止后续资金动作",
                debit.normalize(),
                original.from_asset,
                limits.max_debit.normalize()
            ));
        }
        Ok(credit >= limits.required_credit)
    }
    .await;
    match result {
        Ok(complete) => {
            run.complete = complete;
            true
        }
        Err(problem) => {
            run.receipt_unresolved = true;
            run.problem = Some(problem);
            false
        }
    }
}

fn receipt_amounts(
    original: &OnchainQuoteConversionOrderPlan,
    receipt: &OnchainCexSettlement,
) -> Result<(Decimal, Decimal), String> {
    let (from, to) = if receipt.basis.side == OrderSide::Buy {
        (&receipt.basis.quote_asset, &receipt.basis.base_asset)
    } else {
        (&receipt.basis.base_asset, &receipt.basis.quote_asset)
    };
    if receipt.status != OnchainCexSettlementStatus::Complete
        || receipt.basis.side != original.order.side
        || !receipt
            .basis
            .venue
            .eq_ignore_ascii_case(&original.order.venue)
        || !receipt
            .basis
            .symbol
            .eq_ignore_ascii_case(&original.order.native_symbol)
        || from != &original.from_asset
        || to != &original.to_asset
    {
        return Err("换汇扣费明细缺失或资产方向不一致，不能使用成交总额代替净到账".into());
    }
    Ok((
        amount(receipt.debit_amount.as_deref())?,
        amount(receipt.credit_amount.as_deref())?,
    ))
}

pub(super) fn remaining(
    original: &OnchainQuoteConversionOrderPlan,
    run: &ConversionRun,
    limits: Limits,
) -> Result<(f64, f64), String> {
    let gross_cap = positive(original.planned_from_amount)? - decimal(run.filled_from)?;
    let remaining_input = (limits.max_debit - run.accounted_from)
        .checked_div(Decimal::ONE + limits.fee)
        .ok_or("换汇剩余预算溢出")?
        .min(gross_cap);
    let remaining_output = (limits.required_credit - run.accounted_to)
        .checked_div(Decimal::ONE - limits.fee)
        .ok_or("换汇剩余到账目标溢出")?;
    if remaining_input <= Decimal::ZERO || remaining_output <= Decimal::ZERO {
        return Err(format!(
            "换汇净到账 {} {}，目标 {}；剩余预算不足，不能动用账户其他余额",
            run.accounted_to.normalize(),
            original.to_asset,
            limits.required_credit.normalize()
        ));
    }
    Ok((
        bounded_float(remaining_input, false)?,
        bounded_float(remaining_output, true)?,
    ))
}

pub(super) fn initial_plan_fits(
    original: &OnchainQuoteConversionOrderPlan,
    run: &ConversionRun,
    limits: Limits,
) -> Result<bool, String> {
    let (input, output) = remaining(original, run, limits)?;
    Ok(original.planned_from_amount <= input && original.planned_to_amount >= output)
}

pub(super) fn validate_next(
    original: &OnchainQuoteConversionOrderPlan,
    next: &OnchainQuoteConversionOrderPlan,
    run: &ConversionRun,
    limits: Limits,
) -> Result<(), String> {
    let input = positive(next.planned_from_amount)?;
    if decimal(run.filled_from)? + input > positive(original.planned_from_amount)?
        || product(input, Decimal::ONE + limits.fee)? > limits.max_debit - run.accounted_from
    {
        return Err("换汇补单及手续费预留超过本次剩余资金，已停止提交".into());
    }
    Ok(())
}

fn bounded_float(value: Decimal, round_up: bool) -> Result<f64, String> {
    let mut value_float = value
        .to_f64()
        .filter(|v| v.is_finite() && *v > 0.0)
        .ok_or("换汇资金超出数量精度")?;
    let roundtrip = decimal(value_float)?;
    if round_up && roundtrip < value {
        value_float = value_float.next_up();
    } else if !round_up && roundtrip > value {
        value_float = value_float.next_down();
    }
    Ok(value_float)
}

fn fee(config: &OnchainComparisonConfig) -> Result<Decimal, String> {
    if !(0.0..=2500.0).contains(&config.cex_taker_fee_bps) {
        return Err("换汇手续费预留比例无效".into());
    }
    Ok(decimal(config.cex_taker_fee_bps)? / Decimal::from(10_000))
}

fn positive(value: f64) -> Result<Decimal, String> {
    let value = decimal(value)?;
    if value > Decimal::ZERO {
        Ok(value)
    } else {
        Err("换汇计划金额必须大于零".into())
    }
}

fn decimal(value: f64) -> Result<Decimal, String> {
    if !value.is_finite() {
        return Err("换汇金额无效".into());
    }
    value
        .to_string()
        .parse()
        .map_err(|_| "换汇金额超出精度".into())
}

fn product(amount: Decimal, multiplier: Decimal) -> Result<Decimal, String> {
    amount
        .checked_mul(multiplier)
        .ok_or_else(|| "换汇资金预算计算溢出".into())
}

fn amount(value: Option<&str>) -> Result<Decimal, String> {
    value
        .and_then(|v| v.parse::<Decimal>().ok())
        .filter(|v| *v > Decimal::ZERO)
        .ok_or_else(|| "实际扣费到账金额缺失或无效".into())
}

#[cfg(test)]
mod tests;
