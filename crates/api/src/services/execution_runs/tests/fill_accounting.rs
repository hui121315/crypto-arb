use super::super::*;
use super::fixtures::*;
use super::*;

fn opened() -> ExecutionRun {
    let mut run = run("accounting", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");
    run.short_leg = leg_with_order(HedgeLegRole::Short, "short");
    apply_order_update(&mut run, &filled_record("long", 0.1));
    let mut cancelled = filled_record("short", 0.0);
    cancelled.state = LiveOrderState::Cancelled;
    cancelled.filled_quantity = Some(0.0);
    apply_order_update(&mut run, &cancelled);
    run.long_leg.order_ids.push("unwind".into());
    run
}

fn recovery(quantity: Option<f64>) -> OrderRecord {
    let mut record = filled_record("unwind", 0.0).reduce_only();
    record.intent.side = OrderSide::Sell;
    record.filled_quantity = quantity;
    record
}

#[test]
fn fill_accounting_snapshots_and_individual_fills_do_not_double_count() -> anyhow::Result<()> {
    for snapshot_first in [false, true] {
        let mut run = run("mixed-fills", 1);
        run.long_leg = leg_with_order(HedgeLegRole::Long, "long");
        let mut record = filled_record("long", 0.04);
        record.filled_quantity = Some(0.4);
        record.state = LiveOrderState::PartiallyFilled;
        run.long_leg.identity = Some(record.identity_snapshot());
        let first = ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 0.4, 40.0, Some(0.04));
        if snapshot_first { assert!(apply_order_update(&mut run, &record)); }
        assert!(apply_ledger_event_update(&mut run, &first));
        if !snapshot_first {
            assert!(apply_order_update(&mut run, &record));
        }
        assert_eq!(run.long_leg.filled_quantity, Some(0.4));
        assert_eq!(run.long_leg.filled_fee, Some(0.04));
        let mut restored: ExecutionRun = serde_json::from_value(serde_json::to_value(run)?)?;
        let mut second = ledger_fill_event_row(&restored, HedgeLegRole::Long, "ex-long", 0.6, 60.0, Some(0.06));
        second.event_id.push_str(":second");
        assert!(apply_ledger_event_update(&mut restored, &second));
        assert_eq!(restored.long_leg.filled_quantity, Some(1.0));
        assert_eq!(restored.long_leg.filled_notional_usd, Some(100.0));
        assert_close_option(restored.long_leg.filled_fee, 0.1).map_err(anyhow::Error::msg)?;
        assert!(apply_order_update(&mut restored, &filled_record("long", 0.1)));
        assert_eq!(restored.long_leg.filled_quantity, Some(1.0));
    }
    Ok(())
}

#[test]
fn fill_accounting_recovery_requires_quantities_and_retains_remaining_position() {
    let mut run = opened();
    assert!(apply_order_update(&mut run, &recovery(None)));
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.net_exposure_usd, 100.0);
    assert!(apply_order_update(&mut run, &recovery(Some(0.4))));
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.net_exposure_usd, 60.0);
    assert_eq!(run.long_leg.filled_quantity, Some(1.0));
    assert!(apply_order_update(&mut run, &recovery(Some(1.0))));
    assert_eq!(run.state, ExecutionRunState::Closed);
    assert_eq!(run.net_exposure_usd, 0.0);
    assert!(apply_order_update(&mut run, &recovery(Some(1.0))));
    assert_eq!(run.net_exposure_usd, 0.0);
    let mut stale = recovery(Some(0.0)); stale.state = LiveOrderState::Accepted;
    assert!(!apply_order_update(&mut run, &stale));
    assert_eq!(run.state, ExecutionRunState::Closed);
}

#[test]
fn fill_accounting_recovery_does_not_hide_other_leg_or_unknown_quantity() {
    let mut run = opened();
    run.short_leg.filled_quantity = Some(0.5);
    run.short_leg.filled_notional_usd = Some(50.0);
    assert!(apply_order_update(&mut run, &recovery(Some(1.0))));
    assert_eq!(run.net_exposure_usd, -50.0);
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    let mut unknown = opened(); unknown.short_leg.filled_quantity = None;
    apply_order_update(&mut unknown, &recovery(Some(1.0)));
    assert_ne!(unknown.state, ExecutionRunState::Closed);
    assert!(unknown.unwind_problem.is_some());
    for alter in ["venue", "side"] {
        let mut wrong = recovery(Some(1.0));
        if alter == "venue" { wrong.intent.exchange = "other".into(); }
        else { wrong.intent.side = OrderSide::Buy; }
        assert!(!apply_order_update(&mut unknown, &wrong));
    }
}

#[test]
fn fill_accounting_ledger_recovery_waits_for_quantity_then_closes_once() -> anyhow::Result<()> {
    let mut run = opened();
    let mut event = ledger_state_event_row(&run, HedgeLegRole::Long, "unwind", ExecutionLedgerEventType::OrderState, LiveOrderState::Filled);
    event.order.reduce_only = Some(true); event.order.side = OrderSide::Sell;
    event.order.identity.internal_order_id = "unwind".into();
    assert!(apply_ledger_event_update(&mut run, &event));
    assert_ne!(run.state, ExecutionRunState::Closed);
    let mut fill = ledger_fill_event_row(&run, HedgeLegRole::Long, "unwind", 0.4, 40.0, Some(0.01));
    fill.order = event.order.clone();
    assert!(apply_ledger_event_update(&mut run, &fill));
    assert_eq!(run.net_exposure_usd, 60.0);
    let mut restored: ExecutionRun = serde_json::from_value(serde_json::to_value(run)?)?;
    fill.event_id.push_str(":second");
    if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut fill.payload { fill.quantity = 0.6; fill.quote_value = 60.0; }
    assert!(apply_ledger_event_update(&mut restored, &fill));
    assert_eq!(restored.state, ExecutionRunState::Closed);
    apply_order_update(&mut restored, &recovery(Some(1.0)));
    assert_eq!(restored.net_exposure_usd, 0.0);
    assert_eq!(restored.long_leg.filled_quantity, Some(1.0));
    Ok(())
}

#[tokio::test]
async fn fill_accounting_stale_run_save_keeps_fills_costs_and_new_plan() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.data_dir = dir.path().display().to_string();
    config.storage.portfolio_nav_path = None;
    config.storage.execution_run_ledger_path = Some(dir.path().join("runs.jsonl").display().to_string());
    let state = AppState::new(config.clone()).await?;
    let mut initial = run("stale-save", 1);
    initial.long_leg = leg_with_order(HedgeLegRole::Long, "long");
    initial.short_leg = leg_with_order(HedgeLegRole::Short, "short");
    initial.cost_reconciliation = Some(cost());
    let mut old = record(&state, initial);
    assert_eq!(project_order_update(&state, &filled_record("long", 0.1)).len(), 1);
    let funding = ledger_funding_event_row(&old, HedgeLegRole::Long, "ex-long", -0.2);
    project_ledger_event_update_durable(&state, &funding)?;
    old.short_leg.target_quantity = 0.9;
    old.short_leg.target_notional_usd = 90.0;
    old.state = ExecutionRunState::SecondLegSubmitted;
    old.updated_at_ms = common::time::now_ms() + 1;
    let saved = record(&state, old.clone());
    assert_eq!(saved.long_leg.filled_quantity, Some(1.0));
    assert_eq!(saved.long_leg.filled_notional_usd, Some(100.0));
    assert_eq!(saved.long_leg.filled_fee, Some(0.1));
    assert_eq!(saved.long_leg.state, LiveOrderState::Filled);
    assert!(saved.long_leg.order_ids.contains(&"ex-long".to_owned()));
    assert_eq!(saved.short_leg.target_quantity, 0.9);
    assert_eq!(saved.state, ExecutionRunState::SecondLegSubmitted);
    assert_eq!(saved.cost_reconciliation.as_ref().unwrap().actual_funding_usd, Some(-0.2));
    let restored = AppState::new(config).await?;
    let restored = record(&restored, old);
    assert_eq!(restored.long_leg, saved.long_leg);
    assert_eq!(restored.cost_reconciliation, saved.cost_reconciliation);
    Ok(())
}

#[tokio::test]
async fn fill_accounting_stale_save_preserves_recovery_and_does_not_close_early() -> anyhow::Result<()> {
    let mut config = common::config::AppConfig::default();
    config.history.enabled = false;
    config.storage.portfolio_nav_path = None;
    let state = AppState::new(config).await?;
    let mut old = record(&state, opened());
    old.long_leg.filled_quantity = None;
    old.long_leg.filled_notional_usd = None;
    old.long_leg.state = LiveOrderState::Submitted;
    old.evidence = Default::default();
    project_order_update(&state, &recovery(Some(0.4)));
    let partial = record(&state, old.clone());
    assert_eq!(partial.long_leg.filled_quantity, Some(1.0));
    assert_eq!(partial.net_exposure_usd, 60.0);
    assert_ne!(partial.state, ExecutionRunState::Closed);
    project_order_update(&state, &recovery(Some(1.0)));
    let done = record(&state, old);
    assert_eq!(done.state, ExecutionRunState::Closed);
    assert_eq!(done.net_exposure_usd, 0.0);
    assert_eq!(done.evidence.recovery_orders.len(), 1);
    Ok(())
}

#[test]
fn fill_accounting_multiple_recovery_orders_require_all_remaining_quantity() {
    let mut run = opened();
    let mut first = recovery(Some(0.4));
    first.state = LiveOrderState::Cancelled;
    assert!(apply_order_update(&mut run, &first));
    assert_eq!(run.net_exposure_usd, 60.0);
    run.long_leg.order_ids.push("unwind-2".into());
    let mut second = filled_record("unwind-2", 0.0).reduce_only();
    second.intent.side = OrderSide::Sell;
    second.state = LiveOrderState::PartiallyFilled;
    second.filled_quantity = Some(0.5);
    assert!(apply_order_update(&mut run, &second));
    assert!((run.net_exposure_usd - 10.0).abs() < 1e-9);
    assert_ne!(run.state, ExecutionRunState::Closed);
    second.state = LiveOrderState::Filled;
    second.filled_quantity = Some(0.6);
    for _ in 0..2 {
        apply_order_update(&mut run, &second);
        apply_order_update(&mut run, &first);
        assert_eq!(run.state, ExecutionRunState::Closed);
        assert_eq!(run.evidence.recovery_orders.len(), 2);
        assert_eq!(run.long_leg.filled_quantity, Some(1.0));
        assert_eq!(run.net_exposure_usd, 0.0);
    }
    second.filled_quantity = Some(0.7);
    apply_order_update(&mut run, &second);
    assert_ne!(run.state, ExecutionRunState::Closed);
    assert!(run.unwind_problem.is_some());
}
