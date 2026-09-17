use super::*;

pub(crate) fn normalize_replayed_paper_finality(run: &mut CloseRun) -> bool {
    let mut changed = normalize_close_legs(&mut run.legs);
    if let Some(plan) = run.unwind_plan.as_mut() {
        changed |= normalize_compensation_attempts(&mut plan.compensation_attempts);
    }
    if changed {
        refresh_run_summary(run);
    }
    changed
}

fn normalize_close_legs(legs: &mut [CloseLeg]) -> bool {
    let mut changed = false;
    for leg in legs {
        let Some(order) = leg.order.as_mut() else {
            continue;
        };
        if !normalize_legacy_paper_fill(order, leg.quantity) {
            continue;
        }
        leg.status = close_leg_status(order);
        leg.finality_source = close_finality_source(order);
        leg.confirmed_filled_at_ms = confirmed_filled_at_ms_from(
            leg.confirmed_filled_at_ms,
            leg.status,
            order.updated_at_ms,
        );
        leg.problem = close_leg_problem(order, leg.status);
        changed = true;
    }
    changed
}

fn normalize_compensation_attempts(attempts: &mut [CloseRunCompensationAttempt]) -> bool {
    let mut changed = false;
    for attempt in attempts {
        let Some(order) = attempt.order.as_mut() else {
            continue;
        };
        if !normalize_legacy_paper_fill(order, attempt.target_quantity) {
            continue;
        }
        attempt.status = close_leg_status(order);
        attempt.finality_source = close_finality_source(order);
        attempt.confirmed_filled_at_ms = confirmed_filled_at_ms_from(
            attempt.confirmed_filled_at_ms,
            attempt.status,
            order.updated_at_ms,
        );
        attempt.problem = compensation_attempt_problem(order, attempt.status);
        attempt.updated_at_ms = attempt.updated_at_ms.max(order.updated_at_ms);
        changed = true;
    }
    changed
}

fn normalize_legacy_paper_fill(order: &mut OrderRecord, fallback_quantity: f64) -> bool {
    if order.intent.mode != ExecutionMode::DryRun
        || order.state != LiveOrderState::Filled
        || order.last_update_source != OrderUpdateSource::AdapterAck
        || has_fill_evidence(order)
    {
        return false;
    }
    let quantity = finite_positive(order.filled_quantity)
        .or_else(|| finite_positive(Some(fallback_quantity)))
        .or_else(|| finite_positive(Some(order.intent.quantity)));
    let price = finite_positive(order.filled_price).or_else(|| finite_positive(order.intent.price));
    let (Some(quantity), Some(price)) = (quantity, price) else {
        return false;
    };
    order.filled_quantity = Some(quantity);
    order.filled_price = Some(price);
    true
}

fn finite_positive(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0)
}
