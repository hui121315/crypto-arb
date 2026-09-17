use super::evidence::{artifact_leg, evidence, market_evidence_expired};
use super::integrity::{checksum, validation_command};
use common::AppError;
use shared_types::{
    DeterministicExecutionArtifact, ExecutionArtifactBuildRequest, ExecutionArtifactStatus,
    HedgeLegRole, HedgePreviewResponse, EXECUTION_ARTIFACT_SCHEMA_VERSION,
};
use std::collections::BTreeSet;

pub(super) fn assemble(
    preview: &HedgePreviewResponse,
    request: &ExecutionArtifactBuildRequest,
    now_ms: i64,
) -> Result<DeterministicExecutionArtifact, AppError> {
    // Evidence is snapshot-bound; current time may change validity, never its checksum payload.
    let evidence = evidence(
        preview,
        request,
        crate::services::hedge_ticket::ticket_market_checked_at_ms(&preview.ticket),
    );
    let mut blockers = preview.ticket.blockers.clone();
    blockers.extend(
        evidence
            .iter()
            .filter(|item| !item.passed)
            .map(|item| item.detail.clone()),
    );
    let blockers = dedup(blockers);
    let expired = now_ms > preview.ticket.expires_at_ms
        || market_evidence_expired(&preview.ticket.long_leg, now_ms)
        || market_evidence_expired(&preview.ticket.short_leg, now_ms);
    let status = if expired {
        ExecutionArtifactStatus::Expired
    } else if blockers.is_empty() {
        ExecutionArtifactStatus::Ready
    } else {
        ExecutionArtifactStatus::Blocked
    };
    let total_cost = preview.estimated_open_cost_usd
        + preview.estimated_close_cost_usd
        + preview.estimated_slippage_usd;
    let mut artifact = DeterministicExecutionArtifact {
        schema_version: EXECUTION_ARTIFACT_SCHEMA_VERSION.to_owned(),
        artifact_id: String::new(),
        opportunity_id: preview.opportunity_id.clone(),
        opportunity_snapshot_id: request.opportunity_snapshot_id.clone(),
        ticket_id: preview.ticket.ticket_id.clone(),
        idempotency_key: preview.idempotency_key.clone(),
        environment: preview.long_leg.mode.environment(),
        strategy: preview.ticket.strategy,
        symbol: preview.ticket.symbol.clone(),
        generated_at_ms: preview.ticket.created_at_ms,
        expires_at_ms: preview.ticket.expires_at_ms,
        status,
        expected_gross_edge_usd: preview.estimated_gross_edge_usd,
        expected_total_cost_usd: total_cost,
        expected_net_edge_usd: preview.estimated_gross_edge_usd - total_cost,
        capital_usd: preview.ticket.sizing.requested_capital_usd,
        max_loss_usd: preview.max_loss_usd,
        legs: vec![
            artifact_leg(preview, HedgeLegRole::Long),
            artifact_leg(preview, HedgeLegRole::Short),
        ],
        evidence,
        invalidation_conditions: vec![
            "ticket expiry reached".to_owned(),
            "either market sample exceeds 30 seconds".to_owned(),
            "depth, fee, transfer, instrument, account-mode or risk evidence changes".to_owned(),
            "artifact checksum or snapshot binding changes".to_owned(),
        ],
        blockers,
        checksum: String::new(),
        validation_command: String::new(),
    };
    artifact.checksum = checksum(&artifact)?;
    artifact.artifact_id = format!("artifact-{}", &artifact.checksum[..24]);
    artifact.validation_command = validation_command(&artifact)?;
    Ok(artifact)
}

fn dedup(values: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    values
        .into_iter()
        .filter(|value| !value.trim().is_empty() && seen.insert(value.clone()))
        .collect()
}
