use std::collections::BTreeMap;

use rust_decimal::{prelude::FromPrimitive, Decimal};
use shared_types::{
    ExecutionLedgerEvent, ExecutionLedgerEventType, ExecutionLedgerPayload, ExecutionLedgerQuality,
    InstrumentSpec, OnchainCexSettlement, OnchainCexSettlementBasis, OnchainCexSettlementFee,
    OnchainCexSettlementStatus, OnchainExecutionSubmitResponse, OrderRecord, OrderSide,
};

use crate::state::AppState;

const MAX_ORDER_EVENTS: usize = 10_000;

pub(super) fn seed(record: &OrderRecord, spec: &InstrumentSpec) -> Option<OnchainCexSettlement> {
    let quantity = record.filled_quantity?;
    // Spot fills are base units. Never apply this accounting to contract lots.
    if !super::terminal(record.state)
        || !quantity.is_finite()
        || quantity <= 0.0
        || !record.intent.quantity.is_finite()
        || quantity > record.intent.quantity * (1.0 + 1e-12)
        || spec.product_type.as_deref() != Some("spot")
        || spec.contract_size != Some(1.0)
        || !spec.venue.eq_ignore_ascii_case(&record.intent.exchange)
        || !spec
            .native_symbol
            .eq_ignore_ascii_case(&record.intent.symbol)
    {
        return None;
    }
    let (base, quote) = spec.display_symbol.split_once('/')?;
    let base = asset(base)?;
    let quote = asset(quote)?;
    if base == quote || asset(spec.quote_asset.as_deref()?)? != quote {
        return None;
    }
    Some(pending(OnchainCexSettlementBasis {
        order_id: record.intent.id.clone(),
        venue: record.intent.exchange.clone(),
        symbol: record.intent.symbol.clone(),
        side: record.intent.side,
        base_asset: base,
        quote_asset: quote,
        confirmed_quantity: quantity,
    }))
}

pub(super) fn enrich(state: &AppState, run: &mut OnchainExecutionSubmitResponse) {
    for leg in &mut run.legs {
        let Some(previous) = leg.settlement.as_ref() else {
            continue;
        };
        if previous.status == OnchainCexSettlementStatus::Complete {
            continue;
        }
        if leg.order_id.as_deref() != Some(previous.basis.order_id.as_str()) {
            continue;
        }
        let events = state
            .trading_service()
            .list_execution_ledger_events_by_query(&trading::ExecutionLedgerQuery {
                internal_order_id: Some(previous.basis.order_id.clone()),
                limit: MAX_ORDER_EVENTS,
                ..Default::default()
            });
        let next = reconcile(&previous.basis, &events);
        if previous != &next {
            run.updated_at_ms = run.updated_at_ms.max(next.observed_at_ms.unwrap_or(0));
            leg.settlement = Some(next);
        }
    }
    let can_finalize = state.onchain_execution_run_store().readiness().is_ok()
        && !state
            .onchain_execution_run_store()
            .unresolved_cex_action(&run.run_id);
    super::compensation::refresh_residuals(run, can_finalize);
    super::chain_settlement::reconcile_quantity(run, can_finalize);
    super::accounting::enrich(state, run, can_finalize);
}

pub(super) async fn confirmed_order(
    state: &AppState,
    record: &OrderRecord,
    spec: &InstrumentSpec,
) -> Result<OnchainCexSettlement, String> {
    let basis = seed(record, spec)
        .ok_or("CEX 原始成交数量或现货规格未核清，不能按预计数量回滚")?
        .basis;
    let deadline = tokio::time::Instant::now()
        + std::time::Duration::from_millis(super::PRIVATE_FINALITY_WAIT_MS);
    loop {
        let events = state
            .trading_service()
            .list_execution_ledger_events_by_query(&trading::ExecutionLedgerQuery {
                internal_order_id: Some(basis.order_id.clone()),
                limit: MAX_ORDER_EVENTS,
                ..Default::default()
            });
        let snapshot = reconcile(&basis, &events);
        if snapshot.status == OnchainCexSettlementStatus::Complete {
            return Ok(snapshot);
        }
        if snapshot.status == OnchainCexSettlementStatus::Invalid
            || tokio::time::Instant::now() >= deadline
        {
            return Err(snapshot
                .problem
                .unwrap_or_else(|| "CEX 净到账尚未核清".into()));
        }
        // A final order event can precede its fill event. Wait only on the local ledger.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

fn pending(basis: OnchainCexSettlementBasis) -> OnchainCexSettlement {
    OnchainCexSettlement {
        basis,
        status: OnchainCexSettlementStatus::PendingFills,
        gross_base_amount: None,
        gross_quote_amount: None,
        debit_amount: None,
        credit_amount: None,
        fees: Vec::new(),
        fill_event_ids: Vec::new(),
        observed_at_ms: None,
        problem: Some("等待完整逐笔成交，暂不认定扣费后到账".into()),
    }
}

pub(super) fn reconcile(
    basis: &OnchainCexSettlementBasis,
    events: &[ExecutionLedgerEvent],
) -> OnchainCexSettlement {
    let mut result = pending(basis.clone());
    if let Err(problem) = reconcile_into(&mut result, events) {
        result.status = OnchainCexSettlementStatus::Invalid;
        result.debit_amount = None;
        result.credit_amount = None;
        result.problem = Some(problem.into());
    }
    result
}

fn reconcile_into(
    result: &mut OnchainCexSettlement,
    events: &[ExecutionLedgerEvent],
) -> Result<(), &'static str> {
    let basis = &result.basis;
    let expected = decimal(basis.confirmed_quantity)
        .filter(|value| *value > Decimal::ZERO)
        .ok_or("原订单实际成交数量无效")?;
    if basis.base_asset == basis.quote_asset
        || asset(&basis.base_asset).is_none()
        || asset(&basis.quote_asset).is_none()
    {
        return Err("原订单资产身份无效");
    }
    let mut base = Decimal::ZERO;
    let mut quote = Decimal::ZERO;
    let mut fees = BTreeMap::<String, Decimal>::new();
    let mut seen = BTreeMap::<&str, &ExecutionLedgerEvent>::new();
    let mut missing_fee = false;
    for event in events {
        // Cumulative snapshots must not be added to incremental fills.
        if event.event_type != ExecutionLedgerEventType::FillEvent
            || event.order.identity.internal_order_id != basis.order_id
        {
            continue;
        }
        if !event.order.exchange.eq_ignore_ascii_case(&basis.venue)
            || !event.order.symbol.eq_ignore_ascii_case(&basis.symbol)
            || event.order.side != basis.side
            || event.event_id.trim().is_empty()
        {
            return Err("逐笔成交与原订单身份不一致");
        }
        if let Some(previous) = seen.insert(&event.event_id, event) {
            if previous.payload != event.payload || previous.order != event.order {
                return Err("重复成交编号包含矛盾的数量或手续费");
            }
            continue;
        }
        let ExecutionLedgerPayload::FillSnapshot(fill) = &event.payload else {
            return Err("逐笔成交账本格式不一致");
        };
        if fill.quality != ExecutionLedgerQuality::Actual
            || !fill.confidence.supports_terminal_fill()
        {
            return Err("逐笔成交缺少交易所实际成交证明");
        }
        let quantity = decimal(fill.quantity)
            .filter(|value| *value > Decimal::ZERO)
            .ok_or("逐笔成交数量无效")?;
        let price = decimal(fill.average_price)
            .filter(|value| *value > Decimal::ZERO)
            .ok_or("逐笔成交价格无效")?;
        let cost = decimal(fill.quote_value)
            .filter(|value| *value > Decimal::ZERO)
            .ok_or("逐笔成交金额无效")?;
        let computed = quantity.checked_mul(price).ok_or("成交金额超出支持精度")?;
        if !near(cost, computed) {
            return Err("逐笔成交金额与数量、价格矛盾");
        }
        base = base.checked_add(quantity).ok_or("成交数量溢出")?;
        quote = quote.checked_add(cost).ok_or("成交金额溢出")?;
        result.observed_at_ms = Some(result.observed_at_ms.unwrap_or(0).max(event.captured_at_ms));
        match &fill.fee {
            Some(fee) if fee.quality == ExecutionLedgerQuality::Actual => {
                let amount = decimal(fee.amount).ok_or("实际手续费金额无效")?;
                if let Some(currency) = fee.currency.as_deref().and_then(asset) {
                    let total = fees.entry(currency).or_default();
                    *total = total.checked_add(amount).ok_or("手续费金额溢出")?;
                } else if amount != Decimal::ZERO {
                    missing_fee = true;
                }
            }
            _ => missing_fee = true,
        }
    }
    result.fill_event_ids = seen.keys().map(|id| (*id).to_owned()).collect();
    if seen.is_empty() {
        return Ok(());
    }
    result.gross_base_amount = Some(exact(base));
    result.gross_quote_amount = Some(exact(quote));
    result.fees = fees
        .iter()
        .map(|(asset, amount)| OnchainCexSettlementFee {
            asset: asset.clone(),
            amount: exact(*amount),
        })
        .collect();
    if !near(base, expected) {
        if base > expected {
            return Err("逐笔成交合计超过订单终态数量，停止净到账核算");
        }
        return Ok(());
    }
    if missing_fee {
        result.status = OnchainCexSettlementStatus::PendingFees;
        result.problem = Some("成交数量已对齐，等待实际手续费或扣费资产；不按零费用计算".into());
        return Ok(());
    }
    let (gross_debit, gross_credit, from, to) = match basis.side {
        OrderSide::Buy => (quote, base, &basis.quote_asset, &basis.base_asset),
        OrderSide::Sell => (base, quote, &basis.base_asset, &basis.quote_asset),
    };
    // Third-asset fees stay separate. No USD/USDT/USDC peg or implicit conversion.
    let debit = gross_debit
        .checked_add(*fees.get(from).unwrap_or(&Decimal::ZERO))
        .filter(|value| *value >= Decimal::ZERO)
        .ok_or("扣费后支出金额无效")?;
    let credit = gross_credit
        .checked_sub(*fees.get(to).unwrap_or(&Decimal::ZERO))
        .filter(|value| *value >= Decimal::ZERO)
        .ok_or("扣费后到账金额无效")?;
    result.debit_amount = Some(exact(debit));
    result.credit_amount = Some(exact(credit));
    result.status = OnchainCexSettlementStatus::Complete;
    result.problem = None;
    Ok(())
}

fn asset(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_ascii_uppercase())
}

fn decimal(value: f64) -> Option<Decimal> {
    Decimal::from_f64(value).filter(|number| value == 0.0 || *number != Decimal::ZERO)
}

fn exact(value: Decimal) -> String {
    value.normalize().to_string()
}

fn near(left: Decimal, right: Decimal) -> bool {
    let tolerance = right
        .abs()
        .checked_mul(Decimal::from_f64(f64::EPSILON * 64.0).unwrap_or_default())
        .unwrap_or_default();
    left.checked_sub(right)
        .is_some_and(|difference| difference.abs() <= tolerance)
}

#[cfg(test)]
mod tests;
