mod config_store;

use crate::state::AppState;
use common::AppError;
use shared_types::{
    WebhookConfigPatch, WebhookEvent, WebhookEventKind, WebhookRuntimeStatus,
    WEBHOOK_EVENT_VERSION,
};
#[cfg(test)]
use uuid::Uuid;

pub(crate) async fn status(state: &AppState) -> WebhookRuntimeStatus {
    state.webhook().status(common::time::now_ms()).await
}

/// 读取当前运行态并在**除时间戳以外**有变化时推送到 `webhook` 通道。
///
/// `last_fingerprint` 由调用方持有：投递循环 30s 空闲心跳和配置无实际变化的重复
/// PATCH 都不会重复推送。零订阅者时跳过序列化且不记录 fingerprint，订阅者接入后
/// 的首包由 `ws_replay` 提供全量快照，之后第一次真实变化仍会推送。
pub(crate) async fn publish_status_if_changed(
    state: &AppState,
    last_fingerprint: &mut Option<String>,
) -> WebhookRuntimeStatus {
    let status = status(state).await;
    let fingerprint = status_fingerprint(&status);
    if last_fingerprint.as_deref() == Some(fingerprint.as_str()) {
        return status;
    }
    if state.ws_hub().subscriber_count(realtime::channels::WEBHOOK) == 0 {
        return status;
    }
    match crate::services::ws_publish::publish_webhook_status(state, &status) {
        Ok(()) => *last_fingerprint = Some(fingerprint),
        Err(error) => tracing::warn!(%error, "webhook runtime status serialization failed"),
    }
    status
}

/// 指纹排除 `updated_at_ms`：该字段每次读取都是当前时间，不代表运行态发生变化。
fn status_fingerprint(status: &WebhookRuntimeStatus) -> String {
    serde_json::json!({
        "config": status.config,
        "configurationProblem": status.configuration_problem,
        "queueDepth": status.queue_depth,
        "deliveredTotal": status.delivered_total,
        "failedTotal": status.failed_total,
        "droppedTotal": status.dropped_total,
        "recentDeliveries": status.recent_deliveries,
    })
    .to_string()
}

pub(crate) async fn update_config(
    state: &AppState,
    patch: WebhookConfigPatch,
) -> Result<WebhookRuntimeStatus, AppError> {
    let _mutation = state.webhook_config_mutation_lock().lock().await;
    ensure_configuration_restored(state)?;
    let prepared = state
        .webhook()
        .prepare_config(patch.clone())
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    config_store::persist(&patch, prepared.public())
        .await
        .map_err(|error| {
            tracing::error!(%error, "webhook config persistence failed");
            AppError::domain(axum::http::StatusCode::SERVICE_UNAVAILABLE,
                shared_types::problem::codes::WEBHOOK_CONFIG_STORAGE_FAILED,
                "Webhook 保存失败，当前投递配置未改变；请检查存储并核验原操作")
                .with_details(serde_json::json!({
                    "source": "webhook_config_store", "runtimeApplied": false,
                    "persistence": "unconfirmed",
                }))
        })?;
    state.webhook().apply_config(prepared);
    let status = status(state).await;
    if let Err(error) = crate::services::ws_publish::publish_webhook_status(state, &status) {
        tracing::warn!(%error, "updated webhook runtime status could not be published");
    }
    Ok(status)
}

pub(crate) fn restore_config(state: &AppState) -> Result<(), AppError> {
    config_store::restore(state.webhook()).map_err(|error| {
        state.webhook().block_configuration_restore(error.to_string());
        AppError::Config(error.to_string())
    })
}

pub(crate) async fn emit_idempotent(
    state: &AppState,
    kind: WebhookEventKind,
    event_id: String,
    payload: serde_json::Value,
) -> Result<String, AppError> {
    enqueue(state, event_with_id(kind, event_id, payload), false).await
}

#[cfg(test)]
pub(crate) async fn test(state: &AppState, message: Option<String>) -> Result<String, AppError> {
    test_with_id(state, message, format!("evt-{}", Uuid::new_v4())).await
}

pub(crate) async fn test_with_id(state: &AppState, message: Option<String>, id: String) -> Result<String, AppError> {
    ensure_configuration_restored(state)?;
    enqueue(
        state,
        event_with_id(
            WebhookEventKind::Test,
            id,
            serde_json::json!({
                "message": message.unwrap_or_else(|| "CROSSLINE webhook test".to_owned()),
            }),
        ),
        true,
    )
    .await
}

fn ensure_configuration_restored(state: &AppState) -> Result<(), AppError> {
    match state.webhook().configuration_problem() {
        Some(problem) => Err(AppError::domain(axum::http::StatusCode::SERVICE_UNAVAILABLE,
            shared_types::problem::codes::WEBHOOK_CONFIG_RESTORE_FAILED, problem.message)
            .with_details(serde_json::json!({ "source": "webhook_config_store", "phase": "restore",
                "deliveryBlocked": true, "originalFilePreserved": true }))),
        None => Ok(()),
    }
}

async fn enqueue(state: &AppState, event: WebhookEvent, force: bool) -> Result<String, AppError> {
    let id = event.id.clone();
    state
        .webhook()
        .enqueue(event, force)
        .await
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    Ok(id)
}

fn event_with_id(kind: WebhookEventKind, id: String, payload: serde_json::Value) -> WebhookEvent {
    WebhookEvent {
        id,
        version: WEBHOOK_EVENT_VERSION.to_owned(),
        kind,
        occurred_at_ms: common::time::now_ms(),
        payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn business_event_can_reuse_a_stable_id() {
        let event = event_with_id(
            WebhookEventKind::ExecutionResult,
            "execution-run-1-20".to_owned(),
            serde_json::json!({ "runId": "run-1" }),
        );

        assert_eq!(event.id, "execution-run-1-20");
        assert_eq!(event.version, WEBHOOK_EVENT_VERSION);
    }

    #[tokio::test]
    async fn test_delivery_queues_a_signed_test_event_while_disabled() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;
        update_config(
            &state,
            WebhookConfigPatch {
                url: Some("https://example.com/crossline".to_owned()),
                secret: Some("test-signing-secret".to_owned()),
                ..WebhookConfigPatch::default()
            },
        )
        .await?;

        let event_id = test(&state, Some("diagnostic".to_owned())).await?;
        let runtime = status(&state).await;

        assert!(event_id.starts_with("evt-"));
        assert!(!runtime.config.enabled);
        assert!(runtime.config.secret_configured);
        assert_eq!(runtime.queue_depth, 1);
        Ok(())
    }

    #[tokio::test]
    async fn status_fingerprint_ignores_the_read_timestamp() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;

        let first = status(&state).await;
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
        let second = status(&state).await;

        assert_ne!(
            first.updated_at_ms, second.updated_at_ms,
            "status reads must carry their own timestamp"
        );
        assert_eq!(status_fingerprint(&first), status_fingerprint(&second));
        Ok(())
    }

    #[tokio::test]
    async fn status_fingerprint_changes_when_the_queue_changes() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;
        update_config(
            &state,
            WebhookConfigPatch {
                url: Some("https://example.com/crossline".to_owned()),
                secret: Some("test-signing-secret".to_owned()),
                ..WebhookConfigPatch::default()
            },
        )
        .await?;
        let before = status_fingerprint(&status(&state).await);

        test(&state, None).await?;

        assert_ne!(before, status_fingerprint(&status(&state).await));
        Ok(())
    }

    /// 零订阅者时不记录 fingerprint：订阅者接入后的首包由 `ws_replay` 提供，
    /// 之后第一次真实变化必须仍能推送出去。
    #[tokio::test]
    async fn publish_without_subscribers_keeps_the_fingerprint_unset() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;
        let mut fingerprint = None;

        publish_status_if_changed(&state, &mut fingerprint).await;

        assert_eq!(
            state.ws_hub().subscriber_count(realtime::channels::WEBHOOK),
            0
        );
        assert!(fingerprint.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn subscribed_publish_records_then_dedups_an_unchanged_status() -> anyhow::Result<()> {
        let state = AppState::new(common::config::AppConfig::default()).await?;
        let mut receiver = state.ws_hub().subscribe(realtime::channels::WEBHOOK);
        let mut fingerprint = None;

        publish_status_if_changed(&state, &mut fingerprint).await;
        let published = fingerprint.clone();
        assert!(published.is_some(), "first status must be published");
        assert!(receiver.try_recv().is_ok(), "subscriber must receive it");

        publish_status_if_changed(&state, &mut fingerprint).await;

        assert_eq!(fingerprint, published);
        assert!(
            receiver.try_recv().is_err(),
            "an unchanged status must not be republished"
        );
        Ok(())
    }
}
