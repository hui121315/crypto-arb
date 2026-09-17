use super::*;

mod evidence;

pub(super) use evidence::*;

pub(super) struct CompensationSubmitPlan {
    pub(super) close_run_id: String,
    pub(super) candidate_index: usize,
    pub(super) intent: shared_types::OrderIntent,
}

pub(super) struct CompensationOrderBuild<'a> {
    pub(super) state: &'a AppState,
    pub(super) run: &'a CloseRun,
    pub(super) candidate: &'a CloseRunUnwindLegEvidence,
    pub(super) candidate_index: usize,
    pub(super) action_run: &'a ActionRun,
    pub(super) quantity: f64,
    pub(super) price: f64,
}

pub(super) fn validate_compensation_state(run: &CloseRun) -> Result<(), AppError> {
    let Some(plan) = run.unwind_plan.as_ref() else {
        return Err(compensation_conflict(
            run,
            "close run has no compensation plan",
        ));
    };
    if initial_compensation_submit_allowed(run, plan)
        || retry_compensation_submit_allowed(run, plan)
    {
        return Ok(());
    }
    if plan
        .compensation_attempts
        .iter()
        .any(compensation_attempt_active)
    {
        return Err(AppError::domain(
            StatusCode::CONFLICT,
            codes::ACTION_RUN_IN_FLIGHT,
            "close-run compensation attempt is still in flight",
        )
        .with_details(json!({
            "closeRunId": run.id,
            "status": run.status,
            "unwindPlanStatus": plan.status,
            "attemptCount": plan.compensation_attempts.len(),
        })));
    }
    Err(compensation_conflict(
        run,
        "close run is not waiting for compensation submit or retry",
    ))
}

pub(super) fn validate_compensation_candidate_state(
    run: &CloseRun,
    candidate_index: usize,
) -> Result<(), AppError> {
    let Some(plan) = run.unwind_plan.as_ref() else {
        return Err(compensation_conflict(
            run,
            "close run has no compensation plan",
        ));
    };
    if initial_compensation_submit_allowed(run, plan) {
        return Ok(());
    }
    let Some(candidate) = plan.compensation_candidates.get(candidate_index) else {
        return Err(candidate_index_error(
            run,
            candidate_index,
            plan.compensation_candidates.len(),
        ));
    };
    if retry_compensation_submit_allowed(run, plan)
        && compensation_retry_allowed_for_candidate(candidate, &plan.compensation_attempts)
    {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::CONFLICT,
        codes::CLOSE_RUN_UNWIND_REQUIRED,
        "close-run compensation candidate has no retryable failed attempt",
    )
    .with_details(json!({
        "closeRunId": run.id,
        "status": run.status,
        "unwindPlanStatus": plan.status,
        "candidateIndex": candidate_index,
        "attemptCount": plan.compensation_attempts.len(),
        "maxAttemptsPerCandidate": MAX_COMPENSATION_ATTEMPTS_PER_CANDIDATE,
    })))
}

pub(super) fn initial_compensation_submit_allowed(
    run: &CloseRun,
    plan: &CloseRunUnwindPlan,
) -> bool {
    run.status == CloseRunStatus::UnwindRequired
        && plan.status == CloseRunUnwindPlanStatus::BlockedPendingManualRecheck
        && plan.compensation_attempts.is_empty()
}

pub(super) fn retry_compensation_submit_allowed(run: &CloseRun, plan: &CloseRunUnwindPlan) -> bool {
    run.status == CloseRunStatus::CompensationFailed
        && plan.status == CloseRunUnwindPlanStatus::CompensationFailed
        && !plan
            .compensation_attempts
            .iter()
            .any(compensation_attempt_active)
        && plan.compensation_candidates.iter().any(|candidate| {
            compensation_retry_allowed_for_candidate(candidate, &plan.compensation_attempts)
        })
}

pub(super) fn select_compensation_candidate(
    run: &CloseRun,
    request: &CloseRunCompensationRequest,
) -> Result<(usize, CloseRunUnwindLegEvidence), AppError> {
    let candidates = run
        .unwind_plan
        .as_ref()
        .map(|plan| plan.compensation_candidates.as_slice())
        .unwrap_or(&[]);
    let index = request
        .candidate_index
        .or_else(|| single_candidate_index(candidates))
        .ok_or_else(|| candidate_selection_error(run, candidates.len()))?;
    let candidate = candidates
        .get(index)
        .cloned()
        .ok_or_else(|| candidate_index_error(run, index, candidates.len()))?;
    if candidate.compensation_order_side.is_none() {
        return Err(compensation_conflict(
            run,
            "compensation candidate has no verified order side",
        ));
    }
    Ok((index, candidate))
}

pub(super) fn single_candidate_index(candidates: &[CloseRunUnwindLegEvidence]) -> Option<usize> {
    (candidates.len() == 1).then_some(0)
}

pub(super) fn candidate_selection_error(run: &CloseRun, count: usize) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::CLOSE_RUN_REQUEST_INVALID,
        "candidateIndex is required when close run has multiple compensation candidates",
    )
    .with_details(json!({
        "closeRunId": run.id,
        "candidateCount": count,
    }))
}

pub(super) fn candidate_index_error(run: &CloseRun, index: usize, count: usize) -> AppError {
    candidate_index_error_by_id(&run.id, index, count)
}

pub(super) fn candidate_index_error_by_id(
    close_run_id: &str,
    index: usize,
    count: usize,
) -> AppError {
    AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::CLOSE_RUN_REQUEST_INVALID,
        "candidateIndex does not match a compensation candidate",
    )
    .with_details(json!({
        "closeRunId": close_run_id,
        "candidateIndex": index,
        "candidateCount": count,
    }))
}

pub(super) fn validated_compensation_quantity(
    candidate: &CloseRunUnwindLegEvidence,
    request: &CloseRunCompensationRequest,
) -> Result<f64, AppError> {
    let target = compensation_target_quantity(candidate)?;
    if let Some(requested) = request.target_quantity {
        validate_requested_compensation_quantity(target, requested)?;
    }
    Ok(target)
}

pub(super) fn compensation_target_quantity(
    candidate: &CloseRunUnwindLegEvidence,
) -> Result<f64, AppError> {
    let target = candidate
        .confirmed_quantity
        .unwrap_or(candidate.target_quantity)
        .abs();
    if target.is_finite() && target > 0.0 {
        Ok(target)
    } else {
        Err(compensation_bad_request(
            "compensation candidate has no positive filled quantity",
        ))
    }
}

pub(super) fn validate_requested_compensation_quantity(
    target: f64,
    requested: f64,
) -> Result<(), AppError> {
    if !requested.is_finite() || requested <= 0.0 {
        return Err(compensation_bad_request(
            "targetQuantity must be a positive finite number",
        ));
    }
    let tolerance = (target.abs() * 1e-9).max(1e-9);
    if (requested - target).abs() <= tolerance {
        return Ok(());
    }
    Err(compensation_bad_request(
        "targetQuantity must equal the confirmed filled quantity",
    ))
}

pub(super) fn validated_compensation_price(
    candidate: &CloseRunUnwindLegEvidence,
    request: &CloseRunCompensationRequest,
) -> Result<f64, AppError> {
    let price = request.limit_price.unwrap_or(candidate.mark_price);
    if price.is_finite() && price > 0.0 {
        Ok(price)
    } else {
        Err(compensation_bad_request(
            "limitPrice or compensation candidate mark price must be positive",
        ))
    }
}

pub(super) fn compensation_order_intent(
    input: &CompensationOrderBuild<'_>,
) -> Result<shared_types::OrderIntent, AppError> {
    let Some(side) = input.candidate.compensation_order_side else {
        return Err(compensation_conflict(
            input.run,
            "compensation candidate has no verified order side",
        ));
    };
    let key = compensation_order_key(&input.run.id, input.candidate_index, &input.action_run.id);
    Ok(shared_types::OrderIntent {
        id: key.clone(),
        source: OrderSource::CloseRunCompensation,
        strategy: None,
        mode: execution_mode(input.state.trading_service().adapter_name()),
        exchange: normalized_venue_name(&input.candidate.venue),
        symbol: input.candidate.symbol.clone(),
        side,
        order_type: OrderType::Limit,
        quantity: input.quantity,
        price: Some(input.price),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: MarginMode::Cross,
        leverage: 1.0,
        client_order_id: key,
        client_order_id_policy: None,
        created_at_ms: common::time::now_ms(),
    })
}
