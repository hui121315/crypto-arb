use super::*;

pub(super) struct LedgerRunMatch {
    pub(super) run_id: String,
    pub(super) ticket_id: String,
}

impl LedgerRunMatch {
    pub(super) fn from_run(run: &ExecutionRun) -> Self {
        Self {
            run_id: run.run_id.clone(),
            ticket_id: run.ticket_id.clone(),
        }
    }
}

pub(super) fn funding_event_matches_run(
    run: &LedgerRunMatch,
    execution_run: &ExecutionRun,
    event: &ExecutionLedgerEvent,
) -> bool {
    funding_event_matches_leg(run, &execution_run.long_leg, event)
        || funding_event_matches_leg(run, &execution_run.short_leg, event)
}

pub(super) fn funding_event_matches_leg(
    run: &LedgerRunMatch,
    leg: &ExecutionRunLeg,
    event: &ExecutionLedgerEvent,
) -> bool {
    ledger_event_matches_leg(run, leg, event)
        && event.order.exchange == leg.exchange
        && event.order.symbol == leg.symbol
}

pub(super) fn apply_ledger_fill_to_leg(
    run: &LedgerRunMatch,
    leg: &mut ExecutionRunLeg,
    event: &ExecutionLedgerEvent,
    fill: &FillLedgerSnapshot,
) -> bool {
    if !ledger_event_matches_leg(run, leg, event) {
        return false;
    }
    register_ledger_order_ids(leg, event);
    leg.identity = Some(event.order.identity.clone());
    leg.finality_source = Some(event.source);
    leg.filled_quantity = Some(fill_quantity_after_event(leg, fill));
    leg.filled_notional_usd = Some(fill_notional_after_event(leg, fill));
    leg.filled_fee = fill_fee_after_event(leg, fill);
    leg.confirmed_filled_at_ms = confirmed_fill_time_after_event(leg, event);
    if leg_filled_by_quantity(leg) {
        leg.state = LiveOrderState::Filled;
    } else if leg.state != LiveOrderState::Filled {
        leg.state = LiveOrderState::PartiallyFilled;
    }
    true
}

pub(super) fn apply_ledger_state_to_leg(
    run: &LedgerRunMatch,
    leg: &mut ExecutionRunLeg,
    event: &ExecutionLedgerEvent,
    state: LiveOrderState,
) -> LegUpdate {
    if !ledger_event_matches_leg(run, leg, event) {
        return LegUpdate::ignored();
    }
    register_ledger_order_ids(leg, event);
    if ledger_state_would_regress(leg.state, state) {
        return LegUpdate::applied();
    }
    leg.identity = Some(event.order.identity.clone());
    leg.finality_source = ledger_state_finality_source(event, state);
    if state == LiveOrderState::Filled {
        if leg_has_complete_fill(leg) {
            leg.confirmed_filled_at_ms = confirmed_fill_time_after_event(leg, event);
            return LegUpdate::applied();
        }
        leg.state = LiveOrderState::Accepted;
        return LegUpdate::applied_with_problem(ledger_fill_state_without_fill_problem(
            run, leg, event,
        ));
    }
    leg.state = state;
    LegUpdate::applied()
}

fn ledger_state_would_regress(current: LiveOrderState, incoming: LiveOrderState) -> bool {
    if current == incoming {
        return false;
    }
    if is_terminal_state(current) {
        return true;
    }
    state_progress(incoming) < state_progress(current)
}

fn is_terminal_state(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
}

fn state_progress(state: LiveOrderState) -> u8 {
    match state {
        LiveOrderState::Created | LiveOrderState::Unknown => 0,
        LiveOrderState::RiskChecked => 1,
        LiveOrderState::Submitted => 2,
        LiveOrderState::Accepted => 3,
        LiveOrderState::PartiallyFilled => 4,
        LiveOrderState::CancelRequested => 5,
        LiveOrderState::Filled
        | LiveOrderState::Cancelled
        | LiveOrderState::Rejected
        | LiveOrderState::Failed => 6,
    }
}

fn leg_has_complete_fill(leg: &ExecutionRunLeg) -> bool {
    leg_filled_by_quantity(leg)
        && leg
            .filled_notional_usd
            .is_some_and(|notional| notional.is_finite() && notional > 0.0)
        && leg.confirmed_filled_at_ms.is_some()
}

pub(super) fn fill_quantity_after_event(leg: &ExecutionRunLeg, fill: &FillLedgerSnapshot) -> f64 {
    leg.filled_quantity.unwrap_or(0.0) + fill.quantity
}

pub(super) fn fill_notional_after_event(leg: &ExecutionRunLeg, fill: &FillLedgerSnapshot) -> f64 {
    leg.filled_notional_usd.unwrap_or(0.0) + fill.quote_value
}

pub(super) fn fill_fee_after_event(
    leg: &ExecutionRunLeg,
    fill: &FillLedgerSnapshot,
) -> Option<f64> {
    match fill.fee.as_ref().filter(|fee| fee.amount.is_finite()) {
        Some(fee) => Some(leg.filled_fee.unwrap_or(0.0) + fee.amount),
        None => leg.filled_fee,
    }
}

pub(super) fn ledger_state_finality_source(
    event: &ExecutionLedgerEvent,
    state: LiveOrderState,
) -> Option<OrderUpdateSource> {
    matches!(
        state,
        LiveOrderState::PartiallyFilled
            | LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
    .then_some(event.source)
}

pub(super) fn ledger_fill_state_without_fill_problem(
    run: &LedgerRunMatch,
    leg: &ExecutionRunLeg,
    event: &ExecutionLedgerEvent,
) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::HEDGE_EXECUTION_VALUATION_MISSING,
        "ledger order state is filled but fill snapshot is missing",
    )
    .with_status(409)
    .with_source("execution_run_projector");
    problem.details = Some(serde_json::json!({
        "runId": run.run_id,
        "ticketId": run.ticket_id,
        "legRole": leg.role,
        "eventId": event.event_id,
        "eventType": event.event_type,
        "source": event.source,
        "exchange": event.order.exchange,
        "symbol": event.order.symbol,
    }));
    problem
}

pub(super) fn confirmed_fill_time_after_record(
    leg: &ExecutionRunLeg,
    record: &OrderRecord,
) -> Option<i64> {
    execution_valuation::confirmed_fill_time_from_record(leg.confirmed_filled_at_ms, record)
}

pub(super) fn confirmed_fill_time_after_event(
    leg: &ExecutionRunLeg,
    event: &ExecutionLedgerEvent,
) -> Option<i64> {
    latest_positive_time(leg.confirmed_filled_at_ms, event.occurred_at_ms)
}

pub(super) fn latest_positive_time(current: Option<i64>, next: i64) -> Option<i64> {
    if next <= 0 {
        return current;
    }
    Some(current.map_or(next, |current| current.max(next)))
}

pub(super) fn leg_filled_by_quantity(leg: &ExecutionRunLeg) -> bool {
    let Some(filled) = leg.filled_quantity else {
        return false;
    };
    filled >= leg.target_quantity.max(0.0) - f64::EPSILON
}

pub(super) fn ledger_event_matches_leg(
    run: &LedgerRunMatch,
    leg: &ExecutionRunLeg,
    event: &ExecutionLedgerEvent,
) -> bool {
    if event
        .order
        .run_id
        .as_deref()
        .is_some_and(|id| id != run.run_id)
    {
        return false;
    }
    if event
        .order
        .ticket_id
        .as_deref()
        .is_some_and(|id| id != run.ticket_id)
    {
        return false;
    }
    if event.order.leg_role.is_some_and(|role| role != leg.role) {
        return false;
    }
    let has_run_context = event.order.run_id.is_some() || event.order.ticket_id.is_some();
    if has_run_context && event.order.leg_role.is_some() {
        return true;
    }
    ledger_identity_matches_leg(leg, event)
}

pub(super) fn ledger_identity_matches_leg(
    leg: &ExecutionRunLeg,
    event: &ExecutionLedgerEvent,
) -> bool {
    leg.order_ids
        .iter()
        .any(|id| event.order.identity.matches_order_id(id))
}

pub(super) fn register_ledger_order_ids(leg: &mut ExecutionRunLeg, event: &ExecutionLedgerEvent) {
    let identity = &event.order.identity;
    push_order_id(leg, &identity.internal_order_id);
    push_order_id(leg, &identity.public_client_order_id);
    if let Some(venue_client_order_id) = identity.venue_client_order_id.as_deref() {
        push_order_id(leg, venue_client_order_id);
    }
    if let Some(exchange_order_id) = identity.exchange_order_id.as_deref() {
        push_order_id(leg, exchange_order_id);
    }
}
