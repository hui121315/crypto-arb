use super::{
    alert_worthy_automation_decision, bridge_for, emit_automation, emit_execution_results, Cursor,
    WakeReason, WakeSources,
};

#[tokio::test]
async fn close_run_activity_wakes_bridge_without_portfolio_polling() -> anyhow::Result<()> {
    let state = crate::state::AppState::new(common::config::AppConfig::default()).await?;
    let mut sources = WakeSources::new(&state);

    state
        .ws_hub()
        .notify_activity(realtime::channels::CLOSE_RUN_ACTIVITY);

    let wake =
        tokio::time::timeout(std::time::Duration::from_millis(50), sources.changed()).await?;
    assert_eq!(wake, Some(WakeReason::CloseRuns));
    assert_eq!(
        state
            .ws_hub()
            .subscriber_count(realtime::channels::PORTFOLIO),
        0,
        "internal close-run wake must not activate periodic portfolio AppWS payloads"
    );
    Ok(())
}
use shared_types::{
    AutomationControlAction, AutomationDecisionKind, ExecutionRun, ExecutionRunLeg,
    ExecutionRunState, HedgeLegRole, LiveOrderState, WebhookConfigPatch, WebhookEvent,
    WebhookEventKind, WebhookProvider, WEBHOOK_EVENT_VERSION,
};

#[test]
fn webhook_suppresses_high_frequency_automation_noise() {
    assert!(!alert_worthy_automation_decision(
        AutomationDecisionKind::CandidateSelected
    ));
    assert!(!alert_worthy_automation_decision(
        AutomationDecisionKind::OpportunityQualified
    ));
    assert!(!alert_worthy_automation_decision(
        AutomationDecisionKind::NoEligibleCandidate
    ));
    assert!(!alert_worthy_automation_decision(
        AutomationDecisionKind::PreviewBlocked
    ));
    assert!(alert_worthy_automation_decision(
        AutomationDecisionKind::Submitted
    ));
    assert!(alert_worthy_automation_decision(
        AutomationDecisionKind::Failed
    ));
}

#[tokio::test]
async fn startup_baseline_does_not_replay_existing_automation_decision() -> anyhow::Result<()> {
    let state = crate::state::AppState::new(common::config::AppConfig::default()).await?;
    state.webhook().update_config(WebhookConfigPatch {
        enabled: Some(true),
        provider: Some(WebhookProvider::Bark),
        url: Some("https://api.day.app/crossline-startup-baseline".to_owned()),
        event_kinds: Some(vec![WebhookEventKind::AutomationDecision]),
        ..WebhookConfigPatch::default()
    })?;
    crate::services::automated_arbitrage::control(&state, AutomationControlAction::Pause, 2)
        .await?;

    let mut cursor = Cursor::baseline(&state);
    emit_automation(&state, &mut cursor).await;

    assert_eq!(state.webhook().status(3).await.queue_depth, 0);
    crate::services::automated_arbitrage::control(
        &state,
        AutomationControlAction::EmergencyStop,
        4,
    )
    .await?;
    emit_automation(&state, &mut cursor).await;
    assert_eq!(state.webhook().status(5).await.queue_depth, 1);
    Ok(())
}

#[tokio::test]
async fn failed_enqueue_does_not_advance_the_bridge_cursor() -> anyhow::Result<()> {
    let state = crate::state::AppState::new(common::config::AppConfig::default()).await?;
    state.webhook().update_config(WebhookConfigPatch {
        enabled: Some(true),
        provider: Some(WebhookProvider::Bark),
        url: Some("https://api.day.app/crossline-queue-retry".to_owned()),
        event_kinds: Some(vec![WebhookEventKind::AutomationDecision]),
        queue_capacity: Some(1),
        ..WebhookConfigPatch::default()
    })?;
    state
        .webhook()
        .enqueue(
            WebhookEvent {
                id: "queue-filler".to_owned(),
                version: WEBHOOK_EVENT_VERSION.to_owned(),
                kind: WebhookEventKind::Test,
                occurred_at_ms: 1,
                payload: serde_json::json!({"message": "fill queue"}),
            },
            true,
        )
        .await?;
    crate::services::automated_arbitrage::control(&state, AutomationControlAction::Pause, 2)
        .await?;

    let mut cursor = Cursor::default();
    emit_automation(&state, &mut cursor).await;

    assert!(cursor.automation_decision_id.is_none());
    let status = state.webhook().status(3).await;
    assert_eq!(status.queue_depth, 1);
    assert_eq!(status.dropped_total, 1);
    Ok(())
}

#[tokio::test]
async fn execution_result_is_emitted_once_per_meaningful_state() -> anyhow::Result<()> {
    let state = crate::state::AppState::new(common::config::AppConfig::default()).await?;
    state.webhook().update_config(WebhookConfigPatch {
        enabled: Some(true),
        provider: Some(WebhookProvider::Bark),
        url: Some("https://api.day.app/crossline-execution-result".to_owned()),
        event_kinds: Some(vec![WebhookEventKind::ExecutionResult]),
        ..WebhookConfigPatch::default()
    })?;
    let mut run = execution_run(ExecutionRunState::Hedged, 1);
    run.short_leg.state = LiveOrderState::Submitted;
    state
        .execution_runs()
        .insert(run.run_id.clone(), run.clone());
    let mut cursor = Cursor::default();

    emit_execution_results(&state, &mut cursor.execution_updates).await;
    assert_eq!(state.webhook().status(2).await.queue_depth, 0);

    run.short_leg.state = LiveOrderState::Filled;
    run.updated_at_ms = 3;
    state
        .execution_runs()
        .insert(run.run_id.clone(), run.clone());
    emit_execution_results(&state, &mut cursor.execution_updates).await;
    assert_eq!(state.webhook().status(4).await.queue_depth, 1);

    run.updated_at_ms = 5;
    state
        .execution_runs()
        .insert(run.run_id.clone(), run.clone());
    emit_execution_results(&state, &mut cursor.execution_updates).await;
    assert_eq!(state.webhook().status(6).await.queue_depth, 1);

    run.state = ExecutionRunState::Closed;
    run.updated_at_ms = 7;
    state.execution_runs().insert(run.run_id.clone(), run);
    emit_execution_results(&state, &mut cursor.execution_updates).await;
    assert_eq!(state.webhook().status(8).await.queue_depth, 2);
    Ok(())
}

#[tokio::test]
async fn restart_replays_only_unrecorded_terminal_execution_results() -> anyhow::Result<()> {
    let outbox_path = temp_path("outbox", "sqlite");
    let ledger_path = temp_path("execution-runs", "jsonl");
    let historical = execution_run_with_id("run-webhook-historical", ExecutionRunState::Hedged, 1);
    write_execution_runs(&ledger_path, std::slice::from_ref(&historical))?;
    let mut config = common::config::AppConfig::default();
    config.storage.webhook_outbox_path = Some(outbox_path.display().to_string());
    config.storage.execution_run_ledger_path = Some(ledger_path.display().to_string());

    let first = crate::state::AppState::new(config.clone()).await?;
    let historical_id = webhook::execution_result_event_id(&historical)
        .ok_or_else(|| anyhow::anyhow!("historical execution event id missing"))?;
    assert!(first.webhook().event_known(&historical_id));
    drop(first);

    let unrecorded =
        execution_run_with_id("run-webhook-crash-window", ExecutionRunState::Hedged, 2);
    write_execution_runs(&ledger_path, &[historical, unrecorded])?;
    let restarted = crate::state::AppState::new(config).await?;
    restarted.webhook().update_config(WebhookConfigPatch {
        enabled: Some(true),
        provider: Some(WebhookProvider::Bark),
        url: Some("https://api.day.app/crossline-restart-replay".to_owned()),
        event_kinds: Some(vec![WebhookEventKind::ExecutionResult]),
        ..WebhookConfigPatch::default()
    })?;
    let mut cursor = Cursor::baseline(&restarted);

    emit_execution_results(&restarted, &mut cursor.execution_updates).await;

    assert_eq!(restarted.webhook().status(3).await.queue_depth, 1);
    cleanup_path(&ledger_path);
    cleanup_path(&outbox_path);
    Ok(())
}

#[tokio::test]
async fn bridge_reads_only_the_domain_that_woke_it() -> anyhow::Result<()> {
    let state = crate::state::AppState::new(common::config::AppConfig::default()).await?;
    state.webhook().update_config(WebhookConfigPatch {
        enabled: Some(true),
        provider: Some(WebhookProvider::Bark),
        url: Some("https://api.day.app/crossline-scoped-bridge".to_owned()),
        event_kinds: Some(vec![WebhookEventKind::ExecutionResult]),
        ..WebhookConfigPatch::default()
    })?;
    let run = execution_run(ExecutionRunState::Hedged, 1);
    state.execution_runs().insert(run.run_id.clone(), run);
    let mut cursor = Cursor::default();

    bridge_for(&state, &mut cursor, WakeReason::System).await;
    assert_eq!(state.webhook().status(2).await.queue_depth, 0);

    bridge_for(&state, &mut cursor, WakeReason::Execution).await;
    assert_eq!(state.webhook().status(3).await.queue_depth, 1);
    Ok(())
}

fn execution_run(state: ExecutionRunState, updated_at_ms: i64) -> ExecutionRun {
    execution_run_with_id("run-webhook-finality", state, updated_at_ms)
}

fn execution_run_with_id(
    run_id: &str,
    state: ExecutionRunState,
    updated_at_ms: i64,
) -> ExecutionRun {
    ExecutionRun {
        run_id: run_id.to_owned(),
        ticket_id: "ticket-webhook-finality".to_owned(),
        opportunity_id: "opportunity-webhook-finality".to_owned(),
        state,
        long_leg: execution_leg(HedgeLegRole::Long),
        short_leg: execution_leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(updated_at_ms),
        evidence: Default::default(),
        recovery_action: None,
        status_reason: "fixture".to_owned(),
        created_at_ms: 1,
        updated_at_ms,
    }
}

fn write_execution_runs(path: &std::path::Path, runs: &[ExecutionRun]) -> anyhow::Result<()> {
    let rows = runs
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()?;
    std::fs::write(path, format!("{}\n", rows.join("\n")))?;
    Ok(())
}

fn temp_path(label: &str, extension: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-webhook-{label}-{}-{}.{}",
        std::process::id(),
        common::time::now_ms(),
        extension
    ))
}

fn cleanup_path(path: &std::path::Path) {
    for candidate in [
        path.to_path_buf(),
        std::path::PathBuf::from(format!("{}-wal", path.display())),
        std::path::PathBuf::from(format!("{}-shm", path.display())),
    ] {
        let _ = std::fs::remove_file(candidate);
    }
}

fn execution_leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".to_owned(),
        symbol: "BTC-USDT".to_owned(),
        order_ids: vec![format!("order-{role:?}")],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: Some(1),
        state: LiveOrderState::Filled,
        target_quantity: 1.0,
        filled_quantity: Some(1.0),
        target_notional_usd: 10.0,
        filled_notional_usd: Some(10.0),
        filled_fee: Some(0.01),
    }
}
