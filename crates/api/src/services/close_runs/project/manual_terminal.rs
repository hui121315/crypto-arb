use super::events::encode_close_run_finality;
use super::*;
use std::fmt::Display;
use std::future::Future;

const MANUAL_TERMINAL_PROJECTOR: &str = "close_run_manual_terminal_v1";

pub(crate) async fn record_manual_terminal_evidence(
    state: &AppState,
    close_run_id: &str,
    request: &CloseRunManualTerminalRequest,
    action_run: &ActionRun,
) -> Result<CloseRun, AppError> {
    record_manual_terminal_evidence_with_persist(
        state,
        close_run_id,
        request,
        action_run,
        |event| async move {
            state
                .trading_service()
                .persist_run_finality_event(&event)
                .await
        },
    )
    .await
}

pub(in crate::services::close_runs) async fn record_manual_terminal_evidence_with_persist<
    Persist,
    PersistFuture,
    PersistError,
>(
    state: &AppState,
    close_run_id: &str,
    request: &CloseRunManualTerminalRequest,
    action_run: &ActionRun,
    persist: Persist,
) -> Result<CloseRun, AppError>
where
    Persist: FnOnce(trading::SqlRunFinalityLedgerEvent) -> PersistFuture,
    PersistFuture: Future<Output = Result<trading::SqlLedgerPersistAck, PersistError>>,
    PersistError: Display,
{
    let run = prepare_manual_terminal_evidence(state, close_run_id, request, action_run)?;
    let event = encode_close_run_finality(
        &run,
        OrderUpdateSource::Manual,
        None,
        None,
        run.updated_at_ms,
    )
    .map_err(|error| manual_terminal_finality_persist_error(&run.id, &error))?;
    let finality_event_id = event.event_id().to_owned();
    persist(event)
        .await
        .map_err(|error| manual_terminal_finality_persist_error(&run.id, &error))?;
    commit_manual_terminal_evidence(state, &run, &finality_event_id)?;
    Ok(run)
}

fn prepare_manual_terminal_evidence(
    state: &AppState,
    close_run_id: &str,
    request: &CloseRunManualTerminalRequest,
    action_run: &ActionRun,
) -> Result<CloseRun, AppError> {
    let mut run = state
        .close_runs()
        .get(close_run_id)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| close_run_not_found(close_run_id))?;
    validate_manual_terminal_request(&run, request)?;
    let recorded_at_ms = common::time::now_ms();
    let manual_cost_event = manual_terminal_cost_event(&run, request, action_run, recorded_at_ms);
    let mut plan = run
        .unwind_plan
        .clone()
        .ok_or_else(|| close_run_manual_terminal_not_applicable(&run))?;
    let evidence = CloseRunManualTerminalEvidence {
        action_run_id: Some(action_run.id.clone()),
        actor: action_run.actor.clone(),
        reason: request.reason.trim().to_owned(),
        snapshot_version: run.snapshot_version.clone(),
        recorded_at_ms,
        remaining_positions: plan.remaining_positions.clone(),
        required_evidence: plan.required_evidence.clone(),
        evidence: normalized_manual_terminal_evidence(&request.evidence),
        manual_handling_cost_usd: request.manual_handling_cost_usd,
        manual_handling_event_id: manual_cost_event
            .as_ref()
            .map(|event| event.event_id.clone()),
    };
    plan.status = CloseRunUnwindPlanStatus::ManualTerminalRecorded;
    plan.next_actions.clear();
    plan.manual_terminal_evidence = Some(evidence);
    run.unwind_plan = Some(plan);
    if let Some(event) = manual_cost_event {
        record_cost_event(&mut run.cost_events, event);
    }
    run.status = CloseRunStatus::ManuallyResolved;
    run.updated_at_ms = recorded_at_ms;
    run.message = close_run_message(&run);
    run.problem = close_run_problem(&run);
    refresh_cost_reconciliation(&mut run);
    Ok(run)
}

fn commit_manual_terminal_evidence(
    state: &AppState,
    run: &CloseRun,
    finality_event_id: &str,
) -> Result<(), AppError> {
    state
        .close_run_store()
        .append_projected(
            std::slice::from_ref(run),
            MANUAL_TERMINAL_PROJECTOR,
            finality_event_id,
        )
        .map_err(|error| manual_terminal_finality_persist_error(&run.id, &error))?;
    state.close_runs().insert(run.id.clone(), run.clone());
    refresh_action_run_payload(state, run);
    Ok(())
}

fn manual_terminal_finality_persist_error(run_id: &str, error: &impl Display) -> AppError {
    tracing::warn!(
        close_run_id = %run_id,
        %error,
        "failed to durably persist manual close run finality"
    );
    AppError::domain(
        StatusCode::SERVICE_UNAVAILABLE,
        codes::TRADING_SQL_LEDGER_WRITE_FAILED,
        "manual close-run finality was not durably persisted",
    )
    .with_details(json!({ "closeRunId": run_id }))
}

fn validate_manual_terminal_request(
    run: &CloseRun,
    request: &CloseRunManualTerminalRequest,
) -> Result<(), AppError> {
    if request.confirmation_phrase.trim() != CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::CLOSE_RUN_REQUEST_INVALID,
            "invalid close-run manual terminal confirmation phrase",
        )
        .with_details(json!({
            "closeRunId": run.id,
            "requiredConfirmationPhrase": CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE,
        })));
    }
    if request.reason.trim().is_empty() {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::CLOSE_RUN_REQUEST_INVALID,
            "close-run manual terminal evidence requires a reason",
        )
        .with_details(json!({ "closeRunId": run.id })));
    }
    if let Some(cost) = request.manual_handling_cost_usd {
        if !cost.is_finite() || cost < 0.0 {
            return Err(AppError::domain(
                StatusCode::BAD_REQUEST,
                codes::CLOSE_RUN_REQUEST_INVALID,
                "close-run manual terminal cost must be a non-negative finite USD amount",
            )
            .with_details(json!({
                "closeRunId": run.id,
                "manualHandlingCostUsd": cost,
            })));
        }
    }
    if request
        .snapshot_version
        .as_deref()
        .is_some_and(|version| version != run.snapshot_version)
    {
        return Err(AppError::domain(
            StatusCode::CONFLICT,
            codes::CLOSE_RUN_STALE_SNAPSHOT,
            "close-run manual terminal snapshot is stale",
        )
        .with_details(json!({
            "closeRunId": run.id,
            "expectedSnapshotVersion": run.snapshot_version,
            "receivedSnapshotVersion": request.snapshot_version,
        })));
    }
    if !close_run_manual_terminal_allowed(run) {
        return Err(close_run_manual_terminal_not_applicable(run));
    }
    Ok(())
}

fn manual_terminal_cost_event(
    run: &CloseRun,
    request: &CloseRunManualTerminalRequest,
    action_run: &ActionRun,
    recorded_at_ms: i64,
) -> Option<CloseRunCostLedgerEvent> {
    let amount = request.manual_handling_cost_usd?;
    Some(CloseRunCostLedgerEvent {
        event_id: format!("manual-terminal:{}:{}", run.id, action_run.id),
        component: CloseRunCostComponent::ManualHandling,
        amount_usd: amount,
        source: OrderUpdateSource::Manual,
        quality: ExecutionLedgerQuality::Actual,
        occurred_at_ms: recorded_at_ms,
        captured_at_ms: recorded_at_ms,
    })
}

fn close_run_manual_terminal_allowed(run: &CloseRun) -> bool {
    matches!(
        run.status,
        CloseRunStatus::UnwindRequired | CloseRunStatus::CompensationFailed
    ) && run.unwind_plan.is_some()
}

fn normalized_manual_terminal_evidence(evidence: &[String]) -> Vec<String> {
    evidence
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn close_run_manual_terminal_not_applicable(run: &CloseRun) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        codes::CLOSE_RUN_REQUEST_INVALID,
        "close-run manual terminal evidence is only allowed for unwind incidents",
    )
    .with_details(json!({
        "closeRunId": run.id,
        "status": run.status,
        "hasUnwindPlan": run.unwind_plan.is_some(),
    }))
}

fn close_run_not_found(close_run_id: &str) -> AppError {
    AppError::domain(StatusCode::NOT_FOUND, "NOT_FOUND", "close run not found")
        .with_details(json!({ "closeRunId": close_run_id }))
}
