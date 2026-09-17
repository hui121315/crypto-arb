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
    } else {
        let mut attempt = compensation_attempt_from_order(&candidate, order);
        attempt.action_run_id = action_run_id;
        plan.compensation_attempts.push(attempt);
    }
    run.updated_at_ms = common::time::now_ms();
    refresh_run_summary(run);
    refresh_action_run_payload(state, run);
    state.close_run_store().append(run);
    append_close_run_finality_from_order(state, run, order);
    Ok(run.clone())
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
