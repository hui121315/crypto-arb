use crate::services::action_runs::{self, ActionRunStart};
use crate::services::webhook;
use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use common::AppError;
use shared_types::{
    ActionRunKind, WebhookConfigPatch, WebhookRuntimeStatus, WebhookTestRequest,
    WebhookTestResponse,
};

pub(crate) fn router() -> Router<AppState> {
    Router::new()
        .route("/api/webhook/status", get(status))
        .route("/api/webhook/config", patch(update_config))
        .route("/api/webhook/test", post(test_delivery))
}

async fn status(State(state): State<AppState>) -> Json<WebhookRuntimeStatus> {
    Json(webhook::status(&state).await)
}

async fn update_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(patch): Json<WebhookConfigPatch>,
) -> Result<Json<WebhookRuntimeStatus>, AppError> {
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::WebhookConfigUpdate,
            &headers,
            Some("webhook-delivery".to_owned()),
            "webhook config update accepted",
        )
        .with_idempotency_key(action_runs::explicit_idempotency_key(&headers)),
    )?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let result = webhook::update_config(&state, patch).await;
    action_runs::finish_result_with_payload(
        &state,
        &claim.run().id,
        result,
        "webhook config updated",
    )
    .map(Json)
}

async fn test_delivery(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<WebhookTestRequest>,
) -> Result<Json<WebhookTestResponse>, AppError> {
    let claim = action_runs::begin_idempotent(
        &state,
        ActionRunStart::new(
            ActionRunKind::WebhookTest,
            &headers,
            Some("webhook-test".to_owned()),
            "webhook test enqueue accepted",
        )
        .with_idempotency_key(action_runs::explicit_idempotency_key(&headers)),
    )?;
    if claim.is_replayed() {
        return action_runs::replay_payload(claim.run()).map(Json);
    }
    let run = claim.run();
    let result = webhook::test_with_id(
        &state,
        request.message,
        format!("evt-webhook-test-{}", run.id),
    )
    .await
    .map(|event_id| WebhookTestResponse {
        event_id,
        queued: true,
        request_id: run.request_id.clone(),
        action_run_id: run.id.clone(),
        idempotency_key: run.idempotency_key.clone(),
    });
    let response = action_runs::finish_result_with_payload(
        &state,
        &run.id,
        result,
        "webhook test queued; delivery not yet confirmed",
    )?;
    if let Err(error) =
        crate::services::ws_publish::publish_webhook_status(&state, &webhook::status(&state).await)
    {
        tracing::warn!(%error, "queued webhook runtime status could not be published");
    }
    Ok(Json(response))
}
