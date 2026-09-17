use super::*;

#[tokio::test]
async fn terminal_local_order_repairs_stale_execution_leg() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let order_id = "terminal-local-order";
    let terminal = state
        .trading_service()
        .submit(order_record(order_id, LiveOrderState::Created).intent)
        .await?;
    assert!(is_terminal_order_state(terminal.state));

    let run = execution_run(order_id, "already-terminal-order");
    state.execution_runs().insert(run.run_id.clone(), run);

    let first = refresh_pending_runs(&state).await;
    assert_eq!(first.scanned_order_count, 1);
    assert_eq!(first.skipped_terminal_count, 1);
    assert_eq!(first.publish_failure_count, 0);
    let projected = state
        .execution_runs()
        .get("run-1")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("projected execution run missing"))?;
    assert_eq!(projected.long_leg.state, terminal.state);

    let second = refresh_pending_runs(&state).await;
    assert_eq!(second.scanned_order_count, 0);
    Ok(())
}

#[tokio::test]
async fn missing_unsubmitted_leg_in_failed_safe_run_becomes_terminal() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let mut run = execution_run("never-submitted", "already-terminal-order");
    run.state = ExecutionRunState::FailedSafe;
    run.long_leg.state = LiveOrderState::Created;
    state.execution_runs().insert(run.run_id.clone(), run);

    let first = refresh_pending_runs(&state).await;
    assert_eq!(first.scanned_order_count, 1);
    assert_eq!(first.refreshed_order_count, 1);
    let projected = state
        .execution_runs()
        .get("run-1")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("projected execution run missing"))?;
    assert_eq!(projected.long_leg.state, LiveOrderState::Failed);
    assert_eq!(
        projected.long_leg.finality_source,
        Some(shared_types::OrderUpdateSource::Internal)
    );

    let second = refresh_pending_runs(&state).await;
    assert_eq!(second.scanned_order_count, 0);
    Ok(())
}

#[tokio::test]
async fn missing_leg_during_active_submission_stays_pending() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let mut run = execution_run("possibly-submitted", "already-terminal-order");
    run.state = ExecutionRunState::SubmittingFirstLeg;
    run.long_leg.state = LiveOrderState::Created;
    state.execution_runs().insert(run.run_id.clone(), run);

    let outcome = refresh_pending_runs(&state).await;
    assert_eq!(outcome.skipped_missing_local_count, 1);
    let projected = state
        .execution_runs()
        .get("run-1")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("execution run missing"))?;
    assert_eq!(projected.long_leg.state, LiveOrderState::Created);
    Ok(())
}
