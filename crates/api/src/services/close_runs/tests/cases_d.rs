#![allow(clippy::panic)]
use super::super::*;
use super::fixtures::*;

#[tokio::test]
async fn record_replays_preexisting_paper_fill_cost_and_finality() -> anyhow::Result<()> {
    let state = test_state().await;
    let order = state
        .trading_service()
        .submit(order_record("paper-close", LiveOrderState::Created).intent)
        .await?;
    let ledger = state
        .trading_service()
        .list_execution_ledger_events_by_query(&trading::ExecutionLedgerQuery {
            internal_order_id: Some(order.intent.id.clone()),
            exchange_order_id: None,
            hedge_group_id: None,
            run_id: None,
            ticket_id: None,
            leg_role: None,
            from_ms: None,
            to_ms: None,
            limit: 32,
        });
    assert!(
        ledger
            .iter()
            .any(|event| matches!(event.payload, ExecutionLedgerPayload::FillSnapshot(_))),
        "paper submit must synchronously expose its fill snapshot"
    );
    let mut leg = close_leg("paper-close", CloseLegStatus::Submitted);
    leg.order = Some(order);

    let projected = record(&state, close_run("close-paper", leg));

    let stored = state
        .close_runs()
        .get("close-paper")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("paper close run missing"))?;
    let cost = stored
        .cost_reconciliation
        .ok_or_else(|| anyhow::anyhow!("paper close cost missing"))?;
    assert_eq!(stored.status, CloseRunStatus::Succeeded);
    assert_eq!(projected.status, CloseRunStatus::Succeeded);
    assert_eq!(stored.legs[0].status, CloseLegStatus::Filled);
    assert!(stored.legs[0].confirmed_filled_at_ms.is_some());
    assert_eq!(cost.close_fee_usd, Some(0.05));
    assert_eq!(cost.close_slippage_usd, Some(0.0));
    assert_eq!(cost.funding_usd, Some(0.0));
    assert_eq!(
        cost.funding_event_ids,
        [format!("paper-funding-model:{}", stored.id)]
    );
    assert!(cost.missing_fields.is_empty());
    Ok(())
}

#[tokio::test]
async fn ledger_fee_and_slippage_refresh_close_run_cost_after_fill() {
    let state = test_state().await;
    record(
        &state,
        close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted)),
    );
    let mut filled = order_record("order-1", LiveOrderState::Filled);
    filled.last_update_source = OrderUpdateSource::OrderQuery;
    filled.filled_quantity = Some(1.0);
    filled.filled_price = Some(99.0);

    let first = project_order_update(&state, &filled);
    let first_cost = first[0]
        .cost_reconciliation
        .clone()
        .unwrap_or_else(|| panic!("first cost missing"));
    assert_eq!(first_cost.close_slippage_usd, None);
    assert_eq!(first_cost.total_actual_cost_usd, None);
    assert_eq!(
        first_cost.missing_fields,
        vec!["close_fee".to_owned(), "close_slippage".to_owned()]
    );

    let fee_updated = project_ledger_event_update(&state, &ledger_fee_event("order-1", 0.2));
    let fee_cost = fee_updated[0]
        .cost_reconciliation
        .clone()
        .unwrap_or_else(|| panic!("fee cost missing"));
    assert_eq!(fee_cost.close_fee_usd, Some(0.2));
    assert_eq!(fee_cost.close_slippage_usd, None);
    assert_eq!(fee_cost.total_actual_cost_usd, None);
    assert_eq!(fee_cost.missing_fields, vec!["close_slippage".to_owned()]);
    assert_eq!(
        fee_cost.close_fee_event_ids,
        vec!["fee:order-1:0.2".to_owned()]
    );

    let updated = project_ledger_event_update(&state, &ledger_slippage_event("order-1", 1.0));
    let cost = updated[0]
        .cost_reconciliation
        .clone()
        .unwrap_or_else(|| panic!("slippage cost missing"));
    assert_eq!(cost.close_fee_usd, Some(0.2));
    assert_eq!(cost.close_slippage_usd, Some(1.0));
    assert_eq!(cost.total_actual_cost_usd, Some(1.2));
    assert_eq!(
        cost.close_slippage_event_ids,
        vec!["slippage:order-1:1".to_owned()]
    );
    assert!(cost.missing_fields.is_empty());
}

#[tokio::test]
async fn ledger_partial_fill_keeps_close_run_submitted() {
    let state = test_state().await;
    record(
        &state,
        close_run("close-1", close_leg("order-1", CloseLegStatus::Submitted)),
    );
    let updated = project_ledger_event_update(&state, &ledger_fill_event("order-1", 0.4));
    assert_eq!(updated[0].status, CloseRunStatus::Submitted);
    assert_eq!(updated[0].legs[0].status, CloseLegStatus::PartiallyFilled);
    assert_eq!(
        updated[0].legs[0]
            .order
            .as_ref()
            .and_then(|order| order.filled_quantity),
        Some(0.4)
    );
}

#[tokio::test]
async fn close_cost_facts_replay_after_restart() -> anyhow::Result<()> {
    let path = temp_path("durable-cost-replay");
    let mut config = test_config();
    config.storage.close_run_ledger_path = Some(path.display().to_string());
    let state = AppState::new(config.clone()).await?;
    let mut leg = close_leg("order-1", CloseLegStatus::Submitted);
    leg.pair_evidence = Some(PositionPairEvidence {
        source: shared_types::PositionPairEvidenceSource::ExecutionRun,
        run_id: "run-1".to_owned(),
        ticket_id: "ticket-1".to_owned(),
        opportunity_id: "opp-1".to_owned(),
        venue: "binance".to_owned(),
        symbol: "MUUSDT".to_owned(),
        side: PositionSide::Long,
        partner_venue: "okx".to_owned(),
        partner_symbol: "MUUSDT".to_owned(),
        partner_side: PositionSide::Short,
        leg_filled_quantity: 1.0,
        partner_filled_quantity: 1.0,
        matched_notional_usd: 100.0,
        updated_at_ms: 1,
    });
    record(&state, close_run("close-durable", leg));
    let mut filled = order_record("order-1", LiveOrderState::Filled);
    filled.filled_quantity = Some(1.0);
    filled.filled_price = Some(99.0);
    let _ = project_order_update(&state, &filled);
    let _ = project_ledger_event_update(&state, &ledger_fee_event("order-1", 0.2));
    let _ = project_ledger_event_update(&state, &ledger_slippage_event("order-1", 1.0));
    let funding = ledger_funding_event(
        "open-order-1",
        Some("run-1"),
        Some("ticket-1"),
        Some(HedgeLegRole::Long),
        -0.25,
    );
    let _ = project_ledger_event_update(&state, &funding);

    let restored = AppState::new(config).await?;
    let replayed = restored
        .close_runs()
        .get("close-durable")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("replayed close run missing"))?;
    let reconciliation = replayed
        .cost_reconciliation
        .ok_or_else(|| anyhow::anyhow!("replayed close cost missing"))?;
    assert_eq!(reconciliation.close_fee_usd, Some(0.2));
    assert_eq!(reconciliation.close_slippage_usd, Some(1.0));
    assert_eq!(reconciliation.funding_usd, Some(-0.25));
    assert_eq!(reconciliation.total_actual_cost_usd, Some(0.95));
    assert_eq!(
        reconciliation.close_fee_event_ids,
        ["fee:order-1:0.2".to_owned()]
    );
    assert_eq!(
        reconciliation.close_slippage_event_ids,
        ["slippage:order-1:1".to_owned()]
    );
    assert_eq!(reconciliation.funding_event_ids, [funding.event_id]);

    let _ = std::fs::remove_file(path);
    Ok(())
}
