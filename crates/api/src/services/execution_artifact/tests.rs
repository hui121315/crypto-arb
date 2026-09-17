use super::*;
use shared_types::{
    ExecutionArtifactLeg, ExecutionEnvironment, HedgeLegRole, MarketDataQuality,
    MarketDataSourceKind, OrderSide, StrategyKind, EXECUTION_ARTIFACT_SCHEMA_VERSION,
};

#[test]
fn recorded_artifact_expiry_fails_closed_after_preview_cleanup() -> anyhow::Result<()> {
    let mut artifact = recorded_artifact("expiry", 10, 1);
    artifact.checksum = checksum(&artifact)?;
    let request = validation_request(&artifact);

    let response = validate_immutable_recorded_artifact(artifact, &request, 11)?;

    assert!(!response.valid);
    assert_eq!(response.status, ExecutionArtifactStatus::Expired);
    assert!(response.blockers.iter().any(|row| row.contains("expired")));
    Ok(())
}

#[test]
fn recorded_artifact_rejects_stale_market_evidence_before_ticket_expiry() -> anyhow::Result<()> {
    let mut artifact = recorded_artifact("market-expiry", 60_000, 1);
    artifact.checksum = checksum(&artifact)?;
    let request = validation_request(&artifact);

    let response = validate_immutable_recorded_artifact(artifact, &request, 30_002)?;

    assert!(!response.valid);
    assert_eq!(response.status, ExecutionArtifactStatus::Expired);
    assert!(response
        .blockers
        .iter()
        .any(|row| row.contains("websocket market evidence")));
    Ok(())
}

fn recorded_artifact(
    label: &str,
    expires_at_ms: i64,
    observed_at_ms: i64,
) -> DeterministicExecutionArtifact {
    DeterministicExecutionArtifact {
        schema_version: EXECUTION_ARTIFACT_SCHEMA_VERSION.to_owned(),
        artifact_id: format!("artifact-{label}"),
        opportunity_id: format!("opportunity-{label}"),
        opportunity_snapshot_id: format!("snapshot-{label}"),
        ticket_id: format!("ticket-{label}"),
        idempotency_key: format!("idempotency-{label}"),
        environment: ExecutionEnvironment::Paper,
        strategy: Some(StrategyKind::PerpCross),
        symbol: "BTC".to_owned(),
        generated_at_ms: 1,
        expires_at_ms,
        status: ExecutionArtifactStatus::Ready,
        expected_gross_edge_usd: 1.0,
        expected_total_cost_usd: 0.2,
        expected_net_edge_usd: 0.8,
        capital_usd: 100.0,
        max_loss_usd: 1.0,
        legs: legs(observed_at_ms),
        evidence: Vec::new(),
        invalidation_conditions: vec![format!("{label} invalidation")],
        blockers: Vec::new(),
        checksum: String::new(),
        validation_command: String::new(),
    }
}

fn validation_request(
    artifact: &DeterministicExecutionArtifact,
) -> ExecutionArtifactValidationRequest {
    ExecutionArtifactValidationRequest {
        idempotency_key: artifact.idempotency_key.clone(),
        ticket_id: artifact.ticket_id.clone(),
        opportunity_snapshot_id: artifact.opportunity_snapshot_id.clone(),
        checksum: artifact.checksum.clone(),
    }
}

fn legs(observed_at_ms: i64) -> Vec<ExecutionArtifactLeg> {
    [HedgeLegRole::Long, HedgeLegRole::Short]
        .into_iter()
        .map(|role| ExecutionArtifactLeg {
            role,
            venue: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: match role {
                HedgeLegRole::Long => OrderSide::Buy,
                HedgeLegRole::Short => OrderSide::Sell,
            },
            reference_price: Some(100.0),
            target_notional_usd: 100.0,
            depth_usd_5bps: Some(1_000.0),
            market_quality: Some(MarketDataQuality::Fresh),
            market_source: Some(MarketDataSourceKind::WsPush),
            market_observed_at_ms: Some(observed_at_ms),
        })
        .collect()
}
