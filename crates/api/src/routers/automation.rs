use crate::services::action_runs::{self, ActionRunStart};
use crate::services::automated_arbitrage;
use crate::services::execution_artifact;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use common::AppError;
use shared_types::{
    ActionRunKind, AutomatedArbitrageConfigPatch, AutomationControlRequest,
    AutomationRuntimeStatus, DeterministicExecutionArtifact, ExecutionArtifactBuildRequest,
    ExecutionArtifactValidationRequest, ExecutionArtifactValidationResponse,
};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/automation/status", get(status))
        .route(
            "/api/automation/execution-runs/:run_id",
            get(execution_receipt),
        )
        .route("/api/automation/config", patch(update_config))
        .route("/api/automation/control", post(control))
        .route(
            "/api/automation/execution-artifacts/build",
            post(build_execution_artifact),
        )
        .route(
            "/api/automation/execution-artifacts/validate",
            post(validate_execution_artifact),
        )
}

async fn status(State(state): State<AppState>) -> Json<AutomationRuntimeStatus> {
    Json((*automated_arbitrage::status(&state)).clone())
}

async fn execution_receipt(
    State(state): State<AppState>,
    Path(run_id): Path<String>,
) -> Result<Json<shared_types::AutomationExecutionReceipt>, AppError> {
    automated_arbitrage::execution_receipt(&state, &run_id).map(Json)
}

async fn update_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<AutomatedArbitrageConfigPatch>,
) -> Result<Json<AutomationRuntimeStatus>, AppError> {
    let claim = begin_action(
        &state,
        &headers,
        ActionRunKind::AutomationConfigUpdate,
        "automation config update accepted",
    )?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let result = automated_arbitrage::update_config(&state, &patch, common::time::now_ms())
        .await
        .map(|status| (*status).clone());
    action_runs::finish_result_with_payload(
        &state,
        &claim.run().id,
        result,
        "automation config updated",
    )
    .map(Json)
}

async fn control(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<AutomationControlRequest>,
) -> Result<Json<AutomationRuntimeStatus>, AppError> {
    let claim = begin_action(
        &state,
        &headers,
        ActionRunKind::AutomationControl,
        "automation control accepted",
    )?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let action = request.action;
    let result = automated_arbitrage::control(&state, action, common::time::now_ms())
        .await
        .map(|status| (*status).clone());
    action_runs::finish_result_with_payload(
        &state,
        &claim.run().id,
        result,
        "automation control applied",
    )
    .map(Json)
}

async fn build_execution_artifact(
    State(state): State<AppState>,
    Json(request): Json<ExecutionArtifactBuildRequest>,
) -> Result<Json<DeterministicExecutionArtifact>, AppError> {
    execution_artifact::build(&state, &request).map(Json)
}

async fn validate_execution_artifact(
    State(state): State<AppState>,
    Json(request): Json<ExecutionArtifactValidationRequest>,
) -> Result<Json<ExecutionArtifactValidationResponse>, AppError> {
    execution_artifact::validate(&state, &request).map(Json)
}

fn begin_action(
    state: &AppState,
    headers: &HeaderMap,
    kind: ActionRunKind,
    message: &'static str,
) -> Result<action_runs::ActionRunBegin, AppError> {
    action_runs::begin_idempotent(
        state,
        ActionRunStart::new(
            kind,
            headers,
            Some("automated-arbitrage".to_owned()),
            message,
        )
        .with_idempotency_key(action_runs::explicit_idempotency_key(headers)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn config_and_emergency_stop_are_observable() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;
        let updated = update_config(
            State(state.clone()),
            HeaderMap::new(),
            Json(AutomatedArbitrageConfigPatch {
                enabled: Some(true),
                ..AutomatedArbitrageConfigPatch::default()
            }),
        )
        .await?;
        assert!(updated.0.config.enabled);

        let stopped = control(
            State(state),
            HeaderMap::new(),
            Json(AutomationControlRequest {
                action: shared_types::AutomationControlAction::EmergencyStop,
            }),
        )
        .await?;
        assert!(!stopped.0.config.enabled);
        Ok(())
    }
}
