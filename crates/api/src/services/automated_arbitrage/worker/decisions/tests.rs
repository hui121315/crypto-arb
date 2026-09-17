use super::{
    failed_submission_cooldown_secs, preview_retry_cooldown_secs, record_error, record_if_changed,
    record_preview_exhausted, record_qualified, submission_decision, sync_runtime_if_changed,
    FailureRuntime,
};
use crate::state::AppState;
use shared_types::{
    ActionRunStatus, AutomationDecisionKind, AutomationRuntimeState,
    DeterministicExecutionArtifact, ExecutionArtifactStatus, ExecutionEnvironment,
    ExecutionRunState, HedgeConfirmStatus, EXECUTION_ARTIFACT_SCHEMA_VERSION,
};

#[test]
fn failed_action_is_never_reported_as_submitted() {
    assert_eq!(
        submission_decision(
            ActionRunStatus::Failed,
            HedgeConfirmStatus::Submitted,
            Some(ExecutionRunState::SecondLegSubmitted),
        ),
        (
            AutomationDecisionKind::Failed,
            AutomationRuntimeState::Error
        )
    );
    assert_eq!(
        submission_decision(
            ActionRunStatus::Accepted,
            HedgeConfirmStatus::Submitted,
            Some(ExecutionRunState::SecondLegSubmitted),
        ),
        (
            AutomationDecisionKind::Failed,
            AutomationRuntimeState::Error
        )
    );
    assert_eq!(
        submission_decision(
            ActionRunStatus::Succeeded,
            HedgeConfirmStatus::Replayed,
            Some(ExecutionRunState::Hedged),
        ),
        (
            AutomationDecisionKind::Replayed,
            AutomationRuntimeState::Hedged
        )
    );
    assert_eq!(
        submission_decision(
            ActionRunStatus::Succeeded,
            HedgeConfirmStatus::Submitted,
            Some(ExecutionRunState::SecondLegSubmitted),
        ),
        (
            AutomationDecisionKind::Submitted,
            AutomationRuntimeState::Submitting
        )
    );
    assert_eq!(failed_submission_cooldown_secs(30), 60);
    assert_eq!(failed_submission_cooldown_secs(120), 120);
    assert_eq!(preview_retry_cooldown_secs(1), 5);
    assert_eq!(preview_retry_cooldown_secs(30), 30);
}

#[tokio::test]
async fn live_ready_artifact_enters_automatic_submission_state() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let artifact = DeterministicExecutionArtifact {
        schema_version: EXECUTION_ARTIFACT_SCHEMA_VERSION.to_owned(),
        artifact_id: "artifact-live-1".to_owned(),
        opportunity_id: "opportunity-live-1".to_owned(),
        opportunity_snapshot_id: "snapshot-live-1".to_owned(),
        ticket_id: "ticket-live-1".to_owned(),
        idempotency_key: "idem-live-1".to_owned(),
        environment: ExecutionEnvironment::Live,
        strategy: None,
        symbol: "SOL".to_owned(),
        generated_at_ms: 1,
        expires_at_ms: 30_001,
        status: ExecutionArtifactStatus::Ready,
        expected_gross_edge_usd: 0.1,
        expected_total_cost_usd: 0.02,
        expected_net_edge_usd: 0.08,
        capital_usd: 10.0,
        max_loss_usd: 0.1,
        legs: Vec::new(),
        evidence: Vec::new(),
        invalidation_conditions: Vec::new(),
        blockers: Vec::new(),
        checksum: "checksum-live-1".to_owned(),
        validation_command: "curl --fail-with-body /validate".to_owned(),
    };

    record_qualified(&state, ("opportunity-live-1", "SOL"), artifact, 0, 1);

    let status = state.automation().snapshot();
    assert_eq!(status.state, AutomationRuntimeState::Submitting);
    let Some(decision) = status.last_decision.as_ref() else {
        anyhow::bail!("qualified decision is missing");
    };
    assert_eq!(decision.kind, AutomationDecisionKind::OpportunityQualified);
    assert_eq!(
        decision.reason,
        "deterministic opportunity artifact is ready for automatic live execution"
    );
    assert!(decision.execution_artifact.is_some());
    Ok(())
}

#[tokio::test]
async fn unchanged_runtime_state_does_not_repeat_decisions() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;

    record_if_changed(
        &state,
        AutomationRuntimeState::Paused,
        "automation is paused",
        0,
        1,
    );
    record_if_changed(
        &state,
        AutomationRuntimeState::Paused,
        "automation is paused",
        0,
        60_001,
    );

    let status = state.automation().snapshot();
    assert_eq!(status.recent_decisions.len(), 1);
    assert_eq!(status.updated_at_ms, 1);
    Ok(())
}

#[tokio::test]
async fn active_run_count_change_records_runtime_transition() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;

    record_if_changed(
        &state,
        AutomationRuntimeState::Paused,
        "automation is paused",
        0,
        1,
    );
    record_if_changed(
        &state,
        AutomationRuntimeState::Paused,
        "automation is paused",
        1,
        2,
    );

    let status = state.automation().snapshot();
    assert_eq!(status.recent_decisions.len(), 2);
    assert_eq!(status.active_run_count, 1);
    assert_eq!(status.updated_at_ms, 2);
    Ok(())
}

#[tokio::test]
async fn execution_sync_preserves_the_submission_decision() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    record_if_changed(
        &state,
        AutomationRuntimeState::Submitting,
        "execution accepted",
        1,
        1,
    );

    sync_runtime_if_changed(&state, AutomationRuntimeState::Hedged, 1, 2);
    sync_runtime_if_changed(&state, AutomationRuntimeState::Hedged, 1, 3);

    let status = state.automation().snapshot();
    assert_eq!(status.state, AutomationRuntimeState::Hedged);
    assert_eq!(status.updated_at_ms, 2);
    assert_eq!(status.recent_decisions.len(), 1);
    assert_eq!(
        status
            .last_decision
            .as_ref()
            .map(|decision| decision.reason.as_str()),
        Some("execution accepted")
    );
    Ok(())
}

#[tokio::test]
async fn automatic_error_waits_for_the_configured_retry_window() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    record_error(
        &state,
        ("opportunity-error", "SOL"),
        "automatic preview failed",
        &common::AppError::BadRequest("fixture failure".to_owned()),
        FailureRuntime {
            active_run_count: 0,
            occurred_at_ms: 1_000,
            cooldown_secs: 30,
        },
    );

    let status = state.automation().snapshot();
    assert_eq!(status.state, AutomationRuntimeState::Error);
    assert_eq!(status.cooldown_until_ms, Some(31_000));
    assert_eq!(status.recent_decisions.len(), 1);
    Ok(())
}

#[tokio::test]
async fn exhausted_preview_keeps_attempt_count_and_blocker_evidence() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    record_preview_exhausted(
        &state,
        3,
        &["opportunity-a: depth unavailable".to_owned()],
        0,
        2_000,
        30,
    );

    let status = state.automation().snapshot();
    assert_eq!(status.state, AutomationRuntimeState::CoolingDown);
    assert_eq!(status.cooldown_until_ms, Some(32_000));
    let Some(decision) = status.last_decision.as_ref() else {
        anyhow::bail!("preview exhaustion decision is missing");
    };
    assert!(decision.reason.contains("all 3 bounded candidate previews"));
    assert!(decision.reason.contains("depth unavailable"));
    Ok(())
}

#[tokio::test]
async fn exhausted_preview_keeps_a_retry_floor_below_entry_cooldown() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    record_preview_exhausted(
        &state,
        1,
        &["opportunity-a: websocket depth warming".to_owned()],
        0,
        2_000,
        1,
    );

    assert_eq!(state.automation().snapshot().cooldown_until_ms, Some(7_000));
    Ok(())
}
