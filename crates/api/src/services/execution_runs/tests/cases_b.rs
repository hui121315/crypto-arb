use super::super::*;
use super::fixtures::*;
use super::*;

#[test]
fn private_ws_incremental_fill_events_keep_latest_confirmed_fill_time() -> Result<(), &'static str>
{
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.cost_reconciliation = Some(cost());
    let first = ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 0.4, 40.0, Some(0.02));
    let mut second =
        ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 0.6, 60.0, Some(0.03));
    second.event_id = "fill_event:ex-long:second".to_owned();
    second.occurred_at_ms = 12;
    second.captured_at_ms = 13;

    assert!(apply_ledger_fill_update(
        &mut run,
        &first,
        ledger_fill_event(&first).ok_or("first fill payload missing")?
    ));
    assert_eq!(run.long_leg.state, LiveOrderState::PartiallyFilled);
    assert_eq!(run.long_leg.confirmed_filled_at_ms, Some(10));

    assert!(apply_ledger_fill_update(
        &mut run,
        &second,
        ledger_fill_event(&second).ok_or("second fill payload missing")?
    ));

    assert_eq!(run.long_leg.state, LiveOrderState::Filled);
    assert_eq!(run.long_leg.filled_quantity, Some(1.0));
    assert_eq!(run.long_leg.filled_notional_usd, Some(100.0));
    assert_eq!(run.long_leg.confirmed_filled_at_ms, Some(12));
    assert_close_option(run.long_leg.filled_fee, 0.05)
}

#[test]
fn ledger_fill_events_project_both_legs_to_hedged() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.short_leg = leg_with_order(HedgeLegRole::Short, "ex-short");
    run.cost_reconciliation = Some(cost());
    let long = ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 1.0, 100.0, Some(0.1));
    let short = ledger_fill_event_row(&run, HedgeLegRole::Short, "ex-short", 1.0, 100.0, Some(0.2));

    assert!(apply_ledger_fill_update(
        &mut run,
        &long,
        ledger_fill_event(&long).ok_or("long fill payload missing")?
    ));
    assert!(apply_ledger_fill_update(
        &mut run,
        &short,
        ledger_fill_event(&short).ok_or("short fill payload missing")?
    ));

    assert_eq!(run.state, ExecutionRunState::Hedged);
    assert_eq!(run.net_exposure_usd, 0.0);
    let cost = run.cost_reconciliation.ok_or("cost missing")?;
    assert_close_option(cost.filled_fee_usd, 0.3)?;
    assert_close_option(cost.actual_slippage_usd, 0.0)?;
    assert_close_option(cost.actual_open_cost_usd, 0.3)?;
    assert_eq!(cost.actual_cost_usd, None);
    assert_eq!(cost.cost_delta_usd, None);
    Ok(())
}

#[test]
fn ledger_fill_events_reconcile_open_cost_components() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.short_leg = leg_with_order(HedgeLegRole::Short, "ex-short");
    run.cost_reconciliation = Some(cost());
    let long = ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 1.0, 101.0, Some(0.1));
    let short = ledger_fill_event_row(&run, HedgeLegRole::Short, "ex-short", 1.0, 99.0, Some(0.2));

    assert!(apply_ledger_fill_update(
        &mut run,
        &long,
        ledger_fill_event(&long).ok_or("long fill payload missing")?
    ));
    assert!(apply_ledger_fill_update(
        &mut run,
        &short,
        ledger_fill_event(&short).ok_or("short fill payload missing")?
    ));

    let cost = run.cost_reconciliation.ok_or("cost missing")?;
    assert_close_option(cost.filled_fee_usd, 0.3)?;
    assert_close_option(cost.actual_slippage_usd, 2.0)?;
    assert_close_option(cost.actual_open_cost_usd, 2.3)?;
    assert_eq!(cost.actual_cost_usd, None);
    assert_eq!(cost.cost_delta_usd, None);
    Ok(())
}

#[test]
fn ledger_cancel_event_projects_terminal_failure_and_recovery() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.short_leg = leg_with_order(HedgeLegRole::Short, "ex-short");
    let long = ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 1.0, 100.0, Some(0.1));
    let short_cancel = ledger_state_event_row(
        &run,
        HedgeLegRole::Short,
        "ex-short",
        ExecutionLedgerEventType::Cancel,
        LiveOrderState::Cancelled,
    );

    assert!(apply_ledger_event_update(&mut run, &long));
    assert!(apply_ledger_event_update(&mut run, &short_cancel));

    assert_eq!(run.short_leg.state, LiveOrderState::Cancelled);
    assert_eq!(
        run.short_leg.finality_source,
        Some(OrderUpdateSource::OrderQuery)
    );
    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::UnwindLongLeg));
    assert_eq!(run.net_exposure_usd, 100.0);
}

#[test]
fn funding_payment_ledger_event_updates_run_cost_once() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.cost_reconciliation = Some(cost());
    let event = ledger_funding_event_row(&run, HedgeLegRole::Long, "ex-long", -0.12);

    assert!(apply_ledger_event_update(&mut run, &event));
    assert!(!apply_ledger_event_update(&mut run, &event));

    let cost = run.cost_reconciliation.ok_or("cost missing")?;
    assert_close_option(cost.actual_funding_usd, -0.12)?;
    assert_eq!(cost.funding_event_ids, [event.event_id]);
    assert_eq!(cost.actual_cost_usd, None);
    assert_eq!(cost.cost_delta_usd, None);
    Ok(())
}

#[test]
fn cross_venue_funding_events_project_each_linked_leg_once() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "hl-order");
    run.long_leg.exchange = "hyperliquid".to_owned();
    run.long_leg.symbol = "BTC-USDC".to_owned();
    run.short_leg = leg_with_order(HedgeLegRole::Short, "okx-order");
    run.short_leg.exchange = "okx".to_owned();
    run.short_leg.symbol = "BTC-USDT".to_owned();
    run.cost_reconciliation = Some(cost());
    let mut long = ledger_funding_event_row(&run, HedgeLegRole::Long, "hl-order", -0.12);
    long.order.exchange = "hyperliquid".to_owned();
    long.order.symbol = "BTC-USDC".to_owned();
    let mut short = ledger_funding_event_row(&run, HedgeLegRole::Short, "okx-order", 0.08);
    short.order.exchange = "okx".to_owned();
    short.order.symbol = "BTC-USDT".to_owned();

    assert!(apply_ledger_event_update(&mut run, &long));
    assert!(apply_ledger_event_update(&mut run, &short));
    assert!(!apply_ledger_event_update(&mut run, &long));

    let cost = run.cost_reconciliation.ok_or("cost missing")?;
    assert_close_option(cost.actual_funding_usd, -0.04)?;
    assert_eq!(cost.funding_event_ids, [long.event_id, short.event_id]);
    Ok(())
}

#[test]
fn funding_payment_for_other_run_is_ignored() -> Result<(), &'static str> {
    let mut current_run = run("hedge", 1);
    current_run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    current_run.cost_reconciliation = Some(cost());
    let mut other = run("other", 1);
    other.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    let event = ledger_funding_event_row(&other, HedgeLegRole::Long, "ex-long", -0.12);

    assert!(!apply_ledger_event_update(&mut current_run, &event));

    let cost = current_run.cost_reconciliation.ok_or("cost missing")?;
    assert_eq!(cost.actual_funding_usd, None);
    assert!(cost.funding_event_ids.is_empty());
    Ok(())
}

#[test]
fn estimated_funding_payment_is_ignored() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.cost_reconciliation = Some(cost());
    let mut event = ledger_funding_event_row(&run, HedgeLegRole::Long, "ex-long", -0.12);
    set_funding_quality(&mut event, ExecutionLedgerQuality::Estimated)?;

    assert!(!apply_ledger_event_update(&mut run, &event));

    assert_no_funding_cost(&run)
}

#[test]
fn non_usd_funding_payment_is_ignored() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.cost_reconciliation = Some(cost());
    let mut event = ledger_funding_event_row(&run, HedgeLegRole::Long, "ex-long", -0.12);
    set_funding_currency(&mut event, "BTC")?;

    assert!(!apply_ledger_event_update(&mut run, &event));

    assert_no_funding_cost(&run)
}

#[test]
fn funding_payment_wrong_symbol_is_ignored() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.cost_reconciliation = Some(cost());
    let mut event = ledger_funding_event_row(&run, HedgeLegRole::Long, "ex-long", -0.12);
    event.order.symbol = "ETH-USDT".to_owned();

    assert!(!apply_ledger_event_update(&mut run, &event));

    assert_no_funding_cost(&run)
}

#[test]
fn funding_payment_wrong_venue_is_ignored() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.cost_reconciliation = Some(cost());
    let mut event = ledger_funding_event_row(&run, HedgeLegRole::Long, "ex-long", -0.12);
    event.order.exchange = "other".to_owned();

    assert!(!apply_ledger_event_update(&mut run, &event));

    assert_no_funding_cost(&run)
}
