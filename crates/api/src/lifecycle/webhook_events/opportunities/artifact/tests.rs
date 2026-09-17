use super::*;
use shared_types::{
    ExecutionArtifactEvidence, ExecutionArtifactLeg, ExecutionArtifactStatus, ExecutionEnvironment,
    MarketDataQuality, MarketDataSourceKind, EXECUTION_ARTIFACT_SCHEMA_VERSION,
};

const NOW_MS: i64 = 15;

#[test]
fn spot_cross_payload_keeps_bound_transfer_evidence() {
    let payload = opportunity_artifact_payload(
        &artifact(
            Some(StrategyKind::SpotCross),
            Some(transfer_evidence(
                "双向充提可用：Base 经 bitcoin，Quote 经 tron",
            )),
        ),
        None,
        NOW_MS,
    );
    assert_eq!(payload["mode"], "execution_artifact");
    assert_eq!(payload["deterministicOpportunity"], true);
    assert_eq!(payload["transfer"]["state"], "available");
    assert_eq!(payload["transfer"]["available"], true);
    assert!(payload["message"]
        .as_str()
        .is_some_and(|message| message.contains("Base 经 bitcoin")));
}

#[test]
fn spot_perp_variants_require_transfer_evidence() {
    for strategy in [StrategyKind::CrossSpotPerp, StrategyKind::SpotPerp] {
        let payload = opportunity_artifact_payload(
            &artifact(
                Some(strategy),
                Some(transfer_evidence("Base 与 Quote 充提可用")),
            ),
            None,
            NOW_MS,
        );
        assert_eq!(payload["transfer"]["required"], true);
        assert_eq!(payload["transfer"]["state"], "available");
    }
}

#[test]
fn perp_cross_marks_transfer_not_applicable() {
    let payload =
        opportunity_artifact_payload(&artifact(Some(StrategyKind::PerpCross), None), None, NOW_MS);
    assert_eq!(payload["deterministicOpportunity"], true);
    assert_eq!(payload["transfer"]["state"], "not_applicable");
    assert_eq!(payload["transfer"]["required"], false);
}

#[test]
fn spot_artifact_without_passing_transfer_evidence_is_not_deterministic() {
    let artifact = artifact(Some(StrategyKind::SpotCross), None);
    let payload = opportunity_artifact_payload(&artifact, None, NOW_MS);

    assert!(!deterministic_artifact_ready_at(&artifact, NOW_MS));
    assert_eq!(payload["deterministicOpportunity"], false);
    assert_eq!(payload["transfer"]["state"], "unknown");
}

#[test]
fn current_blocked_transfer_revokes_deterministic_webhook_label() {
    let artifact = artifact(
        Some(StrategyKind::SpotCross),
        Some(transfer_evidence("双向充提可用")),
    );
    let blocked = CandidateTransferStatus::Blocked {
        detail: "共同网络提币已暂停".to_owned(),
    };

    let payload = opportunity_artifact_payload(&artifact, Some(&blocked), NOW_MS);

    assert_eq!(payload["deterministicOpportunity"], false);
    assert_eq!(payload["transfer"]["state"], "blocked");
    assert_eq!(payload["transfer"]["available"], false);
    assert_eq!(payload["transfer"]["source"], "current_registry");
}

#[test]
fn expired_artifact_is_not_a_deterministic_webhook_opportunity() {
    let artifact = artifact(Some(StrategyKind::PerpCross), None);
    assert!(!deterministic_artifact_ready_at(&artifact, 21));
}

fn transfer_evidence(detail: &str) -> ExecutionArtifactEvidence {
    ExecutionArtifactEvidence {
        key: TRANSFER_ROUTE_EVIDENCE_KEY.to_owned(),
        label: "充提路径".to_owned(),
        passed: true,
        detail: detail.to_owned(),
        observed_at_ms: Some(10),
    }
}

fn artifact(
    strategy: Option<StrategyKind>,
    transfer: Option<ExecutionArtifactEvidence>,
) -> DeterministicExecutionArtifact {
    DeterministicExecutionArtifact {
        schema_version: EXECUTION_ARTIFACT_SCHEMA_VERSION.to_owned(),
        artifact_id: "artifact-test".to_owned(),
        opportunity_id: "opportunity-test".to_owned(),
        opportunity_snapshot_id: "snapshot-test".to_owned(),
        ticket_id: "ticket-test".to_owned(),
        idempotency_key: "idempotency-test".to_owned(),
        environment: ExecutionEnvironment::Paper,
        strategy,
        symbol: "BTC-USDT".to_owned(),
        generated_at_ms: 10,
        expires_at_ms: 20,
        status: ExecutionArtifactStatus::Ready,
        expected_gross_edge_usd: 2.0,
        expected_total_cost_usd: 0.5,
        expected_net_edge_usd: 1.5,
        capital_usd: 100.0,
        max_loss_usd: 5.0,
        legs: vec![
            leg(HedgeLegRole::Long, OrderSide::Buy, "binance"),
            leg(HedgeLegRole::Short, OrderSide::Sell, "okx"),
        ],
        evidence: transfer.into_iter().collect(),
        invalidation_conditions: Vec::new(),
        blockers: Vec::new(),
        checksum: "checksum".to_owned(),
        validation_command: "curl --get /validate".to_owned(),
    }
}

fn leg(role: HedgeLegRole, side: OrderSide, venue: &str) -> ExecutionArtifactLeg {
    ExecutionArtifactLeg {
        role,
        venue: venue.to_owned(),
        symbol: "BTC-USDT".to_owned(),
        side,
        reference_price: Some(100.0),
        target_notional_usd: 100.0,
        depth_usd_5bps: Some(1_000.0),
        market_quality: Some(MarketDataQuality::Fresh),
        market_source: Some(MarketDataSourceKind::WsPush),
        market_observed_at_ms: Some(10),
    }
}
