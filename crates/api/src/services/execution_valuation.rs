use shared_types::{
    problem::codes, ApiProblem, ExecutionMode, ExecutionRun, ExecutionRunLeg, ExecutionRunState,
    HedgeLegRole, LiveOrderState, OrderIntent, OrderRecord, OrderUpdateSource, RecoveryAction,
};

mod exposure;
pub(crate) use exposure::net_base_exposure_usd;

const SOURCE_EXECUTION_VALUATION: &str = "execution_valuation";
type ValuationResult = Result<f64, Box<ApiProblem>>;

pub(crate) fn fill_notional(record: &OrderRecord, quantity: f64) -> ValuationResult {
    let price = valuation_price(record.filled_price)
        .ok_or_else(|| fill_evidence_problem(record, "missing_filled_price"))?;
    checked_notional(
        &record.intent,
        "fill_notional",
        quantity,
        price,
        record.filled_price,
    )
}

pub(crate) fn confirmed_fill_time_from_record(
    current: Option<i64>,
    record: &OrderRecord,
) -> Option<i64> {
    let is_complete_mock_fill = record.intent.mode == ExecutionMode::DryRun
        && record.state == LiveOrderState::Filled
        && record.last_update_source == OrderUpdateSource::AdapterAck
        && record
            .filled_quantity
            .is_some_and(|value| value.is_finite() && value > 0.0)
        && record
            .filled_price
            .is_some_and(|value| value.is_finite() && value > 0.0)
        && record.updated_at_ms > 0;
    if !is_complete_mock_fill {
        return current;
    }
    Some(current.map_or(record.updated_at_ms, |value| {
        value.max(record.updated_at_ms)
    }))
}

pub(crate) fn fill_evidence_problem(record: &OrderRecord, reason: &'static str) -> Box<ApiProblem> {
    let mut problem = ApiProblem::new(
        codes::HEDGE_EXECUTION_VALUATION_MISSING,
        format!(
            "{} {} 缺少可靠成交证据，不能从订单意图推导成交",
            record.intent.exchange, record.intent.symbol
        ),
    )
    .with_source(SOURCE_EXECUTION_VALUATION);
    problem.details = Some(serde_json::json!({
        "operation": "fill_notional",
        "reason": reason,
        "orderId": record.intent.id.as_str(),
        "clientOrderId": record.intent.client_order_id.as_str(),
        "exchange": record.intent.exchange.as_str(),
        "symbol": record.intent.symbol.as_str(),
        "filledQuantity": record.filled_quantity,
        "filledPrice": record.filled_price,
        "intentQuantity": record.intent.quantity,
        "intentPrice": record.intent.price,
        "orderState": record.state,
        "lastUpdateSource": record.last_update_source,
    }));
    Box::new(problem)
}

pub(crate) fn order_notional(intent: &OrderIntent) -> ValuationResult {
    let price = valuation_price(intent.price).ok_or_else(|| {
        Box::new(valuation_problem(
            intent,
            "order_notional",
            intent.quantity,
            None,
            "missing_intent_price",
        ))
    })?;
    checked_notional(intent, "order_notional", intent.quantity, price, None)
}

pub(crate) fn refresh_cost_reconciliation(run: &mut ExecutionRun) {
    let visible_fee_usd = visible_filled_fee_usd(run);
    let complete_fee_usd = complete_filled_fee_usd(run);
    let open_slippage_usd = open_slippage_usd(run);
    let requires_unwind_cost = requires_unwind_cost(run);
    if let Some(cost) = run.cost_reconciliation.as_mut() {
        cost.filled_fee_usd = visible_fee_usd;
        cost.actual_slippage_usd = open_slippage_usd.filter(|value| value.is_finite());
        cost.actual_open_cost_usd = actual_open_cost_usd(complete_fee_usd, open_slippage_usd);
        cost.actual_unwind_cost_usd =
            actual_unwind_cost_usd(cost.actual_unwind_fee_usd, cost.actual_unwind_slippage_usd);
        cost.actual_cost_usd = actual_total_cost_usd(cost, requires_unwind_cost);
        cost.missing_fields = missing_cost_fields(cost, requires_unwind_cost);
        cost.cost_delta_usd = cost
            .actual_cost_usd
            .map(|actual| actual - cost.estimated_total_cost_usd);
    }
}

fn actual_open_cost_usd(open_fee_usd: Option<f64>, open_slippage_usd: Option<f64>) -> Option<f64> {
    let value = open_fee_usd? + open_slippage_usd?;
    value.is_finite().then_some(value)
}

fn actual_unwind_cost_usd(
    unwind_fee_usd: Option<f64>,
    unwind_slippage_usd: Option<f64>,
) -> Option<f64> {
    let value = unwind_fee_usd? + unwind_slippage_usd?;
    value.is_finite().then_some(value)
}

fn actual_total_cost_usd(
    cost: &shared_types::ExecutionCostReconciliation,
    requires_unwind_cost: bool,
) -> Option<f64> {
    if !requires_unwind_cost {
        return None;
    }
    let open = cost.actual_open_cost_usd?;
    let unwind = cost.actual_unwind_cost_usd?;
    let funding = cost.actual_funding_usd.unwrap_or(0.0);
    let value = open + unwind + funding;
    value.is_finite().then_some(value)
}

fn missing_cost_fields(
    cost: &shared_types::ExecutionCostReconciliation,
    requires_unwind_cost: bool,
) -> Vec<String> {
    let mut fields = Vec::new();
    if cost.actual_open_cost_usd.is_none() {
        fields.push("actualOpenCostUsd".to_owned());
    }
    if requires_unwind_cost {
        if cost.actual_unwind_fee_usd.is_none() {
            fields.push("actualUnwindFeeUsd".to_owned());
        }
        if cost.actual_unwind_slippage_usd.is_none() {
            fields.push("actualUnwindSlippageUsd".to_owned());
        }
        if cost.actual_unwind_cost_usd.is_none() {
            fields.push("actualUnwindCostUsd".to_owned());
        }
        if cost.actual_cost_usd.is_none() {
            fields.push("actualCostUsd".to_owned());
        }
    }
    fields
}

fn requires_unwind_cost(run: &ExecutionRun) -> bool {
    matches!(
        run.state,
        ExecutionRunState::Unwinding | ExecutionRunState::Closed
    ) || matches!(
        run.recovery_action,
        Some(
            RecoveryAction::UnwindLongLeg
                | RecoveryAction::UnwindShortLeg
                | RecoveryAction::ManualReview
        )
    ) || run
        .cost_reconciliation
        .as_ref()
        .is_some_and(|cost| !cost.unwind_event_ids.is_empty())
}

fn visible_filled_fee_usd(run: &ExecutionRun) -> Option<f64> {
    sum_seen_fees([run.long_leg.filled_fee, run.short_leg.filled_fee])
}

fn complete_filled_fee_usd(run: &ExecutionRun) -> Option<f64> {
    let long = finite_value(run.long_leg.filled_fee)?;
    let short = finite_value(run.short_leg.filled_fee)?;
    Some(long + short)
}

fn sum_seen_fees(fees: [Option<f64>; 2]) -> Option<f64> {
    let mut seen = false;
    let mut total = 0.0;
    for fee in fees.into_iter().flatten() {
        if fee.is_finite() {
            seen = true;
            total += fee;
        }
    }
    seen.then_some(total)
}

fn open_slippage_usd(run: &ExecutionRun) -> Option<f64> {
    Some(
        leg_slippage_usd(&run.long_leg, HedgeLegRole::Long)?
            + leg_slippage_usd(&run.short_leg, HedgeLegRole::Short)?,
    )
}

fn leg_slippage_usd(leg: &ExecutionRunLeg, role: HedgeLegRole) -> Option<f64> {
    let expected = expected_filled_notional(leg)?;
    let actual = finite_positive(leg.filled_notional_usd)?;
    match role {
        HedgeLegRole::Long => Some(actual - expected),
        HedgeLegRole::Short => Some(expected - actual),
    }
}

fn expected_filled_notional(leg: &ExecutionRunLeg) -> Option<f64> {
    let filled_quantity = finite_positive(leg.filled_quantity)?;
    let target_quantity = finite_positive(Some(leg.target_quantity))?;
    let target_notional = finite_positive(Some(leg.target_notional_usd))?;
    Some(target_notional * (filled_quantity / target_quantity))
}

fn checked_notional(
    intent: &OrderIntent,
    operation: &'static str,
    quantity: f64,
    price: f64,
    filled_price: Option<f64>,
) -> ValuationResult {
    if quantity.is_finite() && quantity > 0.0 {
        Ok(quantity * price)
    } else {
        Err(Box::new(valuation_problem(
            intent,
            operation,
            quantity,
            filled_price,
            "invalid_quantity",
        )))
    }
}

fn valuation_price(price: Option<f64>) -> Option<f64> {
    price.filter(|value| value.is_finite() && *value > 0.0)
}

fn finite_value(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

fn finite_positive(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}

fn valuation_problem(
    intent: &OrderIntent,
    operation: &'static str,
    quantity: f64,
    filled_price: Option<f64>,
    reason: &'static str,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::HEDGE_EXECUTION_VALUATION_MISSING,
        format!(
            "{} {} 缺少可靠估值价格，不能把名义金额按 0 处理",
            intent.exchange, intent.symbol
        ),
    )
    .with_source(SOURCE_EXECUTION_VALUATION);
    problem.details = Some(serde_json::json!({
        "operation": operation,
        "reason": reason,
        "orderId": intent.id.as_str(),
        "clientOrderId": intent.client_order_id.as_str(),
        "exchange": intent.exchange.as_str(),
        "symbol": intent.symbol.as_str(),
        "quantity": quantity,
        "filledPrice": filled_price,
        "intentPrice": intent.price,
    }));
    problem
}

#[cfg(test)]
#[path = "execution_valuation/tests.rs"]
mod tests;
