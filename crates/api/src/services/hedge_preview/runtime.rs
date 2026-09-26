use super::{execution_mode_for_adapter, AppError, AppState, HedgePreviewResponse, StatusCode};
use shared_types::{problem::codes, HedgeExecutionBinding};
use std::sync::Arc;
use trading::ExecutionEngine;

fn current(state: &AppState) -> HedgeExecutionBinding {
    let service = state.trading_service();
    let adapter = service.adapter_name();
    HedgeExecutionBinding {
        adapter: adapter.to_owned(),
        account_epoch: service.account_cache_epoch(),
        mode: execution_mode_for_adapter(adapter),
        live_enabled: service.risk_config().live_trading_enabled,
    }
}

pub(crate) async fn capture(state: &AppState) -> HedgeExecutionBinding {
    let _config = state.trading_runtime_config_mutation_lock().lock().await;
    current(state)
}

pub(crate) fn validate(state: &AppState, preview: &HedgePreviewResponse) -> Result<(), AppError> {
    let current = current(state);
    if preview.execution_binding.as_ref() == Some(&current)
        && preview.long_leg.mode == current.mode
        && preview.short_leg.mode == current.mode
    {
        return Ok(());
    }
    Err(AppError::domain(
        StatusCode::CONFLICT,
        codes::HEDGE_EXECUTION_CONTEXT_CHANGED,
        "执行账户或环境已改变，原票据不能继续开仓，请重新构建并校验",
    )
    .with_details(serde_json::json!({
        "source": "hedge_preview.execution_binding",
        "ticketId": preview.ticket.ticket_id,
        "expected": preview.execution_binding,
        "current": current,
    })))
}

pub(crate) async fn ensure_current(
    state: &AppState,
    preview: &HedgePreviewResponse,
) -> Result<(), AppError> {
    let _config = state.trading_runtime_config_mutation_lock().lock().await;
    validate(state, preview)
}

pub(crate) async fn bind_engine(
    state: &AppState,
    preview: &HedgePreviewResponse,
) -> Result<Arc<ExecutionEngine>, AppError> {
    let _config = state.trading_runtime_config_mutation_lock().lock().await;
    validate(state, preview)?;
    // Release the configuration lock before exchange I/O; every leg keeps this adapter.
    Ok(state.trading_service().capture_submission_engine())
}
