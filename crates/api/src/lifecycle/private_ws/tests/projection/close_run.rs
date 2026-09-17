use super::super::projection_support::*;
use shared_types::{
    CloseLegStatus, CloseRunStatus, ExecutionMode, LiveOrderState, OrderSide, OrderUpdateSource,
};

#[tokio::test]
async fn private_ws_cancel_projects_close_run_failure_once_after_apply_ack() -> anyhow::Result<()> {
    let state = isolated_private_ws_state().await?;
    state.trading_service().update_risk_config(|config| {
        config.live_trading_enabled = true;
        config.allowed_exchanges.insert("mock".to_owned());
    });
    let mut intent = private_ws_intent("close-run-private-ws-cancel", OrderSide::Sell, true);
    intent.mode = ExecutionMode::Live;
    let record = state.trading_service().submit(intent).await?;
    assert_eq!(record.state, LiveOrderState::Accepted);
    crate::services::close_runs::record(&state, private_ws_close_run(&record));
    let occurred_at_ms = common::time::now_ms();

    super::super::super::apply::apply_events(
        &state,
        "mock",
        vec![private_order_cancel_event(&record, occurred_at_ms)?],
    )
    .await;

    let cancelled_order = state
        .trading_service()
        .get_order(&record.intent.id)
        .ok_or_else(|| anyhow::anyhow!("order missing after private WS cancel"))?;
    assert_eq!(cancelled_order.state, LiveOrderState::Cancelled);
    assert_eq!(
        cancelled_order.last_update_source,
        OrderUpdateSource::PrivateWs
    );

    let first = state
        .close_runs()
        .get("close-private-ws")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("close run missing after private WS cancel"))?;
    assert_eq!(first.status, CloseRunStatus::Failed);
    assert_eq!(first.legs[0].status, CloseLegStatus::Cancelled);
    assert_eq!(
        first.legs[0].finality_source,
        Some(OrderUpdateSource::PrivateWs)
    );
    assert_eq!(first.naked_exposure_usd, 100.0);
    assert!(first.problem.is_some());

    super::super::super::apply::apply_events(
        &state,
        "mock",
        vec![private_order_cancel_event(&record, occurred_at_ms)?],
    )
    .await;

    let duplicate = state
        .close_runs()
        .get("close-private-ws")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("close run missing after duplicate cancel"))?;
    assert_eq!(duplicate, first);
    Ok(())
}
