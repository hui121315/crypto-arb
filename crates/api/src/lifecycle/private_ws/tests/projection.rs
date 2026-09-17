use super::projection_support::*;
use crate::trading_service::private_ws_events::PrivateWsEvent;
use shared_types::{CloseLegStatus, HedgeLegRole, OrderSide, VenueOperationStatus};

mod close_run;
mod event_bus;

#[tokio::test]
async fn binance_fill_durability_failure_blocks_health_and_projection() -> anyhow::Result<()> {
    let state = isolated_private_ws_state_with_unavailable_sql().await?;
    let run = private_ws_unwind_run();
    state.execution_runs().insert(run.run_id.clone(), run);
    let record = state
        .trading_service()
        .submit_unwind_with_ledger_context(
            private_ws_intent("order-private-ws-failed-ack", OrderSide::Buy, true),
            trading::ExecutionLedgerOrderContext::new(
                "run-private-ws".to_owned(),
                "ticket-private-ws".to_owned(),
                HedgeLegRole::Long,
            ),
        )
        .await?;
    let event =
        binance_private_trade_event(&record, "binance_trade:failed-ack:1", 102.0, 0.5, 100)?;

    super::super::apply::apply_events(&state, "binance", vec![event]).await;

    let health = state
        .private_ws_health()
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == crate::services::private_ws_health::OP_PRIVATE_WS_ORDER_STREAM)
        .ok_or_else(|| anyhow::anyhow!("private WS order health missing"))?;
    assert_eq!(health.status, VenueOperationStatus::Blocked);
    assert_eq!(health.ok_count, 0);
    assert_eq!(health.blocked_count, 1);
    assert!(health
        .error
        .as_deref()
        .is_some_and(|error| error.contains("durable writer is unavailable")));

    let projected = state
        .execution_runs()
        .get("run-private-ws")
        .ok_or_else(|| anyhow::anyhow!("execution run missing after failed durability ACK"))?;
    let costs = projected
        .cost_reconciliation
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("execution cost reconciliation missing"))?;
    assert_eq!(costs.actual_unwind_fee_usd, None);
    assert_eq!(costs.actual_unwind_slippage_usd, None);
    assert!(state.private_account_refresh_queue().drain().is_empty());
    Ok(())
}

#[tokio::test]
async fn private_fill_chain_projects_execution_slippage_once() -> anyhow::Result<()> {
    let state = isolated_private_ws_state().await?;
    let run = private_ws_unwind_run();
    state.execution_runs().insert(run.run_id.clone(), run);
    let record = state
        .trading_service()
        .submit_unwind_with_ledger_context(
            private_ws_intent("order-private-ws", OrderSide::Buy, true),
            trading::ExecutionLedgerOrderContext::new(
                "run-private-ws".to_owned(),
                "ticket-private-ws".to_owned(),
                HedgeLegRole::Long,
            ),
        )
        .await?;
    let fill = private_fill(&record, "venue-fill-private-ws", 102.0, 0.5, 100)?;

    super::super::apply::apply_events(&state, "mock", vec![PrivateWsEvent::Fill(fill.clone())])
        .await;

    let projected = state
        .execution_runs()
        .get("run-private-ws")
        .ok_or_else(|| anyhow::anyhow!("execution run missing after projection"))?;
    let first_cost = projected
        .cost_reconciliation
        .clone()
        .ok_or_else(|| anyhow::anyhow!("execution cost reconciliation missing"))?;
    let first_updated_at_ms = projected.updated_at_ms;
    drop(projected);
    assert_eq!(first_cost.actual_unwind_fee_usd, Some(0.5));
    assert_eq!(first_cost.actual_unwind_slippage_usd, Some(2.0));
    assert_eq!(first_cost.actual_unwind_cost_usd, Some(2.5));
    assert_eq!(first_cost.unwind_event_ids.len(), 2);
    assert_eq!(
        private_fill_projection_count(&state),
        2,
        "{:#?}",
        state
            .trading_service()
            .list_execution_ledger_events()
            .iter()
            .map(|event| (&event.event_type, &event.event_id))
            .collect::<Vec<_>>()
    );

    super::super::apply::apply_events(&state, "mock", vec![PrivateWsEvent::Fill(fill)]).await;

    let duplicate = state
        .execution_runs()
        .get("run-private-ws")
        .ok_or_else(|| anyhow::anyhow!("execution run missing after duplicate"))?;
    assert_eq!(duplicate.cost_reconciliation.as_ref(), Some(&first_cost));
    assert_eq!(duplicate.updated_at_ms, first_updated_at_ms);
    assert_eq!(private_fill_projection_count(&state), 2);
    Ok(())
}

#[tokio::test]
async fn binance_terminal_fill_projects_execution_and_health_once_after_ack() -> anyhow::Result<()>
{
    let state = isolated_private_ws_state().await?;
    let run = private_ws_unwind_run();
    state.execution_runs().insert(run.run_id.clone(), run);
    let record = state
        .trading_service()
        .submit_unwind_with_ledger_context(
            private_ws_intent("order-binance-finality", OrderSide::Buy, true),
            trading::ExecutionLedgerOrderContext::new(
                "run-private-ws".to_owned(),
                "ticket-private-ws".to_owned(),
                HedgeLegRole::Long,
            ),
        )
        .await?;
    let event =
        binance_private_trade_event(&record, "binance_trade:mock-order:9001", 102.0, 0.5, 100)?;

    super::super::apply::apply_events(&state, "binance", vec![event]).await;

    let projected = state
        .execution_runs()
        .get("run-private-ws")
        .ok_or_else(|| anyhow::anyhow!("execution run missing after Binance finality"))?;
    assert_eq!(projected.state, shared_types::ExecutionRunState::Closed);
    assert_eq!(projected.net_exposure_usd, 0.0);
    let first_cost = projected
        .cost_reconciliation
        .clone()
        .ok_or_else(|| anyhow::anyhow!("execution cost reconciliation missing"))?;
    let first_updated_at_ms = projected.updated_at_ms;
    drop(projected);
    assert_eq!(first_cost.actual_unwind_fee_usd, Some(0.5));
    assert_eq!(first_cost.actual_unwind_slippage_usd, Some(2.0));
    assert_eq!(private_fill_projection_count(&state), 2);

    let health = state
        .private_ws_health()
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == crate::services::private_ws_health::OP_PRIVATE_WS_ORDER_STREAM)
        .ok_or_else(|| anyhow::anyhow!("Binance private order health missing"))?;
    assert_eq!(health.status, VenueOperationStatus::Ok);
    assert_eq!(health.ok_count, 1);
    assert_eq!(health.blocked_count, 0);

    let duplicate_event =
        binance_private_trade_event(&record, "binance_trade:mock-order:9001", 102.0, 0.5, 100)?;
    super::super::apply::apply_events(&state, "binance", vec![duplicate_event]).await;
    let duplicate = state
        .execution_runs()
        .get("run-private-ws")
        .ok_or_else(|| anyhow::anyhow!("execution run missing after duplicate"))?;
    assert_eq!(duplicate.cost_reconciliation.as_ref(), Some(&first_cost));
    assert_eq!(duplicate.updated_at_ms, first_updated_at_ms);
    assert_eq!(private_fill_projection_count(&state), 2);
    let health = state
        .private_ws_health()
        .snapshot(common::time::now_ms())
        .into_iter()
        .find(|row| row.operation == crate::services::private_ws_health::OP_PRIVATE_WS_ORDER_STREAM)
        .ok_or_else(|| anyhow::anyhow!("Binance private order health missing"))?;
    assert_eq!(health.ok_count, 1);
    Ok(())
}

#[tokio::test]
async fn binance_fill_chain_projects_close_and_review_slippage_once() -> anyhow::Result<()> {
    let state = isolated_private_ws_state().await?;
    let long = state
        .trading_service()
        .submit(private_ws_intent(
            "review-chain-long",
            OrderSide::Buy,
            false,
        ))
        .await?;
    let short = state
        .trading_service()
        .submit(private_ws_intent(
            "review-chain-short",
            OrderSide::Sell,
            false,
        ))
        .await?;
    crate::services::close_runs::record(&state, private_ws_close_run(&short));
    let occurred_at_ms = common::time::now_ms();
    let events = binance_review_trade_events(&long, &short, occurred_at_ms)?;

    super::super::apply::apply_events(&state, "binance", events).await;

    let first_close = state
        .close_runs()
        .get("close-private-ws")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("close run missing after projection"))?;
    let close_cost = first_close
        .cost_reconciliation
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("close cost reconciliation missing"))?;
    assert_eq!(first_close.legs[0].status, CloseLegStatus::Filled);
    assert_eq!(
        first_close.legs[0]
            .order
            .as_ref()
            .and_then(|row| row.filled_quantity),
        Some(1.0)
    );
    assert_eq!(close_cost.close_slippage_usd, Some(1.0));
    assert_eq!(close_cost.close_slippage_event_ids.len(), 1);

    let ledger = state.trading_service().list_execution_ledger_events();
    let review =
        crate::services::review::executed(&state.trading_service().list_orders(), &ledger, 1);
    let trade = review
        .iter()
        .find(|trade| trade.id == "review-chain")
        .ok_or_else(|| anyhow::anyhow!("review trade missing after projection"))?;
    let first_review_evidence = trade.evidence.clone();
    assert_eq!(trade.slippage_usd, 2.0);
    assert_eq!(trade.evidence.fill_event_ids.len(), 2);
    assert_eq!(trade.evidence.slippage_event_ids.len(), 2);
    assert_eq!(private_fill_projection_count(&state), 4);

    let duplicate_events = binance_review_trade_events(&long, &short, occurred_at_ms)?;
    super::super::apply::apply_events(&state, "binance", duplicate_events).await;

    let duplicate_close = state
        .close_runs()
        .get("close-private-ws")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("close run missing after duplicate"))?;
    assert_eq!(duplicate_close, first_close);
    let duplicate_ledger = state.trading_service().list_execution_ledger_events();
    let duplicate_review = crate::services::review::executed(
        &state.trading_service().list_orders(),
        &duplicate_ledger,
        1,
    );
    let duplicate_trade = duplicate_review
        .iter()
        .find(|trade| trade.id == "review-chain")
        .ok_or_else(|| anyhow::anyhow!("review trade missing after duplicate"))?;
    assert_eq!(duplicate_trade.evidence, first_review_evidence);
    assert_eq!(private_fill_projection_count(&state), 4);
    Ok(())
}
