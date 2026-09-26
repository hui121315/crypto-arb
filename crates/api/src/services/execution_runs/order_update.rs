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
        update_exposure(run);
        if let Some(problem) = long_update.problem.or(short_update.problem) {
            apply_valuation_problem(run, problem);
        } else {
            if record.filled_quantity.is_some_and(|value| value.is_finite() && value >= 0.0)
                && matches!(record.last_update_source, OrderUpdateSource::PrivateWs | OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile)
                && run.valuation_problem.as_ref().and_then(|problem| problem.details.as_ref())
                    .and_then(|details| details.get("orderId")).and_then(|id| id.as_str()) == Some(record.intent.id.as_str())
            {
                run.valuation_problem = None;
            }
            update_state(run);
            run.status_reason = status_reason(run).to_owned();
        }
        refresh_cost_reconciliation(run);
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
    let store = state.execution_run_store();
    let _projection_guard = store.lock_projection();
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

pub(super) fn recovery_finality_problem(run: &ExecutionRun, record: &OrderRecord) -> ApiProblem {
    let message = record
        .message
        .as_ref()
        .map(|message| format!("补救订单未全部成交，请核对剩余持仓: {message}"))
        .unwrap_or_else(|| "补救订单未全部成交，请核对剩余持仓".to_owned());
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
    if ledger_state_would_regress(leg.state, record.state)
        && !matches!(record.state, LiveOrderState::PartiallyFilled | LiveOrderState::Filled
            | LiveOrderState::Cancelled | LiveOrderState::Rejected | LiveOrderState::Failed)
    {
        return LegUpdate::ignored();
    }
    if !ledger_state_would_regress(leg.state, record.state) {
        leg.state = record.state;
    }
    register_order_ids(leg, record);
    leg.identity = Some(record.identity_snapshot());
    leg.finality_source = record_finality_source(record);
    leg.filled_fee = record.filled_fee.or(leg.filled_fee);
    if let Some(quantity) = record.filled_quantity {
        let invalid = if !quantity.is_finite() || quantity < 0.0 {
            Some("invalid_filled_quantity")
        } else if leg.filled_quantity.is_some_and(|known| quantity < known) {
            Some("filled_quantity_regressed")
        } else { None };
        if let Some(reason) = invalid {
            let mut problem = *execution_valuation::fill_evidence_problem(record, reason);
            problem.message = "成交数量与已有记录不一致，已保留原成交数量；请核对订单和持仓".into();
            return LegUpdate::applied_with_problem(problem);
        }
        if quantity == 0.0 && !matches!(record.state, LiveOrderState::Filled | LiveOrderState::PartiallyFilled) {
            leg.filled_quantity = Some(0.0);
            leg.filled_notional_usd = Some(0.0);
        }
    }
    if let Some(quantity) = exact_fill_quantity(record) {
        leg.filled_quantity = Some(quantity);
        return match execution_valuation::fill_notional(record, quantity) {
            Ok(notional) => {
                leg.filled_notional_usd = Some(notional);
                leg.confirmed_filled_at_ms = confirmed_fill_time_after_record(leg, record);
                LegUpdate::applied()
            }
            Err(problem) => {
                leg.filled_notional_usd = None;
                LegUpdate::applied_with_problem(*problem)
            }
        };
    } else if matches!(record.state, LiveOrderState::Filled | LiveOrderState::PartiallyFilled) {
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
