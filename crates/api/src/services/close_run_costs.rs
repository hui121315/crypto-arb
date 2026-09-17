use shared_types::{
    CloseLeg, CloseRun, CloseRunCompensationAttempt, CloseRunCostComponent,
    CloseRunCostLedgerEvent, CloseRunCostReconciliation, ExecutionLedgerQuality, OrderRecord,
};

mod attribution;

use attribution::{CostParts, RunCostParts};

const CLOSE_FEE: &str = "close_fee";
const CLOSE_SLIPPAGE: &str = "close_slippage";
const COMPENSATION_FEE: &str = "compensation_fee";
const COMPENSATION_SLIPPAGE: &str = "compensation_slippage";

pub(crate) fn refresh_cost_reconciliation(run: &mut CloseRun) {
    run.cost_reconciliation = cost_reconciliation(run);
}

fn cost_reconciliation(run: &CloseRun) -> Option<CloseRunCostReconciliation> {
    let close = close_cost_parts(&run.legs);
    let compensation = compensation_cost_parts(run);
    let run_cost = run_cost_parts(run);
    if !close.has_evidence() && !compensation.has_evidence() && !run_cost.has_evidence() {
        return None;
    }
    let mut summary = CloseRunCostReconciliation {
        close_fee_usd: close.fee.observed_sum(),
        close_slippage_usd: close.slippage.observed_sum(),
        compensation_fee_usd: compensation.fee.observed_sum(),
        compensation_slippage_usd: compensation.slippage.observed_sum(),
        funding_usd: run_cost.funding.observed_sum(),
        manual_handling_usd: run_cost.manual_handling.observed_sum(),
        evidence_order_ids: evidence_order_ids(&close, &compensation),
        evidence_event_ids: evidence_event_ids(&close, &compensation, &run_cost),
        close_fee_event_ids: close.fee.event_ids(),
        close_slippage_event_ids: close.slippage.event_ids(),
        compensation_fee_event_ids: compensation.fee.event_ids(),
        compensation_slippage_event_ids: compensation.slippage.event_ids(),
        funding_event_ids: run_cost.funding.event_ids(),
        manual_handling_event_ids: run_cost.manual_handling.event_ids(),
        missing_fields: missing_fields(&close, &compensation, &run_cost),
        ..CloseRunCostReconciliation::default()
    };
    summary.total_actual_cost_usd = total_actual_cost(&close, &compensation, &run_cost);
    Some(summary)
}

fn close_cost_parts(legs: &[CloseLeg]) -> CostParts {
    let mut parts = CostParts::default();
    for leg in legs {
        let Some(order) = leg.order.as_ref() else {
            continue;
        };
        if !has_fill_evidence(order) {
            continue;
        }
        parts.push_evidence(order);
        parts
            .fee
            .observe_order_value(order.filled_fee, &leg.cost_events);
        parts
            .slippage
            .observe_event_sum(&leg.cost_events, CloseRunCostComponent::Slippage);
    }
    parts
}

fn compensation_cost_parts(run: &CloseRun) -> CostParts {
    let mut parts = CostParts::default();
    let Some(plan) = run.unwind_plan.as_ref() else {
        return parts;
    };
    for attempt in &plan.compensation_attempts {
        compensation_attempt_cost_parts(attempt, &mut parts);
    }
    parts
}

fn compensation_attempt_cost_parts(attempt: &CloseRunCompensationAttempt, parts: &mut CostParts) {
    let Some(order) = attempt.order.as_ref() else {
        return;
    };
    if !has_fill_evidence(order) {
        return;
    }
    parts.push_evidence(order);
    parts
        .fee
        .observe_order_value(order.filled_fee, &attempt.cost_events);
    parts
        .slippage
        .observe_event_sum(&attempt.cost_events, CloseRunCostComponent::Slippage);
}

fn run_cost_parts(run: &CloseRun) -> RunCostParts {
    let mut parts = RunCostParts::default();
    parts
        .funding
        .observe_optional_event_sum(&run.cost_events, CloseRunCostComponent::Funding);
    if run
        .unwind_plan
        .as_ref()
        .and_then(|plan| plan.manual_terminal_evidence.as_ref())
        .is_some()
    {
        parts.manual_handling.required += 1;
        parts
            .manual_handling
            .observe_events(&run.cost_events, CloseRunCostComponent::ManualHandling);
    } else {
        parts
            .manual_handling
            .observe_optional_event_sum(&run.cost_events, CloseRunCostComponent::ManualHandling);
    }
    parts
}

fn total_actual_cost(
    close: &CostParts,
    compensation: &CostParts,
    run_cost: &RunCostParts,
) -> Option<f64> {
    if !close.complete() || !compensation.complete() || !run_cost.complete() {
        return None;
    }
    Some(close.total() + compensation.total() + run_cost.total())
}

fn evidence_order_ids(close: &CostParts, compensation: &CostParts) -> Vec<String> {
    let mut ids = close.evidence_order_ids.clone();
    ids.extend(compensation.evidence_order_ids.iter().cloned());
    ids.sort();
    ids.dedup();
    ids
}

fn evidence_event_ids(
    close: &CostParts,
    compensation: &CostParts,
    run_cost: &RunCostParts,
) -> Vec<String> {
    let mut ids = close.fee.event_ids();
    ids.extend(close.slippage.event_ids());
    ids.extend(compensation.fee.event_ids());
    ids.extend(compensation.slippage.event_ids());
    ids.extend(run_cost.funding.event_ids());
    ids.extend(run_cost.manual_handling.event_ids());
    ids.sort();
    ids.dedup();
    ids
}

fn missing_fields(
    close: &CostParts,
    compensation: &CostParts,
    run_cost: &RunCostParts,
) -> Vec<String> {
    let mut fields = Vec::new();
    close.push_missing(CLOSE_FEE, CLOSE_SLIPPAGE, &mut fields);
    compensation.push_missing(COMPENSATION_FEE, COMPENSATION_SLIPPAGE, &mut fields);
    run_cost.push_missing(&mut fields);
    fields
}

fn has_fill_evidence(order: &OrderRecord) -> bool {
    order.filled_quantity.and_then(valid_positive).is_some()
        && order.filled_price.and_then(valid_positive).is_some()
}

fn valid_positive(value: f64) -> Option<f64> {
    (value.is_finite() && value > f64::EPSILON).then_some(value)
}

#[cfg(test)]
#[path = "close_run_costs/tests.rs"]
mod tests;
