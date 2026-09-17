use super::*;
use shared_types::hedge::HedgeTicketOrderPlans;
use shared_types::{
    ExecutionFillConfidence, ExecutionRunEventKind, ExecutionRunTimelineEvent, HedgeLegRole,
    EXECUTION_RUN_TIMELINE_LIMIT,
};

mod event;
#[cfg(test)]
mod tests;

use event::{
    append_event, current_problem, event_request_id, kind_for_state, latest_time,
    ledger_event_confidence, ledger_event_kind, matching_ledger_role, matching_order_id_role,
    matching_record_role, order_event_kind, order_record_confidence, order_record_event_type,
    order_state_token, promote_leg_confidence, state_token,
};

pub(crate) fn initialize_evidence(
    run: &mut ExecutionRun,
    plans: Option<&HedgeTicketOrderPlans>,
    request_id: Option<String>,
) {
    run.evidence.request_id = request_id;
    match ticket_order_plans(run, plans) {
        Ok([long, short]) => {
            run.evidence.long_leg.compile_plan = Some(long.compile_plan.clone());
            run.evidence.short_leg.compile_plan = Some(short.compile_plan.clone());
        }
        Err(problem) => {
            let problem = *problem;
            append_ticket_plan_problem(run, problem);
        }
    }
    append_internal_transition(run);
}

fn ticket_order_plans<'a>(
    run: &ExecutionRun,
    plans: Option<&'a HedgeTicketOrderPlans>,
) -> Result<[&'a shared_types::hedge::HedgeTicketOrderPlanEvidence; 2], Box<ApiProblem>> {
    let plans = plans.ok_or_else(|| {
        Box::new(invalid_ticket_order_plan_problem(
            &run.ticket_id,
            "ticket order plans are missing",
        ))
    })?;
    plans.plans_for_ticket(&run.ticket_id).map_err(|error| {
        Box::new(invalid_ticket_order_plan_problem(
            &run.ticket_id,
            &error.to_string(),
        ))
    })
}

pub(crate) fn invalid_ticket_order_plan_problem(ticket_id: &str, reason: &str) -> ApiProblem {
    let mut problem = ApiProblem::new(
        codes::HEDGE_TICKET_ORDER_PLAN_EVIDENCE_INVALID,
        format!("ticket-bound order plan evidence is invalid: {reason}"),
    )
    .with_status(axum::http::StatusCode::BAD_REQUEST.as_u16())
    .with_source("execution_run_evidence")
    .with_request_id(common::request_id::current());
    problem.details = Some(serde_json::json!({
        "ticketId": ticket_id,
        "reason": reason,
    }));
    problem
}

fn append_ticket_plan_problem(run: &mut ExecutionRun, problem: ApiProblem) {
    append_event(
        run,
        ExecutionRunTimelineEvent {
            event_id: format!("run:{}:ticket-plan-invalid", run.run_id),
            kind: ExecutionRunEventKind::Failure,
            state: run.state,
            source: OrderUpdateSource::Internal,
            message: problem.message.clone(),
            occurred_at_ms: run.updated_at_ms,
            request_id: problem
                .request_id
                .clone()
                .or_else(|| run.evidence.request_id.clone()),
            leg_role: None,
            order_identity: None,
            ledger_event_type: None,
            finality_confidence: ExecutionFillConfidence::Unknown,
            problem: Some(problem),
        },
    );
}

pub(crate) fn merge_existing_evidence(state: &AppState, run: &mut ExecutionRun) {
    let Some(existing) = state.execution_runs().get(&run.run_id) else {
        return;
    };
    let existing = existing.evidence.clone();
    if run.evidence.request_id.is_none() {
        run.evidence.request_id = existing.request_id.clone();
    }
    if run.evidence.long_leg.compile_plan.is_none() {
        run.evidence.long_leg.compile_plan = existing.long_leg.compile_plan.clone();
    }
    if run.evidence.short_leg.compile_plan.is_none() {
        run.evidence.short_leg.compile_plan = existing.short_leg.compile_plan.clone();
    }
    if run.evidence.hedge_ticket_view.is_none() {
        run.evidence.hedge_ticket_view = existing.hedge_ticket_view.clone();
    }
    promote_leg_confidence(
        &mut run.evidence.long_leg,
        existing.long_leg.finality_confidence,
        existing.long_leg.last_finality_event_id.as_deref(),
    );
    promote_leg_confidence(
        &mut run.evidence.short_leg,
        existing.short_leg.finality_confidence,
        existing.short_leg.last_finality_event_id.as_deref(),
    );
    run.evidence.last_reconciled_at_ms = latest_time(
        run.evidence.last_reconciled_at_ms,
        existing.last_reconciled_at_ms,
    );
    run.evidence.dropped_event_count = run
        .evidence
        .dropped_event_count
        .max(existing.dropped_event_count);
    for event in existing.events {
        append_event(run, event);
    }
}

pub(crate) fn append_internal_transition(run: &mut ExecutionRun) {
    let problem = current_problem(run).cloned();
    let request_id = event_request_id(run, problem.as_ref());
    let event = ExecutionRunTimelineEvent {
        event_id: format!(
            "run:{}:state:{}:{}",
            run.run_id,
            state_token(run.state),
            run.updated_at_ms
        ),
        kind: kind_for_state(run.state),
        state: run.state,
        source: OrderUpdateSource::Internal,
        message: run.status_reason.clone(),
        occurred_at_ms: run.updated_at_ms,
        request_id,
        leg_role: None,
        order_identity: None,
        ledger_event_type: None,
        finality_confidence: ExecutionFillConfidence::Unknown,
        problem,
    };
    append_event(run, event);
}

pub(crate) fn append_close_run_transition(
    run: &mut ExecutionRun,
    close_run: &shared_types::CloseRun,
) {
    append_event(
        run,
        ExecutionRunTimelineEvent {
            event_id: format!("close-run:{}:execution-closed", close_run.id),
            kind: ExecutionRunEventKind::Closed,
            state: ExecutionRunState::Closed,
            source: OrderUpdateSource::Internal,
            message: run.status_reason.clone(),
            occurred_at_ms: close_run.updated_at_ms,
            request_id: close_run.request_id.clone(),
            leg_role: None,
            order_identity: None,
            ledger_event_type: None,
            finality_confidence: ExecutionFillConfidence::Unknown,
            problem: None,
        },
    );
}

pub(crate) fn append_order_update_evidence(run: &mut ExecutionRun, record: &OrderRecord) {
    let role = matching_record_role(run, record);
    let confidence = order_record_confidence(record);
    if let Some(role) = role {
        promote_leg_confidence(
            run.evidence.leg_mut(role),
            confidence,
            Some(record.intent.id.as_str()),
        );
    }
    if matches!(
        record.last_update_source,
        OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile
    ) {
        run.evidence.last_reconciled_at_ms = latest_time(
            run.evidence.last_reconciled_at_ms,
            Some(record.updated_at_ms),
        );
    }
    let problem = current_problem(run).cloned();
    let request_id = event_request_id(run, problem.as_ref());
    append_event(
        run,
        ExecutionRunTimelineEvent {
            event_id: format!(
                "order:{}:{}:{}",
                record.intent.id,
                order_state_token(record.state),
                record.updated_at_ms
            ),
            kind: order_event_kind(record),
            state: run.state,
            source: record.last_update_source,
            message: record
                .message
                .clone()
                .unwrap_or_else(|| run.status_reason.clone()),
            occurred_at_ms: record.updated_at_ms,
            request_id,
            leg_role: role,
            order_identity: Some(record.identity_snapshot()),
            ledger_event_type: Some(order_record_event_type(record)),
            finality_confidence: confidence,
            problem,
        },
    );
}

pub(crate) fn append_ledger_event_evidence(run: &mut ExecutionRun, event: &ExecutionLedgerEvent) {
    let role = matching_ledger_role(run, event);
    let confidence = ledger_event_confidence(event);
    if let Some(role) = role {
        promote_leg_confidence(
            run.evidence.leg_mut(role),
            confidence,
            Some(event.event_id.as_str()),
        );
    }
    if matches!(
        event.source,
        OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile
    ) {
        run.evidence.last_reconciled_at_ms = latest_time(
            run.evidence.last_reconciled_at_ms,
            Some(event.occurred_at_ms),
        );
    }
    let problem = current_problem(run).cloned();
    let request_id = event_request_id(run, problem.as_ref());
    append_event(
        run,
        ExecutionRunTimelineEvent {
            event_id: event.event_id.clone(),
            kind: ledger_event_kind(event),
            state: run.state,
            source: event.source,
            message: run.status_reason.clone(),
            occurred_at_ms: event.occurred_at_ms,
            request_id,
            leg_role: role,
            order_identity: Some(event.order.identity.clone()),
            ledger_event_type: Some(event.event_type),
            finality_confidence: confidence,
            problem,
        },
    );
}

pub(crate) fn append_finality_problem_evidence(
    run: &mut ExecutionRun,
    order_id: &str,
    problem: &ApiProblem,
    checked_at_ms: i64,
) {
    run.evidence.last_reconciled_at_ms =
        latest_time(run.evidence.last_reconciled_at_ms, Some(checked_at_ms));
    append_event(
        run,
        ExecutionRunTimelineEvent {
            event_id: format!("reconcile:{order_id}:problem:{checked_at_ms}"),
            kind: ExecutionRunEventKind::Reconcile,
            state: run.state,
            source: OrderUpdateSource::OrderQuery,
            message: problem.message.clone(),
            occurred_at_ms: checked_at_ms,
            request_id: problem
                .request_id
                .clone()
                .or_else(|| run.evidence.request_id.clone()),
            leg_role: matching_order_id_role(run, order_id),
            order_identity: None,
            ledger_event_type: None,
            finality_confidence: ExecutionFillConfidence::Unknown,
            problem: Some(problem.clone()),
        },
    );
}
