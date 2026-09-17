mod assembly;
mod evidence;
mod integrity;

use crate::state::AppState;
use assembly::assemble;
use axum::http::StatusCode;
use common::AppError;
use integrity::{checksum, validation_failure};
use shared_types::{
    problem::codes, DeterministicExecutionArtifact, ExecutionArtifactBuildRequest,
    ExecutionArtifactStatus, ExecutionArtifactValidationRequest,
    ExecutionArtifactValidationResponse, HedgePreviewResponse, HEDGE_PREVIEW_MARKET_MAX_AGE_MS,
};

pub(crate) fn build(
    state: &AppState,
    request: &ExecutionArtifactBuildRequest,
) -> Result<DeterministicExecutionArtifact, AppError> {
    validate_request(request)?;
    let preview = lookup_preview(state, &request.idempotency_key)?;
    validate_ticket_binding(&preview, &request.ticket_id)?;
    validate_snapshot_binding(&preview, &request.opportunity_snapshot_id)?;
    assemble(&preview, request, common::time::now_ms())
}

pub(crate) fn validate(
    state: &AppState,
    request: &ExecutionArtifactValidationRequest,
) -> Result<ExecutionArtifactValidationResponse, AppError> {
    let checked_at_ms = common::time::now_ms();
    let build_request = ExecutionArtifactBuildRequest {
        idempotency_key: request.idempotency_key.clone(),
        ticket_id: request.ticket_id.clone(),
        opportunity_snapshot_id: request.opportunity_snapshot_id.clone(),
    };
    if validate_request(&build_request).is_err() {
        return Ok(validation_failure(
            ExecutionArtifactStatus::Missing,
            checked_at_ms,
            None,
            "execution artifact identifiers are incomplete",
        ));
    }
    let Some(preview) = state
        .hedge_previews()
        .get(&request.idempotency_key)
        .map(|entry| entry.clone())
    else {
        return validate_recorded_artifact(state, request, checked_at_ms);
    };
    if preview.ticket.ticket_id != request.ticket_id {
        return Ok(validation_failure(
            ExecutionArtifactStatus::Tampered,
            checked_at_ms,
            Some(preview.ticket.expires_at_ms),
            "ticket binding does not match the stored preview",
        ));
    }
    if preview.opportunity_snapshot_id != request.opportunity_snapshot_id {
        return Ok(validation_failure(
            ExecutionArtifactStatus::Tampered,
            checked_at_ms,
            Some(preview.ticket.expires_at_ms),
            "opportunity snapshot binding does not match the stored preview",
        ));
    }
    let artifact = assemble(&preview, &build_request, checked_at_ms)?;
    if artifact.checksum != request.checksum {
        return Ok(ExecutionArtifactValidationResponse {
            valid: false,
            status: ExecutionArtifactStatus::Tampered,
            checked_at_ms,
            expires_at_ms: Some(artifact.expires_at_ms),
            blockers: vec!["artifact checksum does not match the stored preview".to_owned()],
            artifact: Some(artifact),
        });
    }
    Ok(ExecutionArtifactValidationResponse {
        valid: artifact.status.is_ready(),
        status: artifact.status,
        checked_at_ms,
        expires_at_ms: Some(artifact.expires_at_ms),
        blockers: artifact.blockers.clone(),
        artifact: Some(artifact),
    })
}

fn validate_recorded_artifact(
    state: &AppState,
    request: &ExecutionArtifactValidationRequest,
    checked_at_ms: i64,
) -> Result<ExecutionArtifactValidationResponse, AppError> {
    let status = state.automation().snapshot();
    let Some(artifact) = status
        .recent_decisions
        .iter()
        .filter_map(|decision| decision.execution_artifact.as_ref())
        .find(|artifact| {
            artifact.idempotency_key == request.idempotency_key
                && artifact.ticket_id == request.ticket_id
                && artifact.opportunity_snapshot_id == request.opportunity_snapshot_id
        })
        .cloned()
    else {
        return Ok(validation_failure(
            ExecutionArtifactStatus::Missing,
            checked_at_ms,
            None,
            "preview and recorded artifact evidence are no longer available; rebuild the preview",
        ));
    };
    validate_immutable_recorded_artifact(artifact, request, checked_at_ms)
}

fn validate_immutable_recorded_artifact(
    artifact: DeterministicExecutionArtifact,
    request: &ExecutionArtifactValidationRequest,
    checked_at_ms: i64,
) -> Result<ExecutionArtifactValidationResponse, AppError> {
    if checksum(&artifact)? != artifact.checksum || request.checksum != artifact.checksum {
        return Ok(validation_failure(
            ExecutionArtifactStatus::Tampered,
            checked_at_ms,
            Some(artifact.expires_at_ms),
            "artifact checksum does not match the recorded qualified artifact",
        ));
    }
    if checked_at_ms > artifact.expires_at_ms {
        return Ok(ExecutionArtifactValidationResponse {
            valid: false,
            status: ExecutionArtifactStatus::Expired,
            checked_at_ms,
            expires_at_ms: Some(artifact.expires_at_ms),
            blockers: vec!["execution artifact expired; rebuild the preview".to_owned()],
            artifact: Some(artifact),
        });
    }
    if recorded_market_evidence_expired(&artifact, checked_at_ms) {
        return Ok(ExecutionArtifactValidationResponse {
            valid: false,
            status: ExecutionArtifactStatus::Expired,
            checked_at_ms,
            expires_at_ms: Some(artifact.expires_at_ms),
            blockers: vec![
                "recorded bilateral websocket market evidence exceeded the 30 second freshness window"
                    .to_owned(),
            ],
            artifact: Some(artifact),
        });
    }
    Ok(ExecutionArtifactValidationResponse {
        valid: artifact.status.is_ready(),
        status: artifact.status,
        checked_at_ms,
        expires_at_ms: Some(artifact.expires_at_ms),
        blockers: artifact.blockers.clone(),
        artifact: Some(artifact),
    })
}

fn recorded_market_evidence_expired(
    artifact: &DeterministicExecutionArtifact,
    checked_at_ms: i64,
) -> bool {
    artifact.legs.len() != 2
        || artifact.legs.iter().any(|leg| {
            leg.market_quality != Some(shared_types::MarketDataQuality::Fresh)
                || leg.market_source != Some(shared_types::MarketDataSourceKind::WsPush)
                || leg.market_observed_at_ms.is_none_or(|observed_at_ms| {
                    observed_at_ms <= 0
                        || observed_at_ms > checked_at_ms
                        || checked_at_ms.saturating_sub(observed_at_ms)
                            > HEDGE_PREVIEW_MARKET_MAX_AGE_MS
                })
        })
}

#[cfg(test)]
mod tests;

fn validate_request(request: &ExecutionArtifactBuildRequest) -> Result<(), AppError> {
    if request.idempotency_key.trim().is_empty()
        || request.ticket_id.trim().is_empty()
        || request.opportunity_snapshot_id.trim().is_empty()
    {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            codes::HEDGE_TICKET_REQUIRED,
            "idempotencyKey, ticketId and opportunitySnapshotId are required",
        ));
    }
    Ok(())
}

fn lookup_preview(
    state: &AppState,
    idempotency_key: &str,
) -> Result<HedgePreviewResponse, AppError> {
    state
        .hedge_previews()
        .get(idempotency_key)
        .map(|entry| entry.clone())
        .ok_or_else(|| {
            AppError::domain(
                StatusCode::NOT_FOUND,
                codes::HEDGE_PREVIEW_NOT_FOUND,
                "hedge preview not found; rebuild the preview",
            )
        })
}

fn validate_snapshot_binding(
    preview: &HedgePreviewResponse,
    snapshot_id: &str,
) -> Result<(), AppError> {
    if preview.opportunity_snapshot_id == snapshot_id {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::HEDGE_TICKET_MISMATCH,
        "opportunitySnapshotId does not belong to the stored hedge preview",
    ))
}

fn validate_ticket_binding(
    preview: &HedgePreviewResponse,
    ticket_id: &str,
) -> Result<(), AppError> {
    if preview.ticket.ticket_id == ticket_id {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::BAD_REQUEST,
        codes::HEDGE_TICKET_MISMATCH,
        "ticketId does not belong to the stored hedge preview",
    ))
}
