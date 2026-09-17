use super::*;
use shared_types::{
    CloseRun, CloseRunCostComponent, CloseRunCostLedgerEvent, CloseRunManualTerminalEvidence,
    CloseRunScope, CloseRunStatus, CloseRunUnwindPlan, CloseRunUnwindPlanStatus,
};

#[tokio::test]
#[ignore = "requires CROSSLINE_TEST_POSTGRES_URL"]
async fn run_cost_facts_rebuild_execution_and_close_reconciliation() -> anyhow::Result<()> {
    let database_url = std::env::var("CROSSLINE_TEST_POSTGRES_URL")
        .map_err(|_| anyhow::anyhow!("CROSSLINE_TEST_POSTGRES_URL must be configured"))?;
    let seed = format!("api-rebuild-{}", common::time::now_ms());
    let execution_path = temp_projection_path(&format!("{seed}-execution"));
    let close_path = temp_projection_path(&format!("{seed}-close"));
    let mut config = projection_config(&execution_path);
    config.storage.postgres_url = Some(database_url);
    config.storage.close_run_ledger_path = Some(close_path.display().to_string());
    let state = AppState::new(config).await?;
    let execution_run_id = persist_execution_sources(&state, &seed).await?;
    let close_run_id = persist_manual_finality(&state, &seed).await?;

    let report = state
        .trading_service()
        .rebuild_run_cost_facts(2)
        .await
        .map_err(anyhow::Error::msg)?;
    assert!(report.complete);
    assert_rebuilt_facts(&state, &execution_run_id, &close_run_id).await?;

    state
        .trading_service()
        .drain_sql_ledger_and_shutdown()
        .await
        .map_err(anyhow::Error::msg)?;
    let _ = std::fs::remove_file(execution_path);
    let _ = std::fs::remove_file(close_path);
    Ok(())
}

async fn persist_execution_sources(state: &AppState, seed: &str) -> anyhow::Result<String> {
    let mut execution = run(&format!("execution-{seed}"), 1);
    execution.long_leg = leg_with_order(HedgeLegRole::Long, &format!("order-{seed}"));
    execution.cost_reconciliation = Some(cost());
    let order_id = format!("order-{seed}");
    let events = [
        ledger_fill_event_row(
            &execution,
            HedgeLegRole::Long,
            &order_id,
            1.0,
            100.0,
            Some(-0.25),
        ),
        ledger_funding_event_row(&execution, HedgeLegRole::Long, &order_id, 0.5),
        ledger_slippage_event_row(&execution, HedgeLegRole::Long, &order_id, 0.75),
    ];
    state
        .trading_service()
        .persist_ledger_event_group_durable(&events)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(execution.run_id)
}

async fn persist_manual_finality(state: &AppState, seed: &str) -> anyhow::Result<String> {
    let close_run_id = format!("close-{seed}");
    let close = manual_close_run(seed, &close_run_id);
    let finality = trading::SqlRunFinalityLedgerEvent::from_close_run(
        &close,
        OrderUpdateSource::Manual,
        None,
        None,
        close.updated_at_ms,
    )
    .map_err(anyhow::Error::msg)?;
    state
        .trading_service()
        .persist_run_finality_event(&finality)
        .await
        .map_err(anyhow::Error::msg)?;
    Ok(close_run_id)
}

fn manual_close_run(seed: &str, close_run_id: &str) -> CloseRun {
    let event_id = format!("manual-cost-{seed}");
    let snapshot = format!("snapshot-{seed}");
    CloseRun {
        id: close_run_id.to_owned(),
        scope: CloseRunScope::Single,
        status: CloseRunStatus::ManuallyResolved,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: snapshot.clone(),
        expected_leg_count: 0,
        reason: Some("run-cost rebuild acceptance".to_owned()),
        legs: Vec::new(),
        submitted_order_count: 0,
        failed_leg_count: 0,
        naked_exposure_usd: 0.0,
        message: "manually resolved".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(4),
        unwind_plan: Some(manual_unwind_plan(snapshot, &event_id)),
        cost_events: vec![manual_cost_event(event_id)],
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms: 4,
    }
}

fn manual_unwind_plan(snapshot_version: String, event_id: &str) -> CloseRunUnwindPlan {
    CloseRunUnwindPlan {
        status: CloseRunUnwindPlanStatus::ManualTerminalRecorded,
        filled_legs: Vec::new(),
        failed_legs: Vec::new(),
        compensation_candidates: Vec::new(),
        remaining_positions: Vec::new(),
        compensation_attempts: Vec::new(),
        manual_terminal_evidence: Some(CloseRunManualTerminalEvidence {
            action_run_id: None,
            actor: "api-test".to_owned(),
            reason: "run-cost rebuild acceptance".to_owned(),
            snapshot_version,
            recorded_at_ms: 4,
            remaining_positions: Vec::new(),
            required_evidence: Vec::new(),
            evidence: vec!["api-rebuild".to_owned()],
            manual_handling_cost_usd: Some(2.0),
            manual_handling_event_id: Some(event_id.to_owned()),
        }),
        next_actions: Vec::new(),
        required_evidence: Vec::new(),
    }
}

fn manual_cost_event(event_id: String) -> CloseRunCostLedgerEvent {
    CloseRunCostLedgerEvent {
        event_id,
        component: CloseRunCostComponent::ManualHandling,
        amount_usd: 2.0,
        source: OrderUpdateSource::Manual,
        quality: ExecutionLedgerQuality::Actual,
        occurred_at_ms: 4,
        captured_at_ms: 5,
    }
}

async fn assert_rebuilt_facts(
    state: &AppState,
    execution_run_id: &str,
    close_run_id: &str,
) -> anyhow::Result<()> {
    let execution = state
        .trading_service()
        .query_run_cost_facts("execution_run", execution_run_id)
        .await
        .map_err(anyhow::Error::msg)?;
    let close = state
        .trading_service()
        .query_run_cost_facts("close_run", close_run_id)
        .await
        .map_err(anyhow::Error::msg)?;
    assert_eq!(execution.len(), 3);
    assert!(execution
        .iter()
        .any(|fact| fact.key.component == "fee" && fact.value.amount == -0.25));
    assert!(execution
        .iter()
        .any(|fact| fact.key.component == "funding" && fact.key.scope == "run"));
    assert!(execution
        .iter()
        .any(|fact| fact.key.component == "slippage" && fact.value.amount_usd == Some(0.75)));
    assert_eq!(close.len(), 1);
    assert_eq!(close[0].key.component, "manual_handling");
    assert_eq!(close[0].key.scope, "run");
    Ok(())
}
