use super::*;

pub(super) fn compensation_attempt_from_order(
    candidate: &CloseRunUnwindLegEvidence,
    record: &OrderRecord,
) -> CloseRunCompensationAttempt {
    let mut attempt = CloseRunCompensationAttempt {
        action_run_id: None,
        venue: candidate.venue.clone(),
        symbol: candidate.symbol.clone(),
        side: candidate.side,
        compensation_order_side: candidate
            .compensation_order_side
            .unwrap_or_else(|| reopen_order_side(candidate.side)),
        target_quantity: candidate
            .confirmed_quantity
            .unwrap_or(candidate.target_quantity)
            .abs(),
        status: CloseLegStatus::Submitted,
        order: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: None,
        cost_events: Vec::new(),
        submitted_at_ms: record.intent.created_at_ms,
        updated_at_ms: record.updated_at_ms,
    };
    update_compensation_attempt_from_order(&mut attempt, record);
    attempt
}

pub(super) fn update_compensation_attempt_from_order(
    attempt: &mut CloseRunCompensationAttempt,
    record: &OrderRecord,
) {
    attempt.order = Some(record.clone());
    attempt.status = close_leg_status(record);
    attempt.finality_source = close_finality_source(record);
    attempt.problem = compensation_attempt_problem(record, attempt.status);
    attempt.updated_at_ms = record.updated_at_ms;
}

pub(super) fn compensation_attempt_problem(
    record: &OrderRecord,
    status: CloseLegStatus,
) -> Option<ApiProblem> {
    match status {
        CloseLegStatus::Accepted if record.state == LiveOrderState::Filled => Some(
            compensation_problem(record, "compensation order is missing fill evidence"),
        ),
        CloseLegStatus::Cancelled | CloseLegStatus::Rejected | CloseLegStatus::Failed => {
            Some(compensation_problem(
                record,
                "compensation order reached a non-filled terminal state",
            ))
        }
        _ => None,
    }
}

pub(super) fn compensation_problem(record: &OrderRecord, message: &'static str) -> ApiProblem {
    let mut problem = ApiProblem::new(codes::CLOSE_RUN_COMPENSATION_FAILED, message)
        .with_status(409)
        .with_source("portfolio.close_run.compensation");
    problem.details = Some(serde_json::json!({
        "orderId": record.intent.id,
        "clientOrderId": record.intent.client_order_id,
        "exchangeOrderId": record.exchange_order_id,
        "exchange": record.intent.exchange,
        "symbol": record.intent.symbol,
        "orderState": record.state,
        "lastUpdateSource": record.last_update_source,
    }));
    problem
}

pub(super) fn finality_problem(record: &OrderRecord, message: &'static str) -> ApiProblem {
    let mut problem = ApiProblem::new(codes::CLOSE_RUN_FAILED, message)
        .with_status(409)
        .with_source("portfolio.close_run.finality");
    problem.details = Some(serde_json::json!({
        "orderId": record.intent.id,
        "clientOrderId": record.intent.client_order_id,
        "exchangeOrderId": record.exchange_order_id,
        "exchange": record.intent.exchange,
        "symbol": record.intent.symbol,
        "orderState": record.state,
        "lastUpdateSource": record.last_update_source,
    }));
    problem
}

pub(super) fn refresh_run_summary(run: &mut CloseRun) {
    run.submitted_order_count = submitted_order_count(&run.legs);
    run.failed_leg_count = failed_leg_count(&run.legs);
    let previous_plan = run.unwind_plan.clone();
    run.unwind_plan = close_run_unwind_plan(&run.legs, previous_plan.as_ref());
    run.status = close_run_status(&run.legs, run.unwind_plan.as_ref());
    run.naked_exposure_usd = close_run_naked_exposure_usd(&run.legs, run.status);
    run.message = close_run_message(run);
    run.problem = close_run_problem(run);
    record_paper_zero_funding_cost(run);
    refresh_cost_reconciliation(run);
}

fn record_paper_zero_funding_cost(run: &mut CloseRun) {
    if run.status != CloseRunStatus::Succeeded
        || run.legs.is_empty()
        || !run.legs.iter().all(|leg| {
            leg.order
                .as_ref()
                .is_some_and(|order| order.intent.mode == ExecutionMode::DryRun)
        })
        || run
            .cost_events
            .iter()
            .any(|event| event.component == CloseRunCostComponent::Funding)
    {
        return;
    }
    let observed_at_ms = run.updated_at_ms.max(run.started_at_ms);
    record_cost_event(
        &mut run.cost_events,
        CloseRunCostLedgerEvent {
            event_id: format!("paper-funding-model:{}", run.id),
            component: CloseRunCostComponent::Funding,
            amount_usd: 0.0,
            source: OrderUpdateSource::Internal,
            quality: ExecutionLedgerQuality::Actual,
            occurred_at_ms: observed_at_ms,
            captured_at_ms: observed_at_ms,
        },
    );
}

pub(super) fn submitted_order_count(legs: &[CloseLeg]) -> usize {
    legs.iter().filter(|leg| leg.order.is_some()).count()
}

pub(super) fn failed_leg_count(legs: &[CloseLeg]) -> usize {
    legs.iter()
        .filter(|leg| close_leg_failed(leg.status))
        .count()
}

pub(super) fn naked_exposure_usd(legs: &[CloseLeg]) -> f64 {
    legs.iter()
        .filter(|leg| close_leg_failed(leg.status))
        .map(|leg| leg.notional_usd)
        .sum()
}

pub(super) fn close_run_naked_exposure_usd(legs: &[CloseLeg], status: CloseRunStatus) -> f64 {
    if status == CloseRunStatus::Compensated {
        0.0
    } else {
        naked_exposure_usd(legs)
    }
}

pub(super) fn close_run_status(
    legs: &[CloseLeg],
    plan: Option<&CloseRunUnwindPlan>,
) -> CloseRunStatus {
    if let Some(status) = compensation_run_status(plan) {
        return status;
    }
    let filled = legs
        .iter()
        .filter(|leg| leg.status == CloseLegStatus::Filled)
        .count();
    let failed = failed_leg_count(legs);
    let has_filled_exposure = legs.iter().any(close_leg_has_fill);
    if filled == legs.len() && failed == 0 {
        CloseRunStatus::Succeeded
    } else if submitted_order_count(legs) == 0 || failed == legs.len() {
        CloseRunStatus::Failed
    } else if failed > 0 && has_filled_exposure {
        CloseRunStatus::UnwindRequired
    } else if failed > 0 {
        CloseRunStatus::PartiallySubmitted
    } else {
        CloseRunStatus::Submitted
    }
}

pub(super) fn compensation_run_status(plan: Option<&CloseRunUnwindPlan>) -> Option<CloseRunStatus> {
    match plan?.status {
        CloseRunUnwindPlanStatus::BlockedPendingManualRecheck => None,
        CloseRunUnwindPlanStatus::CompensationSubmitted => {
            Some(CloseRunStatus::CompensationSubmitted)
        }
        CloseRunUnwindPlanStatus::Compensated => Some(CloseRunStatus::Compensated),
        CloseRunUnwindPlanStatus::CompensationFailed => Some(CloseRunStatus::CompensationFailed),
        CloseRunUnwindPlanStatus::ManualTerminalRecorded => Some(CloseRunStatus::ManuallyResolved),
    }
}

pub(super) fn close_leg_failed(status: CloseLegStatus) -> bool {
    matches!(
        status,
        CloseLegStatus::Cancelled
            | CloseLegStatus::Rejected
            | CloseLegStatus::Failed
            | CloseLegStatus::Skipped
    )
}

pub(super) fn close_leg_has_fill(leg: &CloseLeg) -> bool {
    matches!(
        leg.status,
        CloseLegStatus::Filled | CloseLegStatus::PartiallyFilled
    ) && leg_confirmed_quantity(leg).is_some()
}
