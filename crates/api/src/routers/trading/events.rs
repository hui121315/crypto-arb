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
    state
        .trading_runtime_config_store()
        .persist(state.trading_service().adapter_name(), &snapshot)
        .map_err(anyhow::Error::new)?;
    ws_publish::publish_risk_event(state, event, snapshot)
}
