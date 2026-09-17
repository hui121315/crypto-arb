use super::*;

pub(super) fn append_event(run: &mut ExecutionRun, event: ExecutionRunTimelineEvent) {
    if let Some(existing) = run
        .evidence
        .events
        .iter_mut()
        .find(|existing| existing.event_id == event.event_id)
    {
        *existing = event;
        return;
    }
    run.evidence.events.push(event);
    run.evidence
        .events
        .sort_by_key(|event| event.occurred_at_ms);
    let overflow = run
        .evidence
        .events
        .len()
        .saturating_sub(EXECUTION_RUN_TIMELINE_LIMIT);
    if overflow > 0 {
        run.evidence.events.drain(..overflow);
        run.evidence.dropped_event_count =
            run.evidence.dropped_event_count.saturating_add(overflow);
    }
}

pub(super) fn matching_record_role(
    run: &ExecutionRun,
    record: &OrderRecord,
) -> Option<HedgeLegRole> {
    if leg_matches_order(&run.long_leg, record) {
        Some(HedgeLegRole::Long)
    } else if leg_matches_order(&run.short_leg, record) {
        Some(HedgeLegRole::Short)
    } else {
        None
    }
}

pub(super) fn matching_order_id_role(run: &ExecutionRun, order_id: &str) -> Option<HedgeLegRole> {
    if leg_matches_order_id(&run.long_leg, order_id) {
        Some(HedgeLegRole::Long)
    } else if leg_matches_order_id(&run.short_leg, order_id) {
        Some(HedgeLegRole::Short)
    } else {
        None
    }
}

pub(super) fn matching_ledger_role(
    run: &ExecutionRun,
    event: &ExecutionLedgerEvent,
) -> Option<HedgeLegRole> {
    event.order.leg_role.or_else(|| {
        let run_match = LedgerRunMatch::from_run(run);
        if ledger_event_matches_leg(&run_match, &run.long_leg, event) {
            Some(HedgeLegRole::Long)
        } else if ledger_event_matches_leg(&run_match, &run.short_leg, event) {
            Some(HedgeLegRole::Short)
        } else {
            None
        }
    })
}

pub(super) fn promote_leg_confidence(
    leg: &mut shared_types::ExecutionRunLegEvidence,
    confidence: ExecutionFillConfidence,
    event_id: Option<&str>,
) {
    if confidence.score() >= leg.finality_confidence.score() {
        leg.finality_confidence = confidence;
        leg.last_finality_event_id = event_id.map(str::to_owned);
    }
}

pub(super) fn order_record_confidence(record: &OrderRecord) -> ExecutionFillConfidence {
    ExecutionFillConfidence::from_source(order_record_event_type(record), record.last_update_source)
}

pub(super) fn order_record_event_type(record: &OrderRecord) -> ExecutionLedgerEventType {
    if record.filled_quantity.is_some() {
        ExecutionLedgerEventType::FillSnapshot
    } else {
        ExecutionLedgerEventType::OrderState
    }
}

pub(super) fn ledger_event_confidence(event: &ExecutionLedgerEvent) -> ExecutionFillConfidence {
    match &event.payload {
        ExecutionLedgerPayload::FillSnapshot(fill)
            if fill.confidence != ExecutionFillConfidence::Unknown =>
        {
            fill.confidence
        }
        _ => ExecutionFillConfidence::from_source(event.event_type, event.source),
    }
}

pub(super) fn order_event_kind(record: &OrderRecord) -> ExecutionRunEventKind {
    match record.state {
        LiveOrderState::PartiallyFilled | LiveOrderState::Filled => ExecutionRunEventKind::Fill,
        LiveOrderState::Cancelled | LiveOrderState::CancelRequested => {
            ExecutionRunEventKind::Cancel
        }
        LiveOrderState::Rejected | LiveOrderState::Failed => ExecutionRunEventKind::Failure,
        _ if matches!(
            record.last_update_source,
            OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile
        ) =>
        {
            ExecutionRunEventKind::Reconcile
        }
        _ => ExecutionRunEventKind::OrderUpdate,
    }
}

pub(super) fn ledger_event_kind(event: &ExecutionLedgerEvent) -> ExecutionRunEventKind {
    match event.event_type {
        ExecutionLedgerEventType::FillSnapshot | ExecutionLedgerEventType::FillEvent => {
            ExecutionRunEventKind::Fill
        }
        ExecutionLedgerEventType::FundingPayment => ExecutionRunEventKind::Funding,
        ExecutionLedgerEventType::Cancel => ExecutionRunEventKind::Cancel,
        _ if matches!(
            event.source,
            OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile
        ) =>
        {
            ExecutionRunEventKind::Reconcile
        }
        _ => ExecutionRunEventKind::OrderUpdate,
    }
}

pub(super) fn kind_for_state(state: ExecutionRunState) -> ExecutionRunEventKind {
    match state {
        ExecutionRunState::Previewed => ExecutionRunEventKind::Preview,
        ExecutionRunState::RiskChecked => ExecutionRunEventKind::Preflight,
        ExecutionRunState::SubmittingFirstLeg
        | ExecutionRunState::SubmittingSecondLeg
        | ExecutionRunState::SecondLegSubmitted => ExecutionRunEventKind::Submit,
        ExecutionRunState::FirstLegPartial | ExecutionRunState::Hedged => {
            ExecutionRunEventKind::Fill
        }
        ExecutionRunState::UnwindRequired | ExecutionRunState::Unwinding => {
            ExecutionRunEventKind::Unwind
        }
        ExecutionRunState::FailedSafe => ExecutionRunEventKind::Failure,
        ExecutionRunState::Closed => ExecutionRunEventKind::Closed,
    }
}

pub(super) fn current_problem(run: &ExecutionRun) -> Option<&ApiProblem> {
    run.finality_problem
        .as_ref()
        .or(run.unwind_problem.as_ref())
        .or(run.valuation_problem.as_ref())
}

pub(super) fn event_request_id(run: &ExecutionRun, problem: Option<&ApiProblem>) -> Option<String> {
    problem
        .and_then(|problem| problem.request_id.clone())
        .or_else(common::request_id::current)
        .or_else(|| run.evidence.request_id.clone())
}

pub(super) fn latest_time(current: Option<i64>, next: Option<i64>) -> Option<i64> {
    match (current, next.filter(|value| *value > 0)) {
        (Some(current), Some(next)) => Some(current.max(next)),
        (current, None) => current,
        (None, next) => next,
    }
}

pub(super) fn state_token(state: ExecutionRunState) -> &'static str {
    match state {
        ExecutionRunState::Previewed => "previewed",
        ExecutionRunState::RiskChecked => "risk_checked",
        ExecutionRunState::SubmittingFirstLeg => "submitting_first_leg",
        ExecutionRunState::FirstLegPartial => "first_leg_partial",
        ExecutionRunState::SubmittingSecondLeg => "submitting_second_leg",
        ExecutionRunState::SecondLegSubmitted => "second_leg_submitted",
        ExecutionRunState::Hedged => "hedged",
        ExecutionRunState::UnwindRequired => "unwind_required",
        ExecutionRunState::Unwinding => "unwinding",
        ExecutionRunState::FailedSafe => "failed_safe",
        ExecutionRunState::Closed => "closed",
    }
}

pub(super) fn order_state_token(state: LiveOrderState) -> &'static str {
    match state {
        LiveOrderState::Created => "created",
        LiveOrderState::RiskChecked => "risk_checked",
        LiveOrderState::Submitted => "submitted",
        LiveOrderState::Accepted => "accepted",
        LiveOrderState::PartiallyFilled => "partially_filled",
        LiveOrderState::Filled => "filled",
        LiveOrderState::CancelRequested => "cancel_requested",
        LiveOrderState::Cancelled => "cancelled",
        LiveOrderState::Rejected => "rejected",
        LiveOrderState::Failed => "failed",
        LiveOrderState::Unknown => "unknown",
    }
}
