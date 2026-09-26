use super::*;
use shared_types::{ExecutionRecoveryOrder, HedgeLegRole, OrderSide};

fn opposite(role: HedgeLegRole) -> OrderSide {
    match role { HedgeLegRole::Long => OrderSide::Sell, HedgeLegRole::Short => OrderSide::Buy }
}

pub(super) fn recovery_record_role(run: &ExecutionRun, record: &OrderRecord) -> Option<HedgeLegRole> {
    [&run.long_leg, &run.short_leg].into_iter().find(|leg| {
        leg.exchange == record.intent.exchange && leg.symbol == record.intent.symbol
            && record.intent.side == opposite(leg.role)
            && leg.order_ids.iter().any(|id| order_id_matches(record, id))
            && leg.identity.as_ref().is_none_or(|identity| {
                let incoming = record.identity_snapshot();
                identity.account_scope.is_none() || incoming.account_scope.is_none()
                    || identity.account_scope == incoming.account_scope
            })
    }).map(|leg| leg.role)
}

fn recovery_event_role(run: &ExecutionRun, event: &ExecutionLedgerEvent) -> Option<HedgeLegRole> {
    let context = LedgerRunMatch::from_run(run);
    [&run.long_leg, &run.short_leg].into_iter().find(|leg|
        event.order.side == opposite(leg.role) && ledger_event_matches_leg(&context, leg, event)
    ).map(|leg| leg.role)
}

fn order_progress<'a>(run: &'a mut ExecutionRun, id: &str, role: HedgeLegRole) -> &'a mut ExecutionRecoveryOrder {
    let rows = &mut run.evidence.recovery_orders;
    let index = rows.iter().position(|row| row.order_id == id).unwrap_or_else(|| {
        rows.push(ExecutionRecoveryOrder { order_id: id.into(), role,
            state: LiveOrderState::Unknown, filled_quantity: None, source: None,
            ledger_fills: None, problem: None });
        rows.len() - 1
    });
    &mut rows[index]
}

fn proven_source(source: OrderUpdateSource) -> bool {
    matches!(source, OrderUpdateSource::PrivateWs | OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile)
}

fn problem(message: &str) -> ApiProblem {
    ApiProblem::new(codes::HEDGE_UNWIND_FINALITY_FAILED, message)
        .with_status(409).with_source("execution_run_projector")
}

pub(super) fn apply_recovery_order_update(run: &mut ExecutionRun, record: &OrderRecord) -> bool {
    let Some(role) = recovery_record_role(run, record) else { return false; };
    let terminal_problem = leg_failed(record.state).then(|| recovery_finality_problem(run, record));
    let row = order_progress(run, &record.intent.id, role);
    if ledger_state_would_regress(row.state, record.state) && !terminal(record.state) {
        return false;
    }
    if !ledger_state_would_regress(row.state, record.state) { row.state = record.state; }
    row.source = Some(record.last_update_source);
    let valid_source = proven_source(record.last_update_source)
        || (record.intent.mode == shared_types::ExecutionMode::DryRun
            && record.last_update_source == OrderUpdateSource::AdapterAck);
    match record.filled_quantity {
        Some(quantity) if valid_source && quantity.is_finite() && quantity >= 0.0
            && row.filled_quantity.is_none_or(|known| quantity >= known) => {
                row.filled_quantity = Some(quantity);
                row.problem = None;
            }
        _ => row.problem = Some(problem("补救订单的成交数量尚未核清，不能认定持仓已处理完")),
    }
    // A cancelled remainder does not invalidate confirmed fills or a later replacement.
    if row.problem.is_some() {
        if let Some(problem) = terminal_problem { row.problem = Some(problem); }
    }
    refresh_recovery(run);
    true
}

pub(super) fn apply_recovery_event(run: &mut ExecutionRun, event: &ExecutionLedgerEvent) -> bool {
    let Some(role) = recovery_event_role(run, event) else { return false; };
    let id = &event.order.identity.internal_order_id;
    if id.is_empty() { return false; }
    register_ledger_order_ids(match role {
        HedgeLegRole::Long => &mut run.long_leg, HedgeLegRole::Short => &mut run.short_leg,
    }, event);
    let row = order_progress(run, id, role);
    if let Some(fill) = ledger_fill_event(event) {
        if !proven_source(event.source) { return false; }
        let total = accumulate_fill(&mut row.ledger_fills, fill);
        row.filled_quantity = Some(row.filled_quantity.unwrap_or(0.0).max(total.quantity));
        row.source = Some(event.source);
        row.problem = None;
    } else if let Some(state) = ledger_order_state_event(event) {
        if ledger_state_would_regress(row.state, state) { return false; }
        row.state = state;
        row.source = Some(event.source);
        if !proven_source(event.source) || row.filled_quantity.is_none() {
            row.problem = Some(problem("补救订单已有状态，但成交数量尚未核清，请等待查询结果"));
        }
    } else { return false; }
    refresh_recovery(run);
    true
}

fn terminal(state: LiveOrderState) -> bool {
    matches!(state, LiveOrderState::Filled | LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed)
}

fn opening_quantity(leg: &ExecutionRunLeg) -> Option<f64> {
    leg.filled_quantity.filter(|value| value.is_finite() && *value >= 0.0).or_else(|| {
        // Only the orchestrator's explicit never-submitted failure can prove this zero.
        (leg.state == LiveOrderState::Failed && leg.finality_source == Some(OrderUpdateSource::Internal)).then_some(0.0)
    })
}

pub(super) fn refresh_recovery(run: &mut ExecutionRun) {
    if run.evidence.recovery_orders.is_empty() { return; }
    let quantities = opening_quantity(&run.long_leg).zip(opening_quantity(&run.short_leg));
    let mut quantities_known = false;
    let mut all_flat = false;
    if let Some((long, short)) = quantities {
        let remaining = |role, opened| opened - run.evidence.recovery_orders.iter()
            .filter(|order| order.role == role)
            .filter_map(|order| order.filled_quantity.filter(|value| value.is_finite() && *value >= 0.0))
            .sum::<f64>();
        let left = remaining(HedgeLegRole::Long, long);
        let right = remaining(HedgeLegRole::Short, short);
        let tolerance = f64::EPSILON * 32.0 * long.max(short).max(1.0);
        quantities_known = left >= -tolerance && right >= -tolerance
            && run.evidence.recovery_orders.iter().all(|order| order.filled_quantity
                .is_some_and(|value| value.is_finite() && value >= 0.0));
        all_flat = left.abs() <= tolerance && right.abs() <= tolerance;
        if let Some(price) = execution_valuation::exposure_mark_price(run) {
            run.net_exposure_usd = (left - right) * price;
        } else if all_flat { run.net_exposure_usd = 0.0; }
        else { quantities_known = false; }
    }
    let failure = run.evidence.recovery_orders.iter().find_map(|order| order.problem.clone());
    let original_done = [&run.long_leg, &run.short_leg].into_iter().all(|leg| terminal(leg.state));
    let recovery_done = run.evidence.recovery_orders.iter().all(|order| terminal(order.state));
    if quantities_known && all_flat && original_done && recovery_done && failure.is_none()
        && run.valuation_problem.is_none() && run.finality_problem.is_none() {
        run.state = ExecutionRunState::Closed;
        run.recovery_action = None;
        run.unwind_problem = None;
        run.status_reason = "补救订单成交数量已核对，本次持仓已处理完".into();
    } else if failure.is_some() || !quantities_known {
        run.state = ExecutionRunState::UnwindRequired;
        run.recovery_action = Some(RecoveryAction::ManualReview);
        let failure = failure.unwrap_or_else(|| problem("开仓或补救数量尚未核清，请核对订单和剩余持仓"));
        run.status_reason = failure.message.clone();
        run.unwind_problem = Some(failure);
    } else if !original_done || !recovery_done {
        run.state = ExecutionRunState::Unwinding;
        run.recovery_action = Some(RecoveryAction::CancelOpenOrders);
        run.unwind_problem = None;
        run.status_reason = "补救处理中，仍需等待其余订单的最终结果".into();
    } else {
        run.state = ExecutionRunState::UnwindRequired;
        run.recovery_action = recovery_action(run).or(Some(RecoveryAction::ManualReview));
        run.unwind_problem = None;
        run.status_reason = "补救订单已结束，仍有持仓未处理完，请到持仓页核对".into();
    }
    refresh_cost_reconciliation(run);
}
