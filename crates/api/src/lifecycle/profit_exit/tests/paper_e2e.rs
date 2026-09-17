use super::super::*;
use super::paper_e2e_support::{
    assert_review_contains_succeeded_close, configure_e2e_exit_policy, configure_e2e_webhook,
    paper_runtime_config,
};
use super::paper_fixture::{automation_opportunity, seed_books, seed_instruments};
use shared_types::{
    AutomatedArbitrageConfigPatch, AutomationControlAction, AutomationDecisionKind,
    AutomationRuntimeStatus, CloseRunStatus, ExecutionArtifactStatus,
    ExecutionArtifactValidationRequest, ExecutionRunState, DEFAULT_AUTOMATION_CAPITAL_USD,
};

#[test]
fn paper_automation_opens_protects_closes_and_reaches_review() -> anyhow::Result<()> {
    let worker = std::thread::Builder::new()
        .name("automation-paper-e2e".to_owned())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?
                .block_on(paper_automation_flow())
        })?;
    match worker.join() {
        Ok(result) => result,
        Err(_) => anyhow::bail!("automation paper E2E worker panicked"),
    }
}

async fn paper_automation_flow() -> anyhow::Result<()> {
    let runtime = tempfile::tempdir()?;
    let config = paper_runtime_config(runtime.path());
    let run_id = open_paper_run(&config).await?;
    close_recovered_run(&config, &run_id).await?;
    assert_closed_restart(&config, &run_id).await
}

async fn open_paper_run(config: &common::config::AppConfig) -> anyhow::Result<String> {
    let (state, now_ms, mut webhook_cursor) = paused_paper_state(config).await?;
    crate::services::automated_arbitrage::control(
        &state,
        AutomationControlAction::Resume,
        now_ms.saturating_add(3),
    )
    .await?;

    crate::services::automated_arbitrage::evaluate_once(&state, now_ms.saturating_add(4)).await;
    let automation = crate::services::automated_arbitrage::status(&state);
    let run_id = validate_qualified_artifact_and_run(&state, &automation)?;
    crate::lifecycle::webhook_events::bridge_once(&state, &mut webhook_cursor).await;
    assert_eq!(
        webhook_queue_depth(&state).await,
        4,
        "one qualified artifact plus automation/execution events must be queued without a market-candidate duplicate"
    );
    crate::lifecycle::webhook_events::bridge_once(&state, &mut webhook_cursor).await;
    assert_eq!(webhook_queue_depth(&state).await, 4);

    let run_count = state.execution_runs().len();
    crate::services::automated_arbitrage::evaluate_once(&state, now_ms.saturating_add(5)).await;
    assert_eq!(state.execution_runs().len(), run_count);
    Ok(run_id)
}

async fn paused_paper_state(
    config: &common::config::AppConfig,
) -> anyhow::Result<(
    crate::state::AppState,
    i64,
    crate::lifecycle::webhook_events::Cursor,
)> {
    let state = crate::state::AppState::new(config.clone()).await?;
    let now_ms = common::time::now_ms();
    seed_instruments(&state, now_ms)?;
    seed_books(&state, 100.0, 100.05, now_ms);
    state.opportunity_index().publish(
        "automation-paper-e2e".to_owned(),
        now_ms,
        &[automation_opportunity(now_ms)],
    );
    configure_e2e_exit_policy(&state);
    configure_e2e_webhook(&state)?;
    crate::services::automated_arbitrage::update_config(
        &state,
        &AutomatedArbitrageConfigPatch {
            enabled: Some(true),
            cooldown_secs: Some(10),
            ..AutomatedArbitrageConfigPatch::default()
        },
        now_ms,
    )
    .await?;
    crate::services::automated_arbitrage::control(
        &state,
        AutomationControlAction::Pause,
        now_ms.saturating_add(1),
    )
    .await?;

    crate::services::automated_arbitrage::evaluate_once(&state, now_ms.saturating_add(2)).await;
    assert!(state.execution_runs().is_empty());
    let mut webhook_cursor = crate::lifecycle::webhook_events::Cursor::default();
    crate::lifecycle::webhook_events::bridge_once(&state, &mut webhook_cursor).await;
    assert_eq!(
        webhook_queue_depth(&state).await,
        1,
        "paused public-market candidates must remain in the UI ledger and not emit opportunity events"
    );
    Ok((state, now_ms, webhook_cursor))
}

fn validate_qualified_artifact_and_run(
    state: &crate::state::AppState,
    automation: &AutomationRuntimeStatus,
) -> anyhow::Result<String> {
    let artifact = automation
        .recent_decisions
        .iter()
        .find(|decision| decision.kind == AutomationDecisionKind::OpportunityQualified)
        .and_then(|decision| decision.execution_artifact.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "qualified execution artifact is missing: {:?}",
                automation.recent_decisions
            )
        })?;
    assert_eq!(artifact.status, ExecutionArtifactStatus::Ready);
    assert_eq!(artifact.capital_usd, DEFAULT_AUTOMATION_CAPITAL_USD);
    assert!(artifact.expected_net_edge_usd > 0.0);
    assert!(artifact.evidence.iter().all(|row| row.passed));
    assert!(artifact.evidence.iter().any(|row| {
        row.key == "profit_lock" && row.passed && row.detail.contains("class=projected")
    }));
    assert!(artifact
        .validation_command
        .contains("/api/automation/execution-artifacts/validate"));
    let validation_request = ExecutionArtifactValidationRequest {
        idempotency_key: artifact.idempotency_key,
        ticket_id: artifact.ticket_id,
        opportunity_snapshot_id: artifact.opportunity_snapshot_id,
        checksum: artifact.checksum,
    };
    let validated = crate::services::execution_artifact::validate(state, &validation_request)?;
    assert!(validated.valid, "artifact validation failed: {validated:?}");
    assert_eq!(validated.status, ExecutionArtifactStatus::Ready);
    let mut tampered = validation_request;
    tampered.checksum.push_str("-tampered");
    let rejected = crate::services::execution_artifact::validate(state, &tampered)?;
    assert!(!rejected.valid);
    assert_eq!(rejected.status, ExecutionArtifactStatus::Tampered);

    let run_id = automation
        .last_decision
        .as_ref()
        .and_then(|decision| decision.execution_run_id.clone())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "automatic paper entry did not create an execution run: {:?}",
                automation.last_decision
            )
        })?;
    let opened = state
        .execution_runs()
        .get(&run_id)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("automatic execution run is missing"))?;
    assert_eq!(opened.state, ExecutionRunState::Hedged);
    Ok(run_id)
}

async fn close_recovered_run(
    config: &common::config::AppConfig,
    run_id: &str,
) -> anyhow::Result<()> {
    let state = crate::state::AppState::new(config.clone()).await?;
    let recovered = state
        .execution_runs()
        .get(run_id)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("active execution run did not recover after restart"))?;
    assert_eq!(recovered.state, ExecutionRunState::Hedged);
    let restarted_automation = crate::services::automated_arbitrage::status(&state);
    assert!(!restarted_automation.config.enabled);
    seed_instruments(&state, common::time::now_ms())?;
    configure_e2e_exit_policy(&state);
    configure_e2e_webhook(&state)?;
    let mut webhook_cursor = crate::lifecycle::webhook_events::Cursor::baseline(&state);
    crate::lifecycle::webhook_events::bridge_once(&state, &mut webhook_cursor).await;
    assert_eq!(webhook_queue_depth(&state).await, 0);

    let close_runs = trigger_stop_loss(&state, run_id).await?;
    crate::lifecycle::webhook_events::bridge_once(&state, &mut webhook_cursor).await;
    assert_eq!(webhook_queue_depth(&state).await, 1);
    assert_review_contains_succeeded_close(&state, &close_runs).await;
    Ok(())
}

async fn trigger_stop_loss(
    state: &crate::state::AppState,
    run_id: &str,
) -> anyhow::Result<Vec<shared_types::CloseRun>> {
    seed_books(state, 90.0, 111.0, common::time::now_ms());
    let first = crate::services::portfolio::snapshot(state).await?;
    let first_positions = first.positions.clone();
    state.cache_portfolio_snapshot(first);
    let exit_config = state.trading_service().risk_config().auto_profit_close;
    let first_candidates =
        crate::services::profit_exit::candidates(state, &exit_config, common::time::now_ms());
    if first_candidates.is_empty() {
        let current_run = state
            .execution_runs()
            .get(run_id)
            .map(|entry| entry.value().clone());
        anyhow::bail!(
            "adverse paper marks produced no stop-loss candidate: run={current_run:?} positions={first_positions:?}"
        );
    }
    let mut tracker = ProfitExitTracker::default();
    evaluate_once(state, &mut tracker, common::time::now_ms())
        .await
        .map_err(anyhow::Error::msg)?;
    tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    let second = crate::services::portfolio::snapshot(state).await?;
    state.cache_portfolio_snapshot(second);
    evaluate_once(state, &mut tracker, common::time::now_ms())
        .await
        .map_err(anyhow::Error::msg)?;

    let closed = state
        .execution_runs()
        .get(run_id)
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("closed execution run is missing"))?;
    assert_eq!(closed.state, ExecutionRunState::Closed);
    let close_runs = state
        .close_runs()
        .iter()
        .map(|entry| entry.value().clone())
        .collect::<Vec<_>>();
    assert!(close_runs
        .iter()
        .any(|run| run.status == CloseRunStatus::Succeeded));
    Ok(close_runs)
}

async fn assert_closed_restart(
    config: &common::config::AppConfig,
    run_id: &str,
) -> anyhow::Result<()> {
    let recovered = crate::state::AppState::new(config.clone()).await?;
    assert_eq!(
        recovered
            .execution_runs()
            .get(run_id)
            .map(|entry| entry.state),
        Some(ExecutionRunState::Closed)
    );
    let recovered_close_runs = recovered
        .close_runs()
        .iter()
        .map(|entry| entry.value().clone())
        .collect::<Vec<_>>();
    assert!(recovered_close_runs
        .iter()
        .any(|run| run.status == CloseRunStatus::Succeeded));
    assert_review_contains_succeeded_close(&recovered, &recovered_close_runs).await;
    Ok(())
}

async fn webhook_queue_depth(state: &crate::state::AppState) -> usize {
    state
        .webhook()
        .status(common::time::now_ms())
        .await
        .queue_depth
}
