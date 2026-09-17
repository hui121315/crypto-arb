use super::*;

mod transitions;

pub(super) use transitions::*;

pub(super) fn apply_ledger_event_update(
    run: &mut CloseRun,
    event: &ExecutionLedgerEvent,
    update: &CloseLedgerUpdate<'_>,
) -> bool {
    let mut changed = false;
    for leg in &mut run.legs {
        changed |= apply_ledger_event_to_leg(leg, event, update);
    }
    changed |= apply_ledger_event_to_compensation(run, event, update);
    if changed {
        refresh_run_summary(run);
    }
    changed
}

pub(super) fn apply_close_ledger_event(
    run: &mut CloseRun,
    event: &ExecutionLedgerEvent,
    update: Option<&CloseLedgerUpdate<'_>>,
) -> bool {
    let changed = if let Some(update) = update {
        apply_ledger_event_update(run, event, update)
    } else if matches!(event.payload, ExecutionLedgerPayload::FeeSnapshot(_)) {
        apply_ledger_fee_event_update(run, event)
    } else if matches!(event.payload, ExecutionLedgerPayload::FundingPayment(_)) {
        apply_ledger_funding_event_update(run, event)
    } else {
        apply_ledger_slippage_event_update(run, event)
    };
    if changed {
        refresh_cost_reconciliation(run);
    }
    changed
}

pub(super) fn apply_ledger_fee_event_update(
    run: &mut CloseRun,
    event: &ExecutionLedgerEvent,
) -> bool {
    let Some(fee) = ledger_fee_amount(event) else {
        return false;
    };
    let mut changed = false;
    for leg in &mut run.legs {
        changed |= apply_ledger_fee_to_leg(leg, event, fee);
    }
    changed |= apply_ledger_fee_to_compensation(run, event, fee);
    if changed {
        refresh_run_summary(run);
    }
    changed
}

pub(super) fn apply_ledger_fee_to_leg(
    leg: &mut CloseLeg,
    event: &ExecutionLedgerEvent,
    fee: f64,
) -> bool {
    let Some(order) = leg.order.as_mut() else {
        return false;
    };
    if !apply_ledger_fee_to_order(order, event, fee) {
        return false;
    }
    record_cost_event(
        &mut leg.cost_events,
        ledger_cost_event(event, CloseRunCostComponent::Fee, fee),
    );
    true
}

pub(super) fn apply_ledger_fee_to_compensation(
    run: &mut CloseRun,
    event: &ExecutionLedgerEvent,
    fee: f64,
) -> bool {
    let Some(plan) = run.unwind_plan.as_mut() else {
        return false;
    };
    let mut changed = false;
    for attempt in &mut plan.compensation_attempts {
        if let Some(order) = attempt.order.as_mut() {
            if apply_ledger_fee_to_order(order, event, fee) {
                record_cost_event(
                    &mut attempt.cost_events,
                    ledger_cost_event(event, CloseRunCostComponent::Fee, fee),
                );
                changed = true;
            }
        }
    }
    changed
}

pub(super) fn apply_ledger_fee_to_order(
    order: &mut OrderRecord,
    event: &ExecutionLedgerEvent,
    fee: f64,
) -> bool {
    if !order_matches_ledger_event(order, event) {
        return false;
    }
    apply_ledger_identity(order, event);
    order.filled_fee = Some(fee);
    order.last_update_source = event.source;
    order.updated_at_ms = ledger_update_time(event);
    true
}

pub(super) fn ledger_fee_amount(event: &ExecutionLedgerEvent) -> Option<f64> {
    let ExecutionLedgerPayload::FeeSnapshot(fee) = &event.payload else {
        return None;
    };
    (fee.quality == ExecutionLedgerQuality::Actual && fee.amount.is_finite()).then_some(fee.amount)
}

pub(super) fn apply_ledger_funding_event_update(
    run: &mut CloseRun,
    event: &ExecutionLedgerEvent,
) -> bool {
    let Some(cost_event) = ledger_funding_cost_event(event) else {
        return false;
    };
    if !run_matches_funding_event(run, event) {
        return false;
    }
    run.cost_events
        .retain(|existing| !existing.event_id.starts_with("paper-funding-model:"));
    record_cost_event(&mut run.cost_events, cost_event)
}

pub(super) fn apply_ledger_slippage_event_update(
    run: &mut CloseRun,
    event: &ExecutionLedgerEvent,
) -> bool {
    let Some(cost_event) = ledger_slippage_cost_event(event) else {
        return false;
    };
    let mut changed = false;
    for leg in &mut run.legs {
        changed |= apply_ledger_slippage_to_leg(leg, event, &cost_event);
    }
    changed |= apply_ledger_slippage_to_compensation(run, event, &cost_event);
    changed
}

fn apply_ledger_slippage_to_leg(
    leg: &mut CloseLeg,
    event: &ExecutionLedgerEvent,
    cost_event: &CloseRunCostLedgerEvent,
) -> bool {
    let Some(order) = leg.order.as_mut() else {
        return false;
    };
    if !order_matches_ledger_event(order, event) {
        return false;
    }
    apply_ledger_identity(order, event);
    order.last_update_source = event.source;
    order.updated_at_ms = ledger_update_time(event);
    record_cost_event(&mut leg.cost_events, cost_event.clone())
}

fn apply_ledger_slippage_to_compensation(
    run: &mut CloseRun,
    event: &ExecutionLedgerEvent,
    cost_event: &CloseRunCostLedgerEvent,
) -> bool {
    let Some(plan) = run.unwind_plan.as_mut() else {
        return false;
    };
    let mut changed = false;
    for attempt in &mut plan.compensation_attempts {
        let Some(order) = attempt.order.as_mut() else {
            continue;
        };
        if !order_matches_ledger_event(order, event) {
            continue;
        }
        apply_ledger_identity(order, event);
        order.last_update_source = event.source;
        order.updated_at_ms = ledger_update_time(event);
        attempt.updated_at_ms = order.updated_at_ms;
        changed |= record_cost_event(&mut attempt.cost_events, cost_event.clone());
    }
    changed
}

pub(super) fn apply_leg_update(leg: &mut CloseLeg, record: &OrderRecord) -> bool {
    if !leg_matches_record(leg, record) {
        return false;
    }
    let previous = leg.clone();
    leg.order = Some(record.clone());
    leg.status = close_leg_status(record);
    leg.finality_source = close_finality_source(record);
    leg.confirmed_filled_at_ms = confirmed_filled_at_ms(leg, record);
    leg.problem = close_leg_problem(record, leg.status);
    *leg != previous
}

pub(super) fn apply_ledger_event_to_leg(
    leg: &mut CloseLeg,
    event: &ExecutionLedgerEvent,
    update: &CloseLedgerUpdate<'_>,
) -> bool {
    if !leg_matches_ledger_event(leg, event) {
        return false;
    }
    let target_quantity = leg.quantity;
    let previous_confirmed_at_ms = leg.confirmed_filled_at_ms;
    let Some(order) = leg.order.as_mut() else {
        return false;
    };
    let mut effective_update = *update;
    if replaces_paper_adapter_fill(order, event, update.fill) {
        effective_update.incremental_fill = false;
        remove_paper_adapter_cost_events(&mut leg.cost_events);
    }
    apply_ledger_identity(order, event);
    apply_ledger_order_update(order, target_quantity, event, &effective_update);
    record_fill_fee_cost_event(&mut leg.cost_events, event, effective_update.fill);
    record_paper_fill_slippage_cost_event(
        &mut leg.cost_events,
        order,
        event,
        effective_update.fill,
    );
    leg.status = close_leg_status(order);
    leg.finality_source = close_finality_source(order);
    leg.confirmed_filled_at_ms = confirmed_filled_at_ms_from_ledger(
        previous_confirmed_at_ms,
        leg.status,
        order.intent.mode,
        event,
    );
    leg.problem = close_leg_problem(order, leg.status);
    true
}

pub(super) fn apply_compensation_order_update(run: &mut CloseRun, record: &OrderRecord) -> bool {
    let Some(plan) = run.unwind_plan.as_mut() else {
        return false;
    };
    if let Some(attempt) = plan
        .compensation_attempts
        .iter_mut()
        .find(|attempt| attempt_matches_record(attempt, record))
    {
        let previous = attempt.clone();
        update_compensation_attempt_from_order(attempt, record);
        return *attempt != previous;
    }
    false
}

pub(super) fn apply_ledger_event_to_compensation(
    run: &mut CloseRun,
    event: &ExecutionLedgerEvent,
    update: &CloseLedgerUpdate<'_>,
) -> bool {
    let Some(plan) = run.unwind_plan.as_mut() else {
        return false;
    };
    let mut changed = false;
    for attempt in &mut plan.compensation_attempts {
        changed |= apply_ledger_event_to_compensation_attempt(attempt, event, update);
    }
    changed
}
