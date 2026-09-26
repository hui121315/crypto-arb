use super::super::*;
use super::fixtures::*;
use super::*;

#[path = "cases_d/run_cost_rebuild.rs"]
mod run_cost_rebuild;

#[tokio::test]
async fn execution_cost_facts_replay_after_restart() -> anyhow::Result<()> {
    let path = std::env::temp_dir().join(format!(
        "crossline-execution-cost-replay-{}-{}.jsonl",
        std::process::id(),
        common::time::now_ms()
    ));
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    config.storage.execution_run_ledger_path = Some(path.display().to_string());
    let state = AppState::new(config.clone()).await?;
    let mut current = run("durable-cost", 1);
    current.state = ExecutionRunState::Unwinding;
    current.recovery_action = Some(RecoveryAction::UnwindLongLeg);
    current.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    current.short_leg = leg_with_order(HedgeLegRole::Short, "ex-short");
    current.cost_reconciliation = Some(cost());
    state
        .execution_runs()
        .insert(current.run_id.clone(), current.clone());

    let funding = ledger_funding_event_row(&current, HedgeLegRole::Long, "ex-long", -0.12);
    let mut unwind_fill = ledger_fill_event_row(
        &current,
        HedgeLegRole::Long,
        "ex-unwind",
        1.0,
        100.0,
        Some(0.4),
    );
    unwind_fill.order.reduce_only = Some(true);
    unwind_fill.order.side = OrderSide::Sell;
    let mut unwind_slippage =
        ledger_slippage_event_row(&current, HedgeLegRole::Long, "ex-unwind", 0.7);
    unwind_slippage.order.reduce_only = Some(true);

    assert_eq!(project_ledger_event_update(&state, &funding).len(), 1);
    assert_eq!(project_ledger_event_update(&state, &unwind_fill).len(), 1);
    assert_eq!(
        project_ledger_event_update(&state, &unwind_slippage).len(),
        1
    );

    let restored = AppState::new(config).await?;
    let replayed = restored
        .execution_runs()
        .get("durable-cost")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("replayed execution run missing"))?;
    let reconciliation = replayed
        .cost_reconciliation
        .ok_or_else(|| anyhow::anyhow!("replayed execution cost missing"))?;
    assert_eq!(reconciliation.actual_funding_usd, Some(-0.12));
    assert_eq!(reconciliation.actual_unwind_fee_usd, Some(0.4));
    assert_eq!(reconciliation.actual_unwind_slippage_usd, Some(0.7));
    assert_eq!(reconciliation.actual_unwind_cost_usd, Some(1.1));
    assert_eq!(reconciliation.funding_event_ids, [funding.event_id]);
    assert_eq!(
        reconciliation.unwind_event_ids,
        [unwind_fill.event_id, unwind_slippage.event_id]
    );

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn durable_projection_retry_after_append_does_not_double_incremental_fill(
) -> anyhow::Result<()> {
    let path = temp_projection_path("retry");
    let config = projection_config(&path);
    let state = AppState::new(config.clone()).await?;
    let mut current = run("durable-fill", 1);
    current.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    state
        .execution_runs()
        .insert(current.run_id.clone(), current.clone());
    let event = ledger_fill_event_row(
        &current,
        HedgeLegRole::Long,
        "ex-long",
        0.4,
        40.0,
        Some(0.04),
    );

    let mut snapshot = filled_record("ex-long", 0.04);
    snapshot.state = LiveOrderState::PartiallyFilled;
    snapshot.filled_quantity = Some(0.4);
    assert_eq!(project_order_update(&state, &snapshot).len(), 1);

    let first = project_ledger_event_update_durable(&state, &event)?;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].long_leg.filled_quantity, Some(0.4));

    let restored = AppState::new(config.clone()).await?;
    let retry = project_ledger_event_update_durable(&restored, &event)?;
    let replayed = restored
        .execution_runs()
        .get("durable-fill")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("replayed execution run missing"))?;

    assert!(retry.is_empty());
    assert_eq!(replayed.long_leg.filled_quantity, Some(0.4));
    assert_eq!(replayed.evidence.long_leg.ledger_fills.as_ref().map(|fills| fills.quantity), Some(0.4));
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn durable_projection_append_failure_leaves_hot_run_unchanged() -> anyhow::Result<()> {
    let blocking_parent = temp_projection_path("blocked-parent");
    std::fs::write(&blocking_parent, b"not a directory")?;
    let path = blocking_parent.join("execution-runs.jsonl");
    let state = AppState::new(projection_config(&path)).await?;
    let mut current = run("append-failure", 1);
    current.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    state
        .execution_runs()
        .insert(current.run_id.clone(), current.clone());
    let event = ledger_fill_event_row(&current, HedgeLegRole::Long, "ex-long", 0.4, 40.0, None);

    assert!(project_ledger_event_update_durable(&state, &event).is_err());
    let hot = state
        .execution_runs()
        .get("append-failure")
        .map(|entry| entry.value().clone())
        .ok_or_else(|| anyhow::anyhow!("hot execution run missing"))?;

    assert_eq!(hot.long_leg.filled_quantity, None);
    assert_eq!(hot.updated_at_ms, current.updated_at_ms);
    let _ = std::fs::remove_file(blocking_parent);
    Ok(())
}

#[tokio::test]
async fn durable_projection_noop_is_successful_without_poisoning_receipts() -> anyhow::Result<()> {
    let path = temp_projection_path("noop");
    let state = AppState::new(projection_config(&path)).await?;
    let unmatched = run("unmatched", 1);
    let event = ledger_fill_event_row(
        &unmatched,
        HedgeLegRole::Long,
        "ex-missing",
        0.4,
        40.0,
        None,
    );

    let projected = project_ledger_event_update_durable(&state, &event)?;

    assert!(projected.is_empty());
    assert!(!state
        .execution_run_store()
        .has_projection_receipt(trading::EXECUTION_RUN_PROJECTOR, &event.event_id));
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn successful_pair_close_projects_opening_run_to_closed_once() -> anyhow::Result<()> {
    let path = temp_projection_path("pair-close");
    let state = AppState::new(projection_config(&path)).await?;
    let opening = filled_opening("pair-close");
    state
        .execution_runs()
        .insert(opening.run_id.clone(), opening.clone());
    let close_run = successful_pair_close(&opening)?;

    let projected = project_close_run_update(&state, &close_run);

    assert_eq!(projected.len(), 1);
    assert_eq!(projected[0].state, ExecutionRunState::Closed);
    assert_eq!(projected[0].status_reason, "配对平仓已完成");
    assert_eq!(projected[0].finality_checked_at_ms, Some(20));
    assert!(projected[0]
        .evidence
        .events
        .iter()
        .any(|event| event.event_id == "close-run:close-pair-close:execution-closed"));
    assert!(project_close_run_update(&state, &close_run).is_empty());

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn incomplete_or_single_close_does_not_close_opening_run() -> anyhow::Result<()> {
    let path = temp_projection_path("single-close");
    let state = AppState::new(projection_config(&path)).await?;
    let opening = filled_opening("single-close");
    state
        .execution_runs()
        .insert(opening.run_id.clone(), opening.clone());
    let mut close_run = successful_pair_close(&opening)?;
    close_run.scope = shared_types::CloseRunScope::Single;

    assert!(project_close_run_update(&state, &close_run).is_empty());

    close_run.scope = shared_types::CloseRunScope::Pair;
    close_run.legs.pop();
    assert!(project_close_run_update(&state, &close_run).is_empty());
    assert_eq!(
        state
            .execution_runs()
            .get(&opening.run_id)
            .map(|run| run.state),
        Some(ExecutionRunState::Hedged)
    );

    let _ = std::fs::remove_file(path);
    Ok(())
}

#[tokio::test]
async fn recording_succeeded_pair_immediately_closes_opening_run() -> anyhow::Result<()> {
    let path = temp_projection_path("recorded-pair-close");
    let state = AppState::new(projection_config(&path)).await?;
    let opening = filled_opening("recorded-pair-close");
    state
        .execution_runs()
        .insert(opening.run_id.clone(), opening.clone());

    crate::services::close_runs::record(&state, successful_pair_close(&opening)?);

    let projected = state
        .execution_runs()
        .get(&opening.run_id)
        .map(|run| run.value().clone())
        .ok_or_else(|| anyhow::anyhow!("opening run missing"))?;
    assert_eq!(projected.state, ExecutionRunState::Closed);
    assert!(projected
        .evidence
        .events
        .iter()
        .any(|event| event.event_id == "close-run:close-recorded-pair-close:execution-closed"));

    let _ = std::fs::remove_file(path);
    Ok(())
}

fn projection_config(path: &std::path::Path) -> common::config::AppConfig {
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    config.storage.execution_run_ledger_path = Some(path.display().to_string());
    config
}

fn temp_projection_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "crossline-execution-projection-{label}-{}-{}",
        std::process::id(),
        common::time::now_ms()
    ))
}

fn successful_pair_close(run: &ExecutionRun) -> Result<shared_types::CloseRun, serde_json::Error> {
    serde_json::from_value(serde_json::json!({
        "id": format!("close-{}", run.run_id),
        "scope": "pair",
        "status": "succeeded",
        "requestId": "req-close",
        "snapshotVersion": "snapshot-1",
        "expectedLegCount": 2,
        "legs": [
            close_leg(run, "paper-long", "long", "paper-short", "short"),
            close_leg(run, "paper-short", "short", "paper-long", "long")
        ],
        "submittedOrderCount": 2,
        "failedLegCount": 0,
        "nakedExposureUsd": 0.0,
        "message": "平仓已完成",
        "startedAtMs": 10,
        "updatedAtMs": 20
    }))
}

fn filled_opening(id: &str) -> ExecutionRun {
    let mut run = run(id, 1);
    run.state = ExecutionRunState::Hedged;
    for (leg, venue) in [(&mut run.long_leg, "paper-long"), (&mut run.short_leg, "paper-short")] {
        leg.exchange = venue.into();
        leg.symbol = "BTC".into();
        leg.state = LiveOrderState::Filled;
        leg.filled_quantity = Some(1.0);
        leg.filled_notional_usd = Some(100.0);
        leg.confirmed_filled_at_ms = Some(2);
        leg.finality_source = Some(OrderUpdateSource::PrivateWs);
    }
    run
}

#[tokio::test]
async fn pair_close_requires_complete_quantities_and_exact_execution_context() -> anyhow::Result<()> {
    let path = temp_projection_path("pair-quantities");
    let state = AppState::new(projection_config(&path)).await?;
    let opening = filled_opening("pair-quantities");
    state.execution_runs().insert(opening.run_id.clone(), opening.clone());
    for defect in ["missing", "partial", "excess", "ticket", "opportunity", "venue", "symbol", "partner", "duplicate", "count", "ack"] {
        let mut close = successful_pair_close(&opening)?;
        let leg = &mut close.legs[0];
        match defect {
            "missing" => leg.order = None,
            "partial" => leg.order.as_mut().unwrap().filled_quantity = Some(0.5),
            "excess" => leg.order.as_mut().unwrap().filled_quantity = Some(1.5),
            "ticket" => leg.pair_evidence.as_mut().unwrap().ticket_id = "wrong".into(),
            "opportunity" => leg.pair_evidence.as_mut().unwrap().opportunity_id = "wrong".into(),
            "venue" => leg.venue = "wrong".into(),
            "symbol" => leg.pair_evidence.as_mut().unwrap().symbol = "ETH".into(),
            "partner" => leg.pair_evidence.as_mut().unwrap().partner_venue = "wrong".into(),
            "duplicate" => close.legs[1] = close.legs[0].clone(),
            "count" => close.expected_leg_count = 3,
            "ack" => { let order = leg.order.as_mut().unwrap(); order.intent.mode = ExecutionMode::Live; order.last_update_source = OrderUpdateSource::AdapterAck; }
            _ => unreachable!(),
        }
        assert!(project_close_run_update(&state, &close).is_empty(), "{defect}");
        assert_eq!(state.execution_runs().get(&opening.run_id).unwrap().state, ExecutionRunState::Hedged);
    }
    assert_eq!(project_close_run_update(&state, &successful_pair_close(&opening)?)[0].state, ExecutionRunState::Closed);
    let _ = std::fs::remove_file(path);
    Ok(())
}

fn close_leg(
    run: &ExecutionRun,
    venue: &str,
    side: &str,
    partner_venue: &str,
    partner_side: &str,
) -> serde_json::Value {
    serde_json::json!({
        "venue": venue,
        "symbol": "BTC",
        "side": side,
        "status": "filled",
        "quantity": 1.0,
        "markPrice": 100.0,
        "notionalUsd": 100.0,
        "finalitySource": "order_query",
        "confirmedFilledAtMs": 20,
        "order": {
            "intent": { "id": format!("close-{}-{side}", run.run_id), "clientOrderId": format!("close-{}-{side}", run.run_id),
                "source": "manual", "mode": "dry_run", "exchange": venue, "symbol": "BTC",
                "side": if side == "long" { "sell" } else { "buy" }, "orderType": "market", "quantity": 1.0,
                "price": 100.0, "reduceOnly": true, "createdAtMs": 10 },
            "state": "filled", "lastUpdateSource": "order_query", "filledQuantity": 1.0,
            "filledPrice": 100.0, "updatedAtMs": 20
        },
        "pairEvidence": {
            "source": "execution_run",
            "runId": run.run_id,
            "ticketId": run.ticket_id,
            "opportunityId": run.opportunity_id,
            "venue": venue,
            "symbol": "BTC",
            "side": side,
            "partnerVenue": partner_venue,
            "partnerSymbol": "BTC",
            "partnerSide": partner_side,
            "legFilledQuantity": 1.0,
            "partnerFilledQuantity": 1.0,
            "matchedNotionalUsd": 100.0,
            "updatedAtMs": 20
        }
    })
}
