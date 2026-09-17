use super::*;

pub(in crate::services::close_runs) fn compensation_submit_plan(
    state: &AppState,
    close_run_id: &str,
    request: &CloseRunCompensationRequest,
    action_run: &ActionRun,
) -> Result<CompensationSubmitPlan, AppError> {
    validate_compensation_confirmation(request)?;
    let run = close_run_snapshot(state, close_run_id)?;
    validate_compensation_snapshot(&run, request)?;
    validate_compensation_state(&run)?;
    let (candidate_index, candidate) = select_compensation_candidate(&run, request)?;
    validate_compensation_candidate_state(&run, candidate_index)?;
    let quantity = validated_compensation_quantity(&candidate, request)?;
    let price = validated_compensation_price(&candidate, request)?;
    let intent = compensation_order_intent(&CompensationOrderBuild {
        state,
        run: &run,
        candidate: &candidate,
        candidate_index,
        action_run,
        quantity,
        price,
    })?;
    Ok(CompensationSubmitPlan {
        close_run_id: run.id,
        candidate_index,
        intent,
    })
}

pub(super) fn validate_compensation_confirmation(
    request: &CloseRunCompensationRequest,
) -> Result<(), AppError> {
    if request.confirmation_phrase.trim() == CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE {
        return Ok(());
    }
    Err(compensation_bad_request(
        "invalid close-run compensation confirmation phrase",
    ))
}

pub(super) fn close_run_snapshot(
    state: &AppState,
    close_run_id: &str,
) -> Result<CloseRun, AppError> {
    let close_run_id = close_run_id.trim();
    state
        .close_runs()
        .get(close_run_id)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| AppError::NotFound(format!("close run: {close_run_id}")))
}

pub(super) fn validate_compensation_snapshot(
    run: &CloseRun,
    request: &CloseRunCompensationRequest,
) -> Result<(), AppError> {
    let snapshot = request
        .snapshot_version
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| compensation_bad_request("close-run snapshotVersion is required"))?;
    if snapshot == run.snapshot_version {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::CONFLICT,
        codes::CLOSE_RUN_STALE_SNAPSHOT,
        "close-run snapshot changed; refresh before submitting compensation",
    )
    .with_details(json!({
        "closeRunId": run.id,
        "expectedSnapshotVersion": snapshot,
        "currentSnapshotVersion": run.snapshot_version,
    })))
}
