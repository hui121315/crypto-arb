use crate::middleware::audit;
use crate::services::action_runs::{self, ActionRunStart};
use crate::services::webhook;
use crate::state::AppState;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use common::AppError;
use serde::Serialize;
use serde_json::json;
use shared_types::{ActionRunKind, WebhookConfigPatch, WebhookRuntimeStatus, WebhookTestRequest};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WebhookTestResponse {
    event_id: String,
    queued: bool,
}

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
    let result = match webhook::update_config(&state, patch).await {
        Ok(_) => Ok(webhook::status(&state).await),
        Err(error) => Err(error),
    };
    // 配置更新成功即推送：订阅者不必等下一次投递或轮询就能看到生效后的运行态。
    // 投递循环持有自己的 fingerprint，因此最多产生一次相同快照的重复推送，
    // 客户端按整包替换处理，不会产生可见抖动。
    if let Ok(status) = result.as_ref() {
        if let Err(error) = crate::services::ws_publish::publish_webhook_status(&state, status) {
            tracing::warn!(%error, "updated webhook runtime status could not be published");
        }
    }
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
    let event_id = webhook::test(&state, request.message).await?;
    audit::record_http_event(
        &headers,
        "webhook.test.enqueue",
        "webhook-delivery",
        "success",
        json!({ "eventId": event_id }),
    );
    // 入队本身就是运行态变化（队列深度）；投递终态随后由投递循环推送。
    if let Err(error) =
        crate::services::ws_publish::publish_webhook_status(&state, &webhook::status(&state).await)
    {
        tracing::warn!(%error, "queued webhook runtime status could not be published");
    }
    Ok(Json(WebhookTestResponse {
        event_id,
        queued: true,
    }))
}
