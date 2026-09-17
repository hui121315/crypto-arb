mod guards;
mod worker;

#[cfg(test)]
mod tests;

use crate::state::AppState;
use common::AppError;
use shared_types::{
    AutomatedArbitrageConfigPatch, AutomationControlAction, AutomationRuntimeStatus,
};
use std::sync::Arc;

use guards::exit_protection_blocker;

pub(crate) use worker::evaluate_once;

pub(crate) fn status(state: &AppState) -> Arc<AutomationRuntimeStatus> {
    state.automation().snapshot()
}

pub(crate) async fn update_config(
    state: &AppState,
    patch: &AutomatedArbitrageConfigPatch,
    now_ms: i64,
) -> Result<Arc<AutomationRuntimeStatus>, AppError> {
    let _mutation = state.automation_mutation_lock().lock().await;
    let config = state
        .automation()
        .preview_config(patch)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    persist_config(state, config.clone()).await?;
    let status = state.automation().commit_config(config, now_ms);
    crate::services::ws_publish::publish_automation_status(state, &status)?;
    Ok(status)
}

pub(crate) async fn control(
    state: &AppState,
    action: AutomationControlAction,
    now_ms: i64,
) -> Result<Arc<AutomationRuntimeStatus>, AppError> {
    let _mutation = state.automation_mutation_lock().lock().await;
    if action == AutomationControlAction::Resume {
        let current = state.automation().snapshot();
        if let Some(reason) = exit_protection_blocker(state, current.config.capital_usd) {
            return Err(AppError::BadRequest(reason.to_owned()));
        }
    }
    let config = state.automation().preview_control_config(action);
    let status = if matches!(
        action,
        AutomationControlAction::Pause | AutomationControlAction::EmergencyStop
    ) {
        let status = state.automation().control(action, now_ms);
        persist_config(state, status.config.clone()).await?;
        status
    } else {
        persist_config(state, config).await?;
        state.automation().control(action, now_ms)
    };
    if let Err(error) = crate::services::ws_publish::publish_automation_status(state, &status) {
        tracing::warn!(%error, "automation status websocket publish failed");
    }
    Ok(status)
}

async fn persist_config(
    state: &AppState,
    config: shared_types::AutomatedArbitrageConfig,
) -> Result<(), AppError> {
    let store = Arc::clone(state.automation_config_store());
    tokio::task::spawn_blocking(move || store.persist(&config))
        .await
        .map_err(anyhow::Error::new)?
        .map_err(anyhow::Error::new)?;
    Ok(())
}
