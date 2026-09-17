use rust_decimal::{
    prelude::{FromPrimitive, ToPrimitive},
    Decimal,
};
use shared_types::{
    OnchainCexOrderPlan, OnchainCexSettlement, OnchainCexSettlementStatus, OrderRecord, OrderSide,
};

pub(super) fn plan(
    original: &OnchainCexOrderPlan,
    record: &OrderRecord,
    receipt: &OnchainCexSettlement,
) -> Result<OnchainCexOrderPlan, String> {
    if original.client_order_id != record.intent.client_order_id
        || original.side != record.intent.side
        || !original.venue.eq_ignore_ascii_case(&record.intent.exchange)
        || !original
            .native_symbol
            .eq_ignore_ascii_case(&record.intent.symbol)
        || receipt.basis.order_id != record.intent.id
        || receipt.basis.side != original.side
        || !receipt.basis.venue.eq_ignore_ascii_case(&original.venue)
        || !receipt
            .basis
            .symbol
            .eq_ignore_ascii_case(&original.native_symbol)
    {
        return Err("净到账回执与待回滚订单身份不一致".into());
    }
    let net_base = base_delta(receipt)?.abs();
    if net_base <= Decimal::ZERO {
        return Err("原订单没有可回滚的净资产变动".into());
    }
    let price = record
        .filled_price
        .filter(|price| price.is_finite() && *price > 0.0)
        .ok_or("原订单缺少实际成交均价，不能用参考价代替")?;
    let step = original
        .instrument_spec
        .qty_step
        .and_then(Decimal::from_f64)
        .filter(|step| *step > Decimal::ZERO)
        .ok_or("缺少有效的官方现货数量步长")?;
    let steps = net_base.checked_div(step).ok_or("净到账数量超出支持精度")?;
    // Selling cannot consume unreceived base. Buying restores the actual base debit.
    let lots = match original.side {
        OrderSide::Buy => steps.floor(),
        OrderSide::Sell => steps.ceil(),
    };
    let exact_target = lots.checked_mul(step).ok_or("回滚步长数量溢出")?;
    let target = exact_target
        .to_f64()
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| {
            format!(
                "剩余 {} {} 小于官方下单步长，需保留为待处理余额",
                net_base.normalize(),
                receipt.basis.base_asset
            )
        })?;
    if original.instrument_spec.product_type.as_deref() != Some("spot")
        || original.instrument_spec.contract_size != Some(1.0)
        || target.to_string().parse::<Decimal>().ok() != Some(exact_target)
    {
        return Err("回滚数量不能按现货原生精度准确表达".into());
    }
    let mut sizing =
        shared_types::plan_leg_sizing_for_base_quantity(target, &original.instrument_spec, price)
            .map_err(|block| format!("CEX 净到账回滚数量构建失败：{}", block.code()))?;
    let difference = (sizing.rounded_base_qty - target).abs();
    if difference > f64::EPSILON * 4.0 * target.abs() || difference >= sizing.qty_step / 2.0 {
        return Err("官方下单尺寸与净到账步长数量不一致".into());
    }
    if original.side == OrderSide::Buy && exact_target > net_base {
        return Err("回滚卖出数量超过实际净到账，已停止提交".into());
    }
    // Preserve the decimal lot amount at the f64 sizing boundary, not a multiplied tail.
    sizing.rounded_contracts = target;
    sizing.rounded_base_qty = target;
    sizing.actual_notional_usd = target * price;
    sizing.rounding_delta_usd = (sizing.target_notional_usd - sizing.actual_notional_usd).max(0.0);
    sizing.rounding_loss_bps = sizing.rounding_delta_usd / sizing.target_notional_usd * 10_000.0;
    Ok(OnchainCexOrderPlan {
        venue: original.venue.clone(),
        native_symbol: original.native_symbol.clone(),
        client_order_id: format!("xl{}", &uuid::Uuid::new_v4().simple().to_string()[..18]),
        side: super::opposite(original.side),
        base_quantity: sizing.rounded_base_qty,
        reference_price: price,
        estimated_quote_amount: sizing.rounded_base_qty * price,
        instrument_spec: original.instrument_spec.clone(),
        sizing_plan: sizing,
    })
}

pub(super) fn base_delta(receipt: &OnchainCexSettlement) -> Result<Decimal, String> {
    if receipt.status != OnchainCexSettlementStatus::Complete {
        return Err("成交或扣费明细尚未齐全，不能认定回滚完成".into());
    }
    let amount = match receipt.basis.side {
        OrderSide::Buy => receipt.credit_amount.as_deref(),
        OrderSide::Sell => receipt.debit_amount.as_deref(),
    }
    .and_then(|value| value.parse::<Decimal>().ok())
    .filter(|value| *value >= Decimal::ZERO)
    .ok_or("净资产变动金额无效")?;
    Ok(if receipt.basis.side == OrderSide::Buy {
        amount
    } else {
        -amount
    })
}

pub(super) fn residual(
    original: &OnchainCexSettlement,
    reverse: &OnchainCexSettlement,
) -> Result<Decimal, String> {
    if original.basis.order_id == reverse.basis.order_id
        || original.basis.venue != reverse.basis.venue
        || original.basis.symbol != reverse.basis.symbol
        || original.basis.base_asset != reverse.basis.base_asset
        || original.basis.quote_asset != reverse.basis.quote_asset
        || original.basis.side == reverse.basis.side
    {
        return Err("补偿回执与原订单不是同场所、同资产的反向交易".into());
    }
    let delta = base_delta(original)?
        .checked_add(base_delta(reverse)?)
        .ok_or("回滚剩余资产计算溢出")?;
    Ok(delta)
}

pub(super) fn refresh_residuals(
    run: &mut shared_types::OnchainExecutionSubmitResponse,
    can_finalize: bool,
) {
    use shared_types::{
        OnchainExecutionLegKind as Kind, OnchainExecutionLegStatus as LegStatus,
        OnchainExecutionRunStatus as Status,
    };
    let updates = run
        .legs
        .iter()
        .map(|leg| {
            let mut link = leg.recovery_residual.clone()?;
            let originals = run
                .legs
                .iter()
                .filter(|source| {
                    source.order_id.as_deref() == Some(link.original_order_id.as_str())
                        && matches!(source.kind, Kind::PrimaryCex | Kind::QuoteConversion)
                })
                .collect::<Vec<_>>();
            let reverse_count = run
                .legs
                .iter()
                .filter(|candidate| {
                    candidate.kind == Kind::Compensation
                        && candidate
                            .recovery_residual
                            .as_ref()
                            .is_some_and(|candidate| {
                                candidate.original_order_id == link.original_order_id
                            })
                })
                .count();
            link.amount = None;
            if leg.kind == Kind::Compensation && originals.len() == 1 && reverse_count == 1 {
                if let (Some(original), Some(reverse)) = (&originals[0].settlement, &leg.settlement)
                {
                    if reverse.basis.base_asset == link.asset
                        && original.basis.order_id == link.original_order_id
                        && leg.order_id.as_deref() == Some(reverse.basis.order_id.as_str())
                    {
                        link.amount = residual(original, reverse)
                            .ok()
                            .map(|amount| amount.normalize().to_string());
                    }
                } else if leg.filled_quantity == Some(0.0)
                    && matches!(
                        leg.status,
                        LegStatus::Cancelled | LegStatus::Rejected | LegStatus::Failed
                    )
                {
                    link.amount = originals[0]
                        .settlement
                        .as_ref()
                        .filter(|original| original.basis.base_asset == link.asset)
                        .and_then(|original| base_delta(original).ok())
                        .map(|amount| amount.normalize().to_string());
                }
            }
            Some(link)
        })
        .collect::<Vec<_>>();
    for (leg, residual) in run.legs.iter_mut().zip(updates) {
        leg.recovery_residual = residual;
    }
    let has_compensation = run.legs.iter().any(|leg| leg.kind == Kind::Compensation);
    let terminal = run.legs.iter().all(|leg| {
        matches!(
            leg.status,
            LegStatus::Filled | LegStatus::Cancelled | LegStatus::Rejected | LegStatus::Failed
        )
    });
    let originals_flat = run
        .legs
        .iter()
        .filter(|leg| {
            matches!(leg.kind, Kind::PrimaryCex | Kind::QuoteConversion)
                && leg.filled_quantity.is_some_and(|quantity| quantity > 0.0)
        })
        .all(|source| {
            let matches = run
                .legs
                .iter()
                .filter_map(|leg| leg.recovery_residual.as_ref())
                .filter(|link| source.order_id.as_deref() == Some(link.original_order_id.as_str()))
                .collect::<Vec<_>>();
            matches.len() == 1 && matches[0].amount.as_deref() == Some("0")
        });
    let reverses_flat = run
        .legs
        .iter()
        .filter(|leg| leg.kind == Kind::Compensation)
        .all(|leg| {
            leg.recovery_residual
                .as_ref()
                .is_some_and(|link| link.amount.as_deref() == Some("0"))
        });
    // Read-only reconciliation can settle accounting, but never submit a missing leg.
    // A confirmed or pending chain leg cannot be rolled back by CEX fills alone.
    if can_finalize
        && has_compensation
        && terminal
        && originals_flat
        && reverses_flat
        && !run.legs.iter().any(|leg| {
            leg.kind == Kind::Chain
                && !matches!(leg.status, LegStatus::Rejected | LegStatus::Failed)
        })
        && matches!(run.status, Status::Exposed | Status::FinalityUnresolved)
    {
        run.status = Status::Compensated;
        run.remaining_exposure_usd = 0.0;
        run.message = "CEX 原单与补偿单净数量已核对归零；交易费用已保留在逐腿核算中".into();
        run.problem = None;
        run.recovery_actions = super::recovery_actions(Status::Compensated);
    }
}

#[cfg(test)]
mod tests;
