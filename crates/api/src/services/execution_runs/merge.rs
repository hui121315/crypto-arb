use super::*;

// A local orchestration copy owns the plan, not newer exchange results.
pub(super) fn merge_recorded_progress(run: &mut ExecutionRun, saved: &ExecutionRun) {
    let long_changed = merge_leg(&mut run.long_leg, &saved.long_leg);
    let short_changed = merge_leg(&mut run.short_leg, &saved.short_leg);
    if saved.finality_checked_at_ms > run.finality_checked_at_ms {
        run.finality_checked_at_ms = saved.finality_checked_at_ms;
        run.finality_problem = saved.finality_problem.clone();
    }
    if let Some(saved_cost) = &saved.cost_reconciliation {
        let cost = run
            .cost_reconciliation
            .get_or_insert_with(|| saved_cost.clone());
        if !saved_cost.funding_event_ids.is_empty() {
            cost.funding_event_ids = saved_cost.funding_event_ids.clone();
            cost.actual_funding_usd = saved_cost.actual_funding_usd;
        }
        if !saved_cost.unwind_event_ids.is_empty() {
            cost.unwind_event_ids = saved_cost.unwind_event_ids.clone();
            cost.actual_unwind_fee_usd = saved_cost.actual_unwind_fee_usd;
            cost.actual_unwind_slippage_usd = saved_cost.actual_unwind_slippage_usd;
            cost.actual_unwind_cost_usd = saved_cost.actual_unwind_cost_usd;
        }
    }
    if saved.state == ExecutionRunState::Closed {
        run.state = saved.state;
        run.net_exposure_usd = saved.net_exposure_usd;
        run.recovery_action = saved.recovery_action;
        run.unwind_problem = saved.unwind_problem.clone();
        run.status_reason = saved.status_reason.clone();
    } else if long_changed || short_changed {
        run.valuation_problem = run
            .valuation_problem
            .clone()
            .or_else(|| saved.valuation_problem.clone());
        update_exposure(run);
        if !matches!(
            run.state,
            ExecutionRunState::Unwinding
                | ExecutionRunState::UnwindRequired
                | ExecutionRunState::Closed
        ) {
            update_state(run);
            run.status_reason = status_reason(run).to_owned();
        }
    }
    run.updated_at_ms = run.updated_at_ms.max(saved.updated_at_ms);
    refresh_cost_reconciliation(run);
}

fn merge_leg(incoming: &mut ExecutionRunLeg, saved: &ExecutionRunLeg) -> bool {
    if incoming.role != saved.role
        || incoming.exchange != saved.exchange
        || incoming.symbol != saved.symbol
    {
        return false;
    }
    let before = incoming.clone();
    for id in &saved.order_ids {
        push_order_id(incoming, id);
    }
    let valid = |quantity: Option<f64>| quantity.filter(|value| value.is_finite() && *value >= 0.0);
    let preserve_quantity = valid(saved.filled_quantity).is_some_and(|known| {
        valid(incoming.filled_quantity).is_none_or(|quantity| known > quantity)
    });
    let same_quantity =
        valid(saved.filled_quantity).is_some() && saved.filled_quantity == incoming.filled_quantity;
    let preserve_state = ledger_state_would_regress(saved.state, incoming.state);
    if preserve_quantity {
        incoming.filled_quantity = saved.filled_quantity;
        incoming.filled_notional_usd = saved.filled_notional_usd;
        incoming.filled_fee = saved.filled_fee;
    } else if same_quantity {
        incoming.filled_notional_usd = incoming.filled_notional_usd.or(saved.filled_notional_usd);
        incoming.filled_fee = incoming.filled_fee.or(saved.filled_fee);
    }
    if preserve_state {
        incoming.state = saved.state;
    }
    if preserve_quantity || preserve_state {
        incoming.finality_source = saved.finality_source.or(incoming.finality_source);
        incoming.identity = saved.identity.clone().or_else(|| incoming.identity.clone());
    }
    if let Some(time) = saved.confirmed_filled_at_ms {
        incoming.confirmed_filled_at_ms =
            latest_positive_time(incoming.confirmed_filled_at_ms, time);
    }
    *incoming != before
}
