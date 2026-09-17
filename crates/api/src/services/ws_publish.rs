use crate::state::AppState;
use common::AppError;
use realtime::{channels, WsMessage};
use shared_types::{
    CloseRun, CloseRunEvent, ExecutionRun, ExecutionRunEvent, OrderRecord,
    OrderStreamReconcileEvent, OrderStreamRecordEvent, RiskAlertEvent, TradingRiskStatus,
};
use trading::ReconcileDiff;

pub(crate) fn publish_order_event(
    state: &AppState,
    event: &'static str,
    record: &OrderRecord,
) -> Result<(), AppError> {
    publish_order_record_event(state, event, record)?;
    publish_projected_run_events(state, record)?;
    Ok(())
}

pub(crate) fn publish_order_record_event(
    state: &AppState,
    event: &'static str,
    record: &OrderRecord,
) -> Result<(), AppError> {
    state
        .ws_hub()
        .publish(channels::ORDERS, order_event_message(event, record)?);
    Ok(())
}

pub(crate) fn publish_execution_run_event(
    state: &AppState,
    event: &'static str,
    run: &ExecutionRun,
) -> Result<(), AppError> {
    state
        .ws_hub()
        .publish(channels::EXECUTION, execution_run_message(event, run)?);
    Ok(())
}

pub(crate) fn publish_reconcile_event(
    state: &AppState,
    event: &'static str,
    diffs: &[ReconcileDiff],
) -> Result<(), AppError> {
    state
        .ws_hub()
        .publish(channels::ORDERS, reconcile_event_message(event, diffs)?);
    Ok(())
}

pub(crate) fn publish_risk_event(
    state: &AppState,
    event: &'static str,
    risk: TradingRiskStatus,
) -> Result<(), AppError> {
    state
        .ws_hub()
        .publish(channels::RISK_ALERTS, risk_alert_message(event, risk)?);
    Ok(())
}

pub(crate) fn publish_automation_status(
    state: &AppState,
    status: &shared_types::AutomationRuntimeStatus,
) -> Result<(), AppError> {
    state
        .ws_hub()
        .publish_throttled(channels::AUTOMATION, WsMessage::json(status)?);
    Ok(())
}

pub(crate) fn publish_onchain_comparison(
    state: &AppState,
    snapshot: &shared_types::OnchainComparisonSnapshot,
) -> Result<(), AppError> {
    state
        .ws_hub()
        .publish_throttled(channels::ONCHAIN, WsMessage::json(snapshot)?);
    Ok(())
}

/// Webhook 运行态使用 Immediate 投递：配置变更和投递终态必须立刻到达界面，
/// 不进入批量窗口。调用方负责变化去重，见 [`crate::services::webhook::publish_status_if_changed`]。
pub(crate) fn publish_webhook_status(
    state: &AppState,
    status: &shared_types::WebhookRuntimeStatus,
) -> Result<(), AppError> {
    state
        .ws_hub()
        .publish(channels::WEBHOOK, WsMessage::json(status)?);
    Ok(())
}

fn order_event_message(event: &'static str, record: &OrderRecord) -> Result<WsMessage, AppError> {
    Ok(WsMessage::json(&OrderStreamRecordEvent {
        event: event.to_owned(),
        record: record.clone(),
        timestamp_ms: common::time::now_ms(),
    })?)
}

fn publish_projected_run_events(state: &AppState, record: &OrderRecord) -> Result<(), AppError> {
    for run in crate::services::execution_runs::project_order_update(state, record) {
        publish_execution_run_event(state, "execution_run_updated", &run)?;
    }
    for run in crate::services::close_runs::project_order_update(state, record) {
        publish_close_run_event(state, "close_run_updated", &run)?;
    }
    Ok(())
}

pub(crate) fn publish_close_run_event(
    state: &AppState,
    event: &'static str,
    run: &shared_types::CloseRun,
) -> Result<(), AppError> {
    state.ws_hub().notify_activity(channels::CLOSE_RUN_ACTIVITY);
    state
        .ws_hub()
        .publish(channels::PORTFOLIO, close_run_message(event, run)?);
    for execution_run in crate::services::execution_runs::project_close_run_update(state, run) {
        publish_execution_run_event(state, "execution_run_closed", &execution_run)?;
    }
    Ok(())
}

fn reconcile_event_message(
    event: &'static str,
    diffs: &[ReconcileDiff],
) -> Result<WsMessage, AppError> {
    Ok(WsMessage::json(&OrderStreamReconcileEvent {
        event: event.to_owned(),
        diffs: diffs.to_vec(),
        diff_count: diffs.len(),
        timestamp_ms: common::time::now_ms(),
    })?)
}

fn execution_run_message(event: &'static str, run: &ExecutionRun) -> Result<WsMessage, AppError> {
    Ok(WsMessage::json(&ExecutionRunEvent {
        event: event.to_owned(),
        execution_run: Some(run.clone()),
        timestamp_ms: common::time::now_ms(),
    })?)
}

fn risk_alert_message(event: &'static str, risk: TradingRiskStatus) -> Result<WsMessage, AppError> {
    Ok(WsMessage::json(&RiskAlertEvent {
        event: event.to_owned(),
        risk: Some(risk),
        execution_run: None,
        timestamp_ms: common::time::now_ms(),
    })?)
}

fn close_run_message(event: &'static str, run: &CloseRun) -> Result<WsMessage, AppError> {
    Ok(WsMessage::json(&CloseRunEvent {
        event: event.to_owned(),
        close_run: Some(run.clone()),
        timestamp_ms: common::time::now_ms(),
    })?)
}

#[cfg(test)]
#[path = "ws_publish/tests.rs"]
mod tests;
