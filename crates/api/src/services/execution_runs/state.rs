use super::*;

pub(super) fn update_state(run: &mut ExecutionRun) {
    if both_filled(run) {
        run.state = ExecutionRunState::Hedged;
        run.recovery_action = None;
    } else if terminal_failure(run) && run.net_exposure_usd.abs() > f64::EPSILON {
        run.state = ExecutionRunState::UnwindRequired;
        run.recovery_action = recovery_action(run);
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

pub(super) fn run_matches_order(run: &ExecutionRun, record: &OrderRecord) -> bool {
    leg_matches_order(&run.long_leg, record) || leg_matches_order(&run.short_leg, record)
}

pub(super) fn run_matches_order_id(run: &ExecutionRun, order_id: &str) -> bool {
    leg_matches_order_id(&run.long_leg, order_id) || leg_matches_order_id(&run.short_leg, order_id)
}

pub(super) fn leg_matches_order(leg: &ExecutionRunLeg, record: &OrderRecord) -> bool {
    if let Some(expected) = leg.identity.as_ref() {
        let incoming = record.identity_snapshot();
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

pub(super) fn status_reason(state: ExecutionRunState) -> &'static str {
    match state {
        ExecutionRunState::Hedged => "订单回填确认双腿成交",
        ExecutionRunState::UnwindRequired => "订单回填发现裸露风险，需要补偿",
        _ => "订单回填已更新执行状态",
    }
}
