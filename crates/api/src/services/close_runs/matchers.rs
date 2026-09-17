use super::*;

pub(super) fn apply_ledger_event_to_compensation_attempt(
    attempt: &mut CloseRunCompensationAttempt,
    event: &ExecutionLedgerEvent,
    update: &CloseLedgerUpdate<'_>,
) -> bool {
    let previous_confirmed_at_ms = attempt.confirmed_filled_at_ms;
    let Some(order) = attempt.order.as_mut() else {
        return false;
    };
    if !order_matches_ledger_event(order, event) {
        return false;
    }
    let mut effective_update = *update;
    if replaces_paper_adapter_fill(order, event, update.fill) {
        effective_update.incremental_fill = false;
        remove_paper_adapter_cost_events(&mut attempt.cost_events);
    }
    apply_ledger_identity(order, event);
    apply_ledger_order_update(order, attempt.target_quantity, event, &effective_update);
    record_fill_fee_cost_event(&mut attempt.cost_events, event, effective_update.fill);
    record_paper_fill_slippage_cost_event(
        &mut attempt.cost_events,
        order,
        event,
        effective_update.fill,
    );
    attempt.status = close_leg_status(order);
    attempt.finality_source = close_finality_source(order);
    attempt.confirmed_filled_at_ms = confirmed_filled_at_ms_from(
        previous_confirmed_at_ms,
        attempt.status,
        confirmed_fill_time_from_ledger(order.intent.mode, event),
    );
    attempt.problem = compensation_attempt_problem(order, attempt.status);
    attempt.updated_at_ms = order.updated_at_ms;
    true
}

pub(super) fn leg_matches_record(leg: &CloseLeg, record: &OrderRecord) -> bool {
    leg.order
        .as_ref()
        .is_some_and(|order| order_matches_record(order, record))
}

pub(super) fn leg_matches_ledger_event(leg: &CloseLeg, event: &ExecutionLedgerEvent) -> bool {
    leg.order
        .as_ref()
        .is_some_and(|order| order_matches_ledger_event(order, event))
}

pub(super) fn order_matches_record(order: &OrderRecord, record: &OrderRecord) -> bool {
    match strong_record_identity_match(order, record) {
        Some(matches) => matches,
        None => {
            order_identity_matches_record(order, record)
                || record_identity_matches_order(order, record)
        }
    }
}

pub(super) fn order_matches_ledger_event(
    order: &OrderRecord,
    event: &ExecutionLedgerEvent,
) -> bool {
    let identity = order.identity_snapshot();
    ledger_market_matches(order, event) && identity_matches_ledger(&identity, &event.order.identity)
}

pub(super) fn attempt_matches_record(
    attempt: &CloseRunCompensationAttempt,
    record: &OrderRecord,
) -> bool {
    attempt
        .order
        .as_ref()
        .is_some_and(|order| order_matches_record(order, record))
}

pub(super) fn run_matches_record(run: &CloseRun, record: &OrderRecord) -> bool {
    run.legs.iter().any(|leg| leg_matches_record(leg, record))
        || run_compensation_matches_record(run, record)
}

pub(super) fn run_matches_funding_event(run: &CloseRun, event: &ExecutionLedgerEvent) -> bool {
    run.legs
        .iter()
        .any(|leg| leg_matches_funding_event(leg, event))
}

fn leg_matches_funding_event(leg: &CloseLeg, event: &ExecutionLedgerEvent) -> bool {
    let Some(pair) = leg.pair_evidence.as_ref() else {
        return false;
    };
    pair_context_matches_event(pair, event)
        && venue_names_equal(&pair.venue, &event.order.exchange)
        && pair.symbol.eq_ignore_ascii_case(&event.order.symbol)
        && pair_role_matches_event(pair, event)
}

fn pair_context_matches_event(pair: &PositionPairEvidence, event: &ExecutionLedgerEvent) -> bool {
    let mut has_context = false;
    if let Some(run_id) = event.order.run_id.as_deref() {
        if run_id != pair.run_id {
            return false;
        }
        has_context = true;
    }
    if let Some(ticket_id) = event.order.ticket_id.as_deref() {
        if ticket_id != pair.ticket_id {
            return false;
        }
        has_context = true;
    }
    has_context
}

fn pair_role_matches_event(pair: &PositionPairEvidence, event: &ExecutionLedgerEvent) -> bool {
    event
        .order
        .leg_role
        .is_none_or(|role| hedge_role_matches_position_side(role, pair.side))
}

fn hedge_role_matches_position_side(role: HedgeLegRole, side: PositionSide) -> bool {
    matches!(
        (role, side),
        (HedgeLegRole::Long, PositionSide::Long) | (HedgeLegRole::Short, PositionSide::Short)
    )
}

pub(super) fn run_compensation_matches_record(run: &CloseRun, record: &OrderRecord) -> bool {
    run.unwind_plan.as_ref().is_some_and(|plan| {
        plan.compensation_attempts
            .iter()
            .any(|attempt| attempt_matches_record(attempt, record))
    })
}

pub(super) fn run_matches_order_id(run: &CloseRun, order_id: &str) -> bool {
    run.legs
        .iter()
        .any(|leg| leg_matches_order_id(leg, order_id))
        || run_compensation_matches_order_id(run, order_id)
}

pub(super) fn leg_matches_order_id(leg: &CloseLeg, order_id: &str) -> bool {
    leg.order
        .as_ref()
        .is_some_and(|order| order_matches_order_id(order, order_id))
}

pub(super) fn run_compensation_matches_order_id(run: &CloseRun, order_id: &str) -> bool {
    run.unwind_plan.as_ref().is_some_and(|plan| {
        plan.compensation_attempts
            .iter()
            .any(|attempt| attempt_matches_order_id(attempt, order_id))
    })
}

pub(super) fn attempt_matches_order_id(
    attempt: &CloseRunCompensationAttempt,
    order_id: &str,
) -> bool {
    attempt
        .order
        .as_ref()
        .is_some_and(|order| order_matches_order_id(order, order_id))
}

pub(super) fn order_matches_order_id(order: &OrderRecord, order_id: &str) -> bool {
    order.intent.id == order_id
        || order.intent.client_order_id == order_id
        || order.identity_snapshot().matches_order_id(order_id)
        || order.exchange_order_id.as_deref() == Some(order_id)
}

pub(super) fn candidate_matches_compensation_order(
    candidate: &CloseRunUnwindLegEvidence,
    record: &OrderRecord,
) -> bool {
    let Some(side) = candidate.compensation_order_side else {
        return false;
    };
    record.intent.source == OrderSource::CloseRunCompensation
        && venue_names_equal(&candidate.venue, &record.intent.exchange)
        && candidate.symbol.eq_ignore_ascii_case(&record.intent.symbol)
        && record.intent.side == side
        && !record.intent.reduce_only
        && compensation_quantity_matches(candidate, record.intent.quantity)
}

pub(super) fn compensation_quantity_matches(
    candidate: &CloseRunUnwindLegEvidence,
    quantity: f64,
) -> bool {
    let target = candidate
        .confirmed_quantity
        .unwrap_or(candidate.target_quantity)
        .abs();
    quantity.is_finite()
        && quantity > 0.0
        && target.is_finite()
        && quantity <= target + f64::EPSILON
}

pub(super) fn ledger_market_matches(order: &OrderRecord, event: &ExecutionLedgerEvent) -> bool {
    order.intent.exchange == event.order.exchange && order.intent.symbol == event.order.symbol
}

pub(super) fn identity_matches_ledger(
    order: &VenueOrderIdentity,
    ledger: &VenueOrderIdentity,
) -> bool {
    match strong_venue_identity_match(order, ledger) {
        Some(matches) => matches,
        None => ledger
            .exchange_order_id
            .as_deref()
            .is_some_and(|id| order.matches_order_id(id)),
    }
}

fn strong_record_identity_match(order: &OrderRecord, record: &OrderRecord) -> Option<bool> {
    strong_identity_pairs([
        (order.intent.id.as_str(), record.intent.id.as_str()),
        (
            order.intent.client_order_id.as_str(),
            record.intent.client_order_id.as_str(),
        ),
    ])
}

fn strong_venue_identity_match(
    order: &VenueOrderIdentity,
    ledger: &VenueOrderIdentity,
) -> Option<bool> {
    strong_identity_pairs([
        (
            order.internal_order_id.as_str(),
            ledger.internal_order_id.as_str(),
        ),
        (
            order.public_client_order_id.as_str(),
            ledger.public_client_order_id.as_str(),
        ),
        (
            order.venue_client_order_id.as_deref().unwrap_or(""),
            ledger.venue_client_order_id.as_deref().unwrap_or(""),
        ),
    ])
}

fn strong_identity_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> Option<bool> {
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

pub(super) fn order_identity_matches_record(order: &OrderRecord, record: &OrderRecord) -> bool {
    let identity = order.identity_snapshot();
    identity.matches_order_id(&record.intent.id)
        || identity.matches_order_id(&record.intent.client_order_id)
        || record
            .exchange_order_id
            .as_deref()
            .is_some_and(|id| identity.matches_order_id(id))
}

pub(super) fn record_identity_matches_order(order: &OrderRecord, record: &OrderRecord) -> bool {
    let identity = record.identity_snapshot();
    identity.matches_order_id(&order.intent.id)
        || identity.matches_order_id(&order.intent.client_order_id)
        || order
            .exchange_order_id
            .as_deref()
            .is_some_and(|id| identity.matches_order_id(id))
}
