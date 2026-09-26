use shared_types::{ExecutionRun, ExecutionRunLeg, ExecutionRunState, LiveOrderState, OrderUpdateSource};

pub(crate) fn has_unfilled_outcome(run: &ExecutionRun) -> bool {
    run.state == ExecutionRunState::FailedSafe && run.orders_ended_without_fills()
}

pub(crate) fn has_completed_recovery(run: &ExecutionRun) -> bool {
    if run.state != ExecutionRunState::Closed || run.evidence.recovery_orders.is_empty()
        || run.unwind_problem.is_some() || run.valuation_problem.is_some() || run.finality_problem.is_some() {
        return false;
    }
    if run.evidence.recovery_orders.iter().any(|order| order.problem.is_some()
        || !matches!(order.state, LiveOrderState::Filled | LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed)
        || !matches!(order.source, Some(OrderUpdateSource::PrivateWs | OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile))
        || !order.filled_quantity.is_some_and(|quantity| quantity.is_finite() && quantity >= 0.0)) {
        return false;
    }
    [&run.long_leg, &run.short_leg].into_iter().all(|leg| {
        if !matches!(leg.state, LiveOrderState::Filled | LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed) {
            return false;
        }
        let quantity = leg.filled_quantity.or_else(|| (leg.state == LiveOrderState::Failed
            && leg.finality_source == Some(OrderUpdateSource::Internal)).then_some(0.0));
        quantity.is_some_and(|quantity| {
            let closed: f64 = run.evidence.recovery_orders.iter().filter(|order| order.role == leg.role)
                .filter_map(|order| order.filled_quantity).sum();
            quantity.is_finite() && quantity >= 0.0
                && (quantity - closed).abs() <= f64::EPSILON * 32.0 * quantity.max(1.0)
        })
    })
}

pub(crate) fn cancellation_exposure_unconfirmed(run: &ExecutionRun) -> bool {
    if has_completed_recovery(run) { return false; }
    if run.evidence.recovery_orders.iter().any(|order| order.problem.is_some()
        || !order.filled_quantity.is_some_and(|quantity| quantity.is_finite() && quantity >= 0.0)) {
        return true;
    }
    let legs = [&run.long_leg, &run.short_leg];
    legs.iter().any(|leg| matches!(leg.state, LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed))
        && (run.valuation_problem.is_some() || legs.iter().any(|leg| {
            !leg.filled_quantity.is_some_and(|value| value.is_finite() && value >= 0.0)
                || !matches!(leg.finality_source, Some(OrderUpdateSource::PrivateWs | OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile))
        }))
}

// A status tag is not a fill receipt. Source/mode checks remain with the caller.
pub(crate) fn has_recorded_fill(leg: &ExecutionRunLeg) -> bool {
    leg.state == LiveOrderState::Filled
        && leg.filled_quantity.is_some_and(|quantity| quantity.is_finite() && quantity > 0.0)
        && leg.confirmed_filled_at_ms.is_some_and(|time| time > 0)
}
