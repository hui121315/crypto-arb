use shared_types::{
    AutoProfitCloseConfig, CloseRunStatus, WebhookConfigPatch, WebhookEventKind, WebhookProvider,
};

pub(super) fn paper_runtime_config(path: &std::path::Path) -> common::config::AppConfig {
    let mut config = common::config::AppConfig::default();
    config.storage.execution_run_ledger_path =
        Some(path.join("execution-runs.jsonl").display().to_string());
    config.storage.execution_ledger_path =
        Some(path.join("execution-events.jsonl").display().to_string());
    config.storage.order_snapshot_path =
        Some(path.join("order-snapshots.jsonl").display().to_string());
    config.storage.close_run_ledger_path =
        Some(path.join("close-runs.jsonl").display().to_string());
    config.storage.portfolio_nav_path = None;
    config.storage.watchlist_alerts_path = None;
    config.security.audit_log_path = None;
    config
}

pub(super) fn configure_e2e_webhook(state: &crate::state::AppState) -> anyhow::Result<()> {
    state.webhook().update_config(WebhookConfigPatch {
        enabled: Some(true),
        provider: Some(WebhookProvider::Bark),
        url: Some("https://api.day.app/crossline-paper-e2e".to_owned()),
        event_kinds: Some(vec![
            WebhookEventKind::Opportunity,
            WebhookEventKind::AutomationDecision,
            WebhookEventKind::ExecutionResult,
        ]),
        queue_capacity: Some(16),
        ..WebhookConfigPatch::default()
    })?;
    Ok(())
}

pub(super) fn configure_e2e_exit_policy(state: &crate::state::AppState) {
    state.trading_service().update_risk_config(|risk| {
        risk.auto_profit_close = AutoProfitCloseConfig {
            stop_loss_enabled: true,
            max_net_loss_usd: 0.01,
            max_loss_roi_bps: 1.0,
            confirmation_samples: 2,
            cooldown_secs: 10,
            ..AutoProfitCloseConfig::default()
        };
    });
}

pub(super) async fn assert_review_contains_succeeded_close(
    state: &crate::state::AppState,
    close_runs: &[shared_types::CloseRun],
) {
    let review = crate::services::review::executed_envelope_from_trading(
        state.trading_service(),
        close_runs,
        1,
        &crate::services::review::ReviewPageQuery::default(),
    )
    .await;
    assert!(review.rows.iter().any(|row| {
        row.evidence
            .close_run_evidence
            .iter()
            .any(|evidence| evidence.status == CloseRunStatus::Succeeded)
    }));
}
