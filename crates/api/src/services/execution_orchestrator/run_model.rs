use super::events::publish_run_event;
use super::*;

pub(super) fn new_run(preview: &HedgePreviewResponse, idempotency_key: &str) -> ExecutionRun {
    let now_ms = common::time::now_ms();
    let ticket_id = preview.ticket.ticket_id.clone();
    let long_leg = run_leg(HedgeLegRole::Long, &preview.long_leg);
    let short_leg = run_leg(HedgeLegRole::Short, &preview.short_leg);
    let valuation_problem = long_leg.problem.or(short_leg.problem);
    let mut run = ExecutionRun {
        run_id: format!("run-{idempotency_key}"),
        ticket_id,
        opportunity_id: preview.opportunity_id.clone(),
        state: ExecutionRunState::Previewed,
        long_leg: long_leg.leg,
        short_leg: short_leg.leg,
        net_exposure_usd: 0.0,
        cost_reconciliation: Some(estimated_cost_reconciliation(preview)),
        valuation_problem,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "预览已创建".to_owned(),
        created_at_ms: now_ms,
        updated_at_ms: now_ms,
    };
    crate::services::execution_runs::initialize_evidence(
        &mut run,
        preview.ticket_order_plans.as_ref(),
        common::request_id::current(),
    );
    run.evidence.hedge_ticket_view = Some(preview.workflow_view.clone());
    crate::services::execution_runs::refresh_workflow_view(&mut run);
    run
}

pub(super) fn ledger_context(
    run: &ExecutionRun,
    leg_role: HedgeLegRole,
) -> ExecutionLedgerOrderContext {
    ExecutionLedgerOrderContext::new(run.run_id.clone(), run.ticket_id.clone(), leg_role)
}

struct ValuedRunLeg {
    leg: ExecutionRunLeg,
    problem: Option<ApiProblem>,
}

fn run_leg(role: HedgeLegRole, intent: &OrderIntent) -> ValuedRunLeg {
    let (target_notional_usd, problem) = match execution_valuation::order_notional(intent) {
        Ok(notional) => (notional, None),
        Err(problem) => (0.0, Some(*problem)),
    };
    ValuedRunLeg {
        leg: ExecutionRunLeg {
            role,
            exchange: intent.exchange.clone(),
            symbol: intent.symbol.clone(),
            order_ids: vec![intent.id.clone()],
            identity: Some(VenueOrderIdentity::from_intent(intent)),
            finality_source: None,
            confirmed_filled_at_ms: None,
            state: LiveOrderState::Created,
            target_quantity: intent.quantity,
            filled_quantity: None,
            target_notional_usd,
            filled_notional_usd: None,
            filled_fee: None,
        },
        problem,
    }
}

pub(super) fn apply_leg_record(
    leg: &mut ExecutionRunLeg,
    record: &OrderRecord,
) -> Option<ApiProblem> {
    leg.state = record.state;
    leg.identity = Some(record.identity_snapshot());
    leg.finality_source = record_finality_source(record);
    register_order_ids(leg, record);
    leg.filled_fee = record.filled_fee;
    if let Some(quantity) = exact_fill_quantity(record) {
        leg.filled_quantity = Some(quantity);
        return match execution_valuation::fill_notional(record, quantity) {
            Ok(notional) => {
                leg.filled_notional_usd = Some(notional);
                leg.confirmed_filled_at_ms = confirmed_fill_time_after_record(leg, record);
                None
            }
            Err(problem) => Some(*problem),
        };
    } else if record.state == LiveOrderState::Filled {
        return Some(*execution_valuation::fill_evidence_problem(
            record,
            "missing_filled_quantity",
        ));
    }
    None
}

pub(super) fn refresh_second_leg_plan(
    run: &mut ExecutionRun,
    preview: &HedgePreviewResponse,
    second_role: HedgeLegRole,
) -> Option<ApiProblem> {
    let intent = super::legs::intent_for_role(preview, second_role);
    let notional = match execution_valuation::order_notional(intent) {
        Ok(notional) => notional,
        Err(problem) => return Some(*problem),
    };
    let leg = super::legs::run_leg_mut(run, second_role);
    leg.target_quantity = intent.quantity;
    leg.target_notional_usd = notional;
    leg.identity = Some(VenueOrderIdentity::from_intent(intent));
    run.cost_reconciliation = Some(estimated_cost_reconciliation(preview));
    run.evidence.hedge_ticket_view = Some(preview.workflow_view.clone());
    crate::services::execution_runs::refresh_workflow_view(run);
    None
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

fn push_order_id(leg: &mut ExecutionRunLeg, order_id: &str) {
    if !order_id.is_empty() && !leg.order_ids.iter().any(|id| id == order_id) {
        leg.order_ids.push(order_id.to_owned());
    }
}

fn record_finality_source(record: &OrderRecord) -> Option<OrderUpdateSource> {
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

fn estimated_cost_reconciliation(preview: &HedgePreviewResponse) -> ExecutionCostReconciliation {
    let estimated_total_cost_usd = preview.estimated_open_cost_usd
        + preview.estimated_close_cost_usd
        + preview.estimated_slippage_usd;
    ExecutionCostReconciliation {
        estimated_open_cost_usd: preview.estimated_open_cost_usd,
        estimated_close_cost_usd: preview.estimated_close_cost_usd,
        estimated_slippage_usd: preview.estimated_slippage_usd,
        estimated_total_cost_usd,
        filled_fee_usd: None,
        actual_slippage_usd: None,
        actual_open_cost_usd: None,
        actual_funding_usd: None,
        funding_event_ids: Vec::new(),
        actual_unwind_fee_usd: None,
        actual_unwind_slippage_usd: None,
        actual_unwind_cost_usd: None,
        unwind_event_ids: Vec::new(),
        missing_fields: Vec::new(),
        actual_cost_usd: None,
        cost_delta_usd: None,
    }
}

fn confirmed_fill_time_after_record(leg: &ExecutionRunLeg, record: &OrderRecord) -> Option<i64> {
    execution_valuation::confirmed_fill_time_from_record(leg.confirmed_filled_at_ms, record)
}

pub(super) fn exact_fill_quantity(record: &OrderRecord) -> Option<f64> {
    record
        .filled_quantity
        .filter(|quantity| quantity.is_finite() && *quantity > 0.0)
}

pub(super) fn run_state_after_second(long: &OrderRecord, short: &OrderRecord) -> ExecutionRunState {
    if long.state == LiveOrderState::Filled && short.state == LiveOrderState::Filled {
        ExecutionRunState::Hedged
    } else if leg_needs_unwind(long.state) || leg_needs_unwind(short.state) {
        ExecutionRunState::UnwindRequired
    } else {
        ExecutionRunState::SecondLegSubmitted
    }
}

fn leg_needs_unwind(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Rejected | LiveOrderState::Failed | LiveOrderState::Cancelled
    )
}

pub(super) fn update_exposure(run: &mut ExecutionRun) {
    run.net_exposure_usd = execution_valuation::net_base_exposure_usd(run);
}

pub(super) fn update_run(state: &AppState, run: &mut ExecutionRun, reason: &str) {
    run.status_reason = reason.to_owned();
    run.updated_at_ms = common::time::now_ms();
    *run = crate::services::execution_runs::record(state, run.clone());
    publish_run_event(state, run);
}
