use super::*;

pub(super) fn collect_pending_order_refs(state: &AppState) -> Vec<PendingOrderRef> {
    let mut refs = PendingOrderMap::new();
    for run in state.execution_runs().iter() {
        collect_execution_run_orders(run.value(), &mut refs);
    }
    for run in state.close_runs().iter() {
        collect_close_run_orders(run.value(), &mut refs);
    }
    refs.into_iter()
        .map(|(order_id, source)| PendingOrderRef { order_id, source })
        .collect()
}

pub(super) fn collect_execution_run_orders(run: &ExecutionRun, refs: &mut PendingOrderMap) {
    collect_execution_leg_orders(&run.long_leg, refs);
    collect_execution_leg_orders(&run.short_leg, refs);
}

fn collect_execution_leg_orders(leg: &ExecutionRunLeg, refs: &mut PendingOrderMap) {
    if is_terminal_order_state(leg.state) {
        return;
    }
    for order_id in &leg.order_ids {
        insert_pending_order(refs, order_id, PendingOrderSource::ExecutionRun);
    }
}

pub(super) fn collect_close_run_orders(run: &CloseRun, refs: &mut PendingOrderMap) {
    for leg in &run.legs {
        if let Some(order) = leg.order.as_ref() {
            collect_order_record(
                &order.intent.id,
                order.state,
                PendingOrderSource::CloseRun,
                refs,
            );
        }
    }
    if let Some(plan) = run.unwind_plan.as_ref() {
        collect_compensation_orders(plan, refs);
    }
}

fn collect_compensation_orders(plan: &CloseRunUnwindPlan, refs: &mut PendingOrderMap) {
    for attempt in &plan.compensation_attempts {
        collect_compensation_order(attempt, refs);
    }
}

fn collect_compensation_order(attempt: &CloseRunCompensationAttempt, refs: &mut PendingOrderMap) {
    if let Some(order) = attempt.order.as_ref() {
        collect_order_record(
            &order.intent.id,
            order.state,
            PendingOrderSource::CloseRun,
            refs,
        );
    }
}

fn collect_order_record(
    order_id: &str,
    state: LiveOrderState,
    source: PendingOrderSource,
    refs: &mut PendingOrderMap,
) {
    if !is_terminal_order_state(state) {
        insert_pending_order(refs, order_id, source);
    }
}

fn insert_pending_order(refs: &mut PendingOrderMap, order_id: &str, source: PendingOrderSource) {
    refs.entry(order_id.to_owned())
        .and_modify(|existing| {
            if *existing != source {
                *existing = PendingOrderSource::Mixed;
            }
        })
        .or_insert(source);
}

pub(super) fn is_terminal_order_state(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
}
