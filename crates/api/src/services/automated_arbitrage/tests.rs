use super::guards::{
    entry_blocker, exit_protection_blocker, EXIT_PROTECTION_REQUIRED, STOP_LOSS_CAPITAL_MISMATCH,
    TAKE_PROFIT_CAPITAL_MISMATCH,
};
use super::{control, status, update_config};
use crate::state::AppState;
use shared_types::{
    AutomatedArbitrageConfigPatch, AutomationControlAction, AutomationRuntimeState,
    ExecutionEnvironment,
};

#[tokio::test]
async fn config_and_pause_survive_restart() -> anyhow::Result<()> {
    let runtime = tempfile::tempdir()?;
    let mut config = common::config::AppConfig::default();
    config.storage.automation_config_path =
        Some(runtime.path().join("automation.json").display().to_string());
    let state = AppState::new(config.clone()).await?;
    update_config(
        &state,
        &AutomatedArbitrageConfigPatch {
            enabled: Some(true),
            environment: Some(ExecutionEnvironment::Live),
            capital_usd: Some(10.0),
            min_depth_usd: Some(10.0),
            cooldown_secs: Some(30),
            ..AutomatedArbitrageConfigPatch::default()
        },
        1,
    )
    .await?;
    control(&state, AutomationControlAction::Pause, 2).await?;
    drop(state);

    let restored = AppState::new(config).await?;
    let status = status(&restored);

    assert!(status.config.enabled);
    assert!(status.config.paused);
    assert_eq!(status.config.environment, ExecutionEnvironment::Live);
    assert_eq!(status.config.capital_usd, 10.0);
    assert_eq!(status.config.min_depth_usd, 10.0);
    assert_eq!(status.config.cooldown_secs, 30);
    assert_eq!(status.state, AutomationRuntimeState::Paused);
    Ok(())
}

#[tokio::test]
async fn default_runtime_is_disabled() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;

    assert_eq!(
        entry_blocker(&state, &status(&state), 0, 1),
        Some((AutomationRuntimeState::Disabled, "automation is disabled"))
    );
    Ok(())
}

#[tokio::test]
async fn environment_mismatch_fails_closed() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    update_config(
        &state,
        &AutomatedArbitrageConfigPatch {
            enabled: Some(true),
            environment: Some(ExecutionEnvironment::Live),
            ..AutomatedArbitrageConfigPatch::default()
        },
        1,
    )
    .await?;

    assert_eq!(
        entry_blocker(&state, &status(&state), 0, 2),
        Some((
            AutomationRuntimeState::Blocked,
            "automation mode does not match the trading runtime environment"
        ))
    );
    Ok(())
}

#[tokio::test]
async fn automated_entry_requires_an_enabled_pair_exit_policy() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    update_config(
        &state,
        &AutomatedArbitrageConfigPatch {
            enabled: Some(true),
            ..AutomatedArbitrageConfigPatch::default()
        },
        1,
    )
    .await?;

    assert_eq!(
        entry_blocker(&state, &status(&state), 0, 2),
        Some((
            AutomationRuntimeState::Blocked,
            "automatic entry requires take-profit, stop-loss, or liquidation protection"
        ))
    );
    state.trading_service().update_risk_config(|config| {
        config.auto_profit_close.enabled = true;
        config.auto_profit_close.min_net_profit_usd = 0.02;
    });
    assert_eq!(entry_blocker(&state, &status(&state), 0, 3), None);
    Ok(())
}

#[tokio::test]
async fn resume_requires_pair_exit_protection_before_state_mutation() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    update_config(
        &state,
        &AutomatedArbitrageConfigPatch {
            enabled: Some(true),
            paused: Some(true),
            ..AutomatedArbitrageConfigPatch::default()
        },
        1,
    )
    .await?;

    let Err(error) = control(&state, AutomationControlAction::Resume, 2).await else {
        anyhow::bail!("resume without exit protection did not fail closed");
    };
    assert!(matches!(
        error,
        common::AppError::BadRequest(message) if message == EXIT_PROTECTION_REQUIRED
    ));
    assert!(status(&state).config.paused);

    state.trading_service().update_risk_config(|risk| {
        risk.auto_profit_close.enabled = true;
        risk.auto_profit_close.min_net_profit_usd = 0.02;
    });
    let resumed = control(&state, AutomationControlAction::Resume, 3).await?;
    assert!(!resumed.config.paused);
    Ok(())
}

#[tokio::test]
async fn ten_dollar_live_resume_requires_capital_matched_exit_thresholds() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    update_config(
        &state,
        &AutomatedArbitrageConfigPatch {
            enabled: Some(true),
            paused: Some(false),
            capital_usd: Some(10.0),
            ..AutomatedArbitrageConfigPatch::default()
        },
        1,
    )
    .await?;
    state.trading_service().update_risk_config(|risk| {
        risk.auto_profit_close.enabled = true;
    });

    assert_eq!(
        exit_protection_blocker(&state, 10.0),
        Some(TAKE_PROFIT_CAPITAL_MISMATCH)
    );
    control(&state, AutomationControlAction::Pause, 2).await?;
    let Err(error) = control(&state, AutomationControlAction::Resume, 3).await else {
        anyhow::bail!("resume with mismatched take-profit did not fail closed");
    };
    assert!(matches!(
        error,
        common::AppError::BadRequest(message) if message == TAKE_PROFIT_CAPITAL_MISMATCH
    ));
    assert!(status(&state).config.paused);

    state.trading_service().update_risk_config(|risk| {
        risk.auto_profit_close.enabled = false;
        risk.auto_profit_close.stop_loss_enabled = true;
    });
    assert_eq!(
        exit_protection_blocker(&state, 10.0),
        Some(STOP_LOSS_CAPITAL_MISMATCH)
    );

    state.trading_service().update_risk_config(|risk| {
        risk.auto_profit_close.enabled = true;
        risk.auto_profit_close.min_net_profit_usd = 0.02;
        risk.auto_profit_close.max_net_loss_usd = 0.25;
    });
    let resumed = control(&state, AutomationControlAction::Resume, 5).await?;
    assert!(!resumed.config.paused);
    Ok(())
}

#[tokio::test]
async fn live_runtime_needs_no_per_order_confirmation_and_kill_switch_still_wins(
) -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    state.trading_service().update_risk_config(|risk| {
        risk.live_trading_enabled = true;
        risk.auto_profit_close.enabled = true;
        risk.auto_profit_close.min_net_profit_usd = 0.02;
    });
    update_config(
        &state,
        &AutomatedArbitrageConfigPatch {
            enabled: Some(true),
            environment: Some(ExecutionEnvironment::Live),
            ..AutomatedArbitrageConfigPatch::default()
        },
        1,
    )
    .await?;

    assert_eq!(entry_blocker(&state, &status(&state), 0, 2), None);

    state
        .trading_service()
        .update_risk_config(|risk| risk.kill_switch_active = true);
    assert_eq!(
        entry_blocker(&state, &status(&state), 0, 3),
        Some((
            AutomationRuntimeState::Blocked,
            "trading kill switch blocks new automated entries"
        ))
    );
    Ok(())
}
