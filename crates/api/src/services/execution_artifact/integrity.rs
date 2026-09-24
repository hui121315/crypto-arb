use common::AppError;
use shared_types::{
    DeterministicExecutionArtifact, ExecutionArtifactStatus, ExecutionArtifactValidationResponse,
};

const CHECKSUM_DOMAIN: &[u8] = b"crossline-execution-artifact-v1";

pub(super) fn checksum(artifact: &DeterministicExecutionArtifact) -> Result<String, AppError> {
    let canonical = serde_json::to_vec(&serde_json::json!({
        "schemaVersion": artifact.schema_version,
        "opportunityId": artifact.opportunity_id,
        "opportunitySnapshotId": artifact.opportunity_snapshot_id,
        "ticketId": artifact.ticket_id,
        "idempotencyKey": artifact.idempotency_key,
        "environment": artifact.environment,
        "strategy": artifact.strategy,
        "symbol": artifact.symbol,
        "generatedAtMs": artifact.generated_at_ms,
        "expiresAtMs": artifact.expires_at_ms,
        "expectedGrossEdgeUsd": artifact.expected_gross_edge_usd,
        "expectedTotalCostUsd": artifact.expected_total_cost_usd,
        "expectedNetEdgeUsd": artifact.expected_net_edge_usd,
        "capitalUsd": artifact.capital_usd,
        "maxLossUsd": artifact.max_loss_usd,
        "legs": artifact.legs,
        "evidence": artifact.evidence,
        "invalidationConditions": artifact.invalidation_conditions,
        "blockers": artifact.blockers,
    }))?;
    Ok(common::signing::hmac_sha256_hex(
        CHECKSUM_DOMAIN,
        &canonical,
    ))
}

pub(super) fn validation_command(
    artifact: &DeterministicExecutionArtifact,
) -> Result<String, AppError> {
    let payload = serde_json::to_string(&artifact.validation_request())?;
    Ok(format!(
        "curl -sS -X POST \"${{CROSSLINE_API_BASE:-http://127.0.0.1:8000}}/api/automation/execution-artifacts/validate\" -H \"Authorization: Bearer ${{CROSSLINE_API_TOKEN:?Set CROSSLINE_API_TOKEN locally}}\" -H 'content-type: application/json' --data-binary '{}'",
        shell_single_quote(&payload)
    ))
}

fn shell_single_quote(value: &str) -> String {
    value.replace('\'', "'\\''")
}

pub(super) fn validation_failure(
    status: ExecutionArtifactStatus,
    checked_at_ms: i64,
    expires_at_ms: Option<i64>,
    blocker: &str,
) -> ExecutionArtifactValidationResponse {
    ExecutionArtifactValidationResponse {
        valid: false,
        status,
        checked_at_ms,
        expires_at_ms,
        blockers: vec![blocker.to_owned()],
        artifact: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_payload_remains_one_argument() {
        assert_eq!(shell_single_quote("a'b"), "a'\\''b");
    }

    #[test]
    fn validation_failure_never_looks_ready() {
        let response = validation_failure(
            ExecutionArtifactStatus::Tampered,
            10,
            Some(20),
            "checksum mismatch",
        );
        assert!(!response.valid);
        assert_eq!(response.status, ExecutionArtifactStatus::Tampered);
    }
}
