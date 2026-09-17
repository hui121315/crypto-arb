use super::*;

pub(super) fn sort_recent(rows: &mut [ExecutionRun]) {
    rows.sort_by_key(|run| Reverse(run.updated_at_ms));
}

pub(super) fn apply_order_update(run: &mut ExecutionRun, record: &OrderRecord) -> bool {
    if record.intent.reduce_only {
        return apply_recovery_order_update(run, record);
    }
    let long_update = apply_leg_update(&mut run.long_leg, record);
    let short_update = apply_leg_update(&mut run.short_leg, record);
    if long_update.matched || short_update.matched {
        apply_finality_success(run, record);
        refresh_cost_reconciliation(run);
        update_exposure(run);
        if let Some(problem) = long_update.problem.or(short_update.problem) {
            apply_valuation_problem(run, problem);
        } else {
            update_state(run);
            run.status_reason = status_reason(run.state).to_owned();
        }
        true
    } else {
        false
    }
}

pub(crate) fn project_finality_problem(
    state: &AppState,
    order_id: &str,
    problem: &ApiProblem,
    checked_at_ms: i64,
) -> Vec<ExecutionRun> {
    let mut updated = Vec::new();
    for mut entry in state.execution_runs().iter_mut() {
        let run = entry.value_mut();
        if apply_finality_problem(run, order_id, problem, checked_at_ms) {
            append_finality_problem_evidence(run, order_id, problem, checked_at_ms);
            refresh_workflow_view(run);
            state.execution_run_store().append(run);
            append_run_finality(
                state,
                run,
                OrderUpdateSource::OrderQuery,
                None,
                Some(order_id),
                checked_at_ms,
            );
            updated.push(run.clone());
        }
    }
    updated
}

pub(super) fn apply_finality_problem(
    run: &mut ExecutionRun,
    order_id: &str,
    problem: &ApiProblem,
    checked_at_ms: i64,
) -> bool {
    if !run_matches_order_id(run, order_id) {
        return false;
    }
    run.finality_problem = Some(problem.clone());
    run.finality_checked_at_ms = latest_positive_time(run.finality_checked_at_ms, checked_at_ms);
    run.updated_at_ms = run.updated_at_ms.max(checked_at_ms);
    true
}

pub(super) fn apply_finality_success(run: &mut ExecutionRun, record: &OrderRecord) {
    if record.last_update_source != OrderUpdateSource::OrderQuery {
        return;
    }
    run.finality_problem = None;
    run.finality_checked_at_ms =
        latest_positive_time(run.finality_checked_at_ms, record.updated_at_ms);
}

pub(super) fn apply_recovery_order_update(run: &mut ExecutionRun, record: &OrderRecord) -> bool {
    if !run_matches_order(run, record) {
        return false;
    }
    if record.state == LiveOrderState::Filled {
        run.state = ExecutionRunState::Closed;
        run.net_exposure_usd = 0.0;
        run.recovery_action = None;
        run.unwind_problem = None;
        run.status_reason = "补偿单成交，裸露已关闭".to_owned();
        true
    } else if leg_failed(record.state) {
        run.state = ExecutionRunState::UnwindRequired;
        run.recovery_action = Some(RecoveryAction::ManualReview);
        let problem = recovery_finality_problem(run, record);
        run.status_reason = problem.message.clone();
        run.unwind_problem = Some(problem);
        true
    } else {
        run.state = ExecutionRunState::Unwinding;
        run.unwind_problem = None;
        run.status_reason = "补偿单状态已回填".to_owned();
        true
    }
}

pub(super) fn recovery_finality_problem(run: &ExecutionRun, record: &OrderRecord) -> ApiProblem {
    let message = record
        .message
        .as_ref()
        .map(|message| format!("补偿单终态失败，需要人工复核: {message}"))
        .unwrap_or_else(|| "补偿单终态失败，需要人工复核".to_owned());
    let mut problem = ApiProblem::new(codes::HEDGE_UNWIND_FINALITY_FAILED, message)
        .with_status(409)
        .with_source("execution_run_projector");
    problem.details = Some(serde_json::json!({
        "runId": run.run_id,
        "ticketId": run.ticket_id,
        "opportunityId": run.opportunity_id,
        "orderId": record.intent.id,
        "clientOrderId": record.intent.client_order_id,
        "exchangeOrderId": record.exchange_order_id,
        "exchange": record.intent.exchange,
        "symbol": record.intent.symbol,
        "orderState": record.state,
        "lastUpdateSource": record.last_update_source,
        "netExposureUsd": run.net_exposure_usd,
    }));
    problem
}

pub(super) fn apply_leg_update(leg: &mut ExecutionRunLeg, record: &OrderRecord) -> LegUpdate {
    if !leg_matches_order(leg, record) {
        return LegUpdate::ignored();
    }
    leg.state = record.state;
    register_order_ids(leg, record);
    leg.identity = Some(record.identity_snapshot());
    leg.finality_source = record_finality_source(record);
    leg.filled_fee = record.filled_fee;
    if let Some(quantity) = exact_fill_quantity(record) {
        leg.filled_quantity = Some(quantity);
        return match execution_valuation::fill_notional(record, quantity) {
            Ok(notional) => {
                leg.filled_notional_usd = Some(notional);
                leg.confirmed_filled_at_ms = confirmed_fill_time_after_record(leg, record);
                LegUpdate::applied()
            }
            Err(problem) => LegUpdate::applied_with_problem(*problem),
        };
    } else if record.state == LiveOrderState::Filled {
        return LegUpdate::applied_with_problem(*execution_valuation::fill_evidence_problem(
            record,
            "missing_filled_quantity",
        ));
    }
    LegUpdate::applied()
}

pub(super) struct LegUpdate {
    pub(super) matched: bool,
    pub(super) problem: Option<ApiProblem>,
}

impl LegUpdate {
    pub(super) const fn ignored() -> Self {
        Self {
            matched: false,
            problem: None,
        }
    }

    pub(super) const fn applied() -> Self {
        Self {
            matched: true,
            problem: None,
        }
    }

    pub(super) fn applied_with_problem(problem: ApiProblem) -> Self {
        Self {
            matched: true,
            problem: Some(problem),
        }
    }
}

pub(super) fn apply_valuation_problem(run: &mut ExecutionRun, problem: ApiProblem) {
    run.state = ExecutionRunState::UnwindRequired;
    run.recovery_action = Some(RecoveryAction::ManualReview);
    run.status_reason = problem.message.clone();
    run.valuation_problem = Some(problem);
    run.unwind_problem = None;
}
