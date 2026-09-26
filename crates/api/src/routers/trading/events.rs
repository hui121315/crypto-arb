use crate::services::ws_publish;
use crate::state::AppState;
use common::AppError;
use shared_types::OrderRecord;
use trading::RiskConfig;

pub(super) fn publish_order_event(
    state: &AppState,
    event: &'static str,
    record: &OrderRecord,
) -> Result<(), AppError> {
    ws_publish::publish_order_event(state, event, record)
}

pub(super) fn publish_risk_event(
    state: &AppState,
    event: &'static str,
    risk: &RiskConfig,
) -> Result<(), AppError> {
    let snapshot = crate::services::risk_config::snapshot(risk);
    ws_publish::publish_risk_event(state, event, snapshot)
}

pub(super) fn persist_risk_config(
    state: &AppState,
    adapter_id: &str,
    risk: &RiskConfig,
    emergency_stop_applied: bool,
) -> Result<(), AppError> {
    state
        .trading_runtime_config_store()
        .persist(adapter_id, &crate::services::risk_config::snapshot(risk))
        .map_err(|error| {
            tracing::error!(%error, emergency_stop_applied, "trading configuration persistence failed");
            AppError::domain(
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                shared_types::problem::codes::TRADING_CONFIG_STORAGE_FAILED,
                if emergency_stop_applied {
                    "当前进程已急停，但保存失败；重启后不保证恢复急停，请修复存储并重新核验"
                } else {
                    "配置保存失败，运行状态未改变；请检查后端存储后重试"
                },
            )
            .with_details(serde_json::json!({
                "source": "trading_runtime_config",
                "runtimeApplied": emergency_stop_applied,
                "persisted": false,
            }))
        })
}
