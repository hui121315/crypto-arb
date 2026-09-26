use super::*;

pub(super) fn record_compensation_order_for_candidate(
    state: &AppState,
    close_run_id: &str,
    candidate_index: usize,
    order: &OrderRecord,
    action_run_id: Option<String>,
) -> Result<CloseRun, AppError> {
    let mut entry = state
        .close_runs()
        .get_mut(close_run_id)
        .ok_or_else(|| AppError::NotFound(format!("close run: {close_run_id}")))?;
    let run = entry.value_mut();
    let Some(plan) = run.unwind_plan.as_mut() else {
        return Err(compensation_conflict(
            run,
            "close run has no compensation plan",
        ));
    };
    let run_id = run.id.clone();
    let run_status = run.status;
    let plan_status = plan.status;
    let candidate_count = plan.compensation_candidates.len();
    let Some(candidate) = plan.compensation_candidates.get(candidate_index).cloned() else {
        return Err(candidate_index_error_by_id(
            &run_id,
            candidate_index,
            candidate_count,
        ));
    };
    if !candidate_matches_compensation_order(&candidate, order) {
        return Err(compensation_conflict_values(
            &run_id,
            run_status,
            Some(plan_status),
            "submitted order does not match compensation candidate",
        ));
    }
    if let Some(attempt) = plan
        .compensation_attempts
        .iter_mut()
        .find(|attempt| attempt_matches_record(attempt, order))
    {
        attempt.action_run_id = action_run_id;
        update_compensation_attempt_from_order(attempt, order);
        restore_compensation_confirmation(state, attempt);
    } else {
        let mut attempt = compensation_attempt_from_order(&candidate, order);
        attempt.action_run_id = action_run_id;
        restore_compensation_confirmation(state, &mut attempt);
        plan.compensation_attempts.push(attempt);
    }
    run.updated_at_ms = common::time::now_ms();
    refresh_run_summary(run);
    refresh_action_run_payload(state, run);
    state.close_run_store().append(run);
    append_close_run_finality_from_order(state, run, order);
    Ok(run.clone())
}

fn restore_compensation_confirmation(state: &AppState, attempt: &mut CloseRunCompensationAttempt) {
    if attempt.status != CloseLegStatus::Filled || attempt.confirmed_filled_at_ms.is_some() {
        return;
    }
    let Some(order) = attempt.order.as_ref() else {
        return;
    };
    let events = state
        .trading_service()
        .list_execution_ledger_events_by_query(&trading::ExecutionLedgerQuery {
            internal_order_id: Some(order.intent.id.clone()),
            exchange_order_id: None,
            hedge_group_id: None,
            run_id: None,
            ticket_id: None,
            leg_role: None,
            from_ms: None,
            to_ms: None,
            limit: 64,
        });
    confirm_compensation_from_ledger(attempt, events);
}

pub(super) fn confirm_compensation_from_ledger(
    attempt: &mut CloseRunCompensationAttempt,
    mut events: Vec<ExecutionLedgerEvent>,
) {
    let Some(order) = attempt.order.as_ref() else {
        return;
    };
    if attempt.status != CloseLegStatus::Filled || !has_fill_evidence(order) {
        return;
    }
    // Reconstruct evidence separately: the returned order already includes its fills.
    let mut evidence = attempt.clone();
    let Some(evidence_order) = evidence.order.as_mut() else {
        return;
    };
    evidence_order.filled_quantity = None;
    evidence_order.filled_price = None;
    evidence_order.filled_fee = None;
    evidence_order.state = LiveOrderState::Submitted;
    evidence.confirmed_filled_at_ms = None;
    evidence.cost_events.clear();
    events.sort_by_key(|event| (event.occurred_at_ms, event.event_id.clone()));
    let mut seen = std::collections::HashSet::new();
    for event in events {
        if !seen.insert(event.event_id.clone())
            || confirmed_fill_time_from_ledger(order.intent.mode, &event) <= 0
            || !order_matches_ledger_event(order, &event)
        {
            continue;
        }
        let Some(update) = CloseLedgerUpdate::from_event(&event) else {
            continue;
        };
        if !update
            .fill
            .is_some_and(|fill| fill.quality == ExecutionLedgerQuality::Actual)
        {
            continue;
        }
        apply_ledger_event_to_compensation_attempt(&mut evidence, &event, &update);
    }
    let (Some(replayed), Some(recorded)) =
        (evidence.confirmed_filled_quantity(), order.filled_quantity)
    else {
        return;
    };
    let tolerance = f64::EPSILON * recorded.abs().max(1.0) * 16.0;
    if evidence.status != CloseLegStatus::Filled || (replayed - recorded).abs() > tolerance {
        return;
    }
    attempt.confirmed_filled_at_ms = evidence.confirmed_filled_at_ms;
    for event in evidence.cost_events {
        record_cost_event(&mut attempt.cost_events, event);
    }
}

pub(super) fn compensation_bad_request(message: &'static str) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::CLOSE_RUN_REQUEST_INVALID,
        message,
    )
}

pub(super) fn compensation_conflict(run: &CloseRun, message: &'static str) -> AppError {
    compensation_conflict_values(
        &run.id,
        run.status,
        run.unwind_plan.as_ref().map(|plan| plan.status),
        message,
    )
}

pub(super) fn compensation_conflict_values(
    close_run_id: &str,
    status: CloseRunStatus,
    plan_status: Option<CloseRunUnwindPlanStatus>,
    message: &'static str,
) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::CLOSE_RUN_UNWIND_REQUIRED,
        message,
    )
    .with_details(json!({
        "closeRunId": close_run_id,
        "status": status,
        "unwindPlanStatus": plan_status,
    }))
}

pub(super) fn compensation_order_key(
    close_run_id: &str,
    candidate_index: usize,
    action_run_id: &str,
) -> String {
    let seed = format!("{close_run_id}:{candidate_index}:{action_run_id}");
    format!("cc-{:016x}", stable_hash(&seed))
}

pub(super) fn stable_hash(value: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}

pub(super) fn execution_mode(adapter_name: &str) -> ExecutionMode {
    if adapter_name == "mock" {
        ExecutionMode::DryRun
    } else if adapter_name.ends_with("_testnet") {
        ExecutionMode::Testnet
    } else {
        ExecutionMode::Live
    }
}
