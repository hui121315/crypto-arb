use super::*;

pub(super) fn update_state(run: &mut ExecutionRun) {
    if !run.evidence.recovery_orders.is_empty() {
        refresh_recovery(run);
        return;
    }
    if run.valuation_problem.is_some() {
        run.state = ExecutionRunState::UnwindRequired;
        run.recovery_action = Some(RecoveryAction::ManualReview);
    } else if both_filled(run) {
        run.state = ExecutionRunState::Hedged;
        run.recovery_action = None;
    } else if run.orders_ended_without_fills() {
        run.state = ExecutionRunState::FailedSafe;
        run.recovery_action = None;
    } else if terminal_failure(run) {
        run.state = ExecutionRunState::UnwindRequired;
        let legs = [&run.long_leg, &run.short_leg];
        run.recovery_action = Some(if legs.iter().any(|leg| !matches!(leg.state,
            LiveOrderState::Filled | LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed)) {
            RecoveryAction::CancelOpenOrders
        } else if legs.iter().all(|leg| leg.filled_quantity.is_some_and(|value| value.is_finite() && value >= 0.0)) {
            recovery_action(run).unwrap_or(RecoveryAction::ManualReview)
        } else {
            RecoveryAction::ManualReview
        });
    }
}

pub(super) fn both_filled(run: &ExecutionRun) -> bool {
    run.long_leg.state == LiveOrderState::Filled && run.short_leg.state == LiveOrderState::Filled
}

pub(super) fn terminal_failure(run: &ExecutionRun) -> bool {
    leg_failed(run.long_leg.state) || leg_failed(run.short_leg.state)
}

pub(super) fn leg_failed(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Rejected | LiveOrderState::Failed | LiveOrderState::Cancelled
    )
}

pub(super) fn recovery_action(run: &ExecutionRun) -> Option<RecoveryAction> {
    if run.net_exposure_usd > 0.0 {
        Some(RecoveryAction::UnwindLongLeg)
    } else if run.net_exposure_usd < 0.0 {
        Some(RecoveryAction::UnwindShortLeg)
    } else {
        None
    }
}

pub(super) fn update_exposure(run: &mut ExecutionRun) {
    run.net_exposure_usd = execution_valuation::net_base_exposure_usd(run);
}

pub(super) fn exact_fill_quantity(record: &OrderRecord) -> Option<f64> {
    record
        .filled_quantity
        .filter(|quantity| quantity.is_finite() && *quantity > 0.0)
}

pub(super) fn run_matches_order_id(run: &ExecutionRun, order_id: &str) -> bool {
    leg_matches_order_id(&run.long_leg, order_id) || leg_matches_order_id(&run.short_leg, order_id)
}

pub(super) fn leg_matches_order(leg: &ExecutionRunLeg, record: &OrderRecord) -> bool {
    if leg.exchange != record.intent.exchange || leg.symbol != record.intent.symbol {
        return false;
    }
    if let Some(expected) = leg.identity.as_ref() {
        let incoming = record.identity_snapshot();
        if expected.account_scope.is_some() && incoming.account_scope.is_some()
            && expected.account_scope != incoming.account_scope {
            return false;
        }
        if let Some(matches) = strong_identity_match(expected, &incoming) {
            return matches;
        }
    }
    leg.order_ids.iter().any(|id| order_id_matches(record, id))
}

fn strong_identity_match(
    expected: &shared_types::VenueOrderIdentity,
    incoming: &shared_types::VenueOrderIdentity,
) -> Option<bool> {
    let pairs = [
        (
            expected.internal_order_id.as_str(),
            incoming.internal_order_id.as_str(),
        ),
        (
            expected.public_client_order_id.as_str(),
            incoming.public_client_order_id.as_str(),
        ),
        (
            expected.venue_client_order_id.as_deref().unwrap_or(""),
            incoming.venue_client_order_id.as_deref().unwrap_or(""),
        ),
    ];
    let mut compared = false;
    for (expected, incoming) in pairs {
        if expected.trim().is_empty() || incoming.trim().is_empty() {
            continue;
        }
        compared = true;
        if expected == incoming {
            return Some(true);
        }
    }
    compared.then_some(false)
}

pub(super) fn leg_matches_order_id(leg: &ExecutionRunLeg, order_id: &str) -> bool {
    !order_id.is_empty() && leg.order_ids.iter().any(|id| id == order_id)
}

pub(super) fn order_id_matches(record: &OrderRecord, id: &str) -> bool {
    record.identity_snapshot().matches_order_id(id)
}

pub(super) fn register_order_ids(leg: &mut ExecutionRunLeg, record: &OrderRecord) {
    let identity = record.identity_snapshot();
    push_order_id(leg, &identity.internal_order_id);
    push_order_id(leg, &identity.public_client_order_id);
    if let Some(venue_client_order_id) = identity.venue_client_order_id.as_deref() {
        push_order_id(leg, venue_client_order_id);
    }
    if let Some(exchange_order_id) = identity.exchange_order_id.as_deref() {
        push_order_id(leg, exchange_order_id);
    }
}

pub(super) fn push_order_id(leg: &mut ExecutionRunLeg, order_id: &str) {
    if !order_id.is_empty() && !leg.order_ids.iter().any(|id| id == order_id) {
        leg.order_ids.push(order_id.to_owned());
    }
}

pub(super) fn record_finality_source(record: &OrderRecord) -> Option<OrderUpdateSource> {
    matches!(
        record.state,
        LiveOrderState::PartiallyFilled
            | LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
    .then_some(record.last_update_source)
}

pub(super) fn status_reason(run: &ExecutionRun) -> &str {
    if !run.evidence.recovery_orders.is_empty() { return &run.status_reason; }
    if let Some(problem) = &run.valuation_problem {
        return &problem.message;
    }
    if run.orders_ended_without_fills() {
        return "订单已结束，未成交；本次交易没有产生持仓";
    }
    match run.state {
        ExecutionRunState::Hedged => "已收到两边的成交记录",
        ExecutionRunState::UnwindRequired if run.recovery_action == Some(RecoveryAction::CancelOpenOrders) =>
            "一边订单已结束，请先撤销并核对其余挂单",
        ExecutionRunState::UnwindRequired if [&run.long_leg, &run.short_leg].into_iter()
            .any(|leg| leg.filled_quantity.is_none()) => "订单已结束，但成交数量尚未确认，请核对订单和持仓",
        ExecutionRunState::UnwindRequired if run.net_exposure_usd.abs() <= f64::EPSILON =>
            "订单结果仍需核对；未对冲金额为零不等于已经平仓",
        ExecutionRunState::UnwindRequired => "订单已结束，仍有未对冲持仓；撤单不等于平仓",
        _ => "订单结果已更新",
    }
}
