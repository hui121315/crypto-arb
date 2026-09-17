use super::super::*;
use super::fixtures::*;
use super::*;

#[test]
fn sort_recent_orders_newest_first() {
    let mut rows = vec![run("old", 1), run("new", 3), run("mid", 2)];

    sort_recent(&mut rows);

    let ids: Vec<_> = rows.into_iter().map(|run| run.run_id).collect();
    assert_eq!(ids, ["new", "mid", "old"]);
}

#[test]
fn execution_run_query_filters_by_exact_context() {
    let query = ExecutionRunQuery {
        run_id: None,
        ticket_id: Some("ticket-a".into()),
        opportunity_id: Some("opp-a".into()),
    };
    let matching = ExecutionRun {
        ticket_id: "ticket-a".into(),
        opportunity_id: "opp-a".into(),
        ..run("matching", 3)
    };
    let wrong_ticket = ExecutionRun {
        ticket_id: "ticket-b".into(),
        opportunity_id: "opp-a".into(),
        ..run("wrong-ticket", 2)
    };
    let wrong_opportunity = ExecutionRun {
        ticket_id: "ticket-a".into(),
        opportunity_id: "opp-b".into(),
        ..run("wrong-opportunity", 1)
    };

    assert!(query.matches(&matching));
    assert!(!query.matches(&wrong_ticket));
    assert!(!query.matches(&wrong_opportunity));
}

#[test]
fn workflow_refresh_promotes_legacy_evidence_without_downgrading_future_versions() {
    let mut legacy = run("legacy", 1);
    legacy.evidence.schema_version = 1;
    legacy.evidence.hedge_ticket_view = None;

    refresh_workflow_view(&mut legacy);

    assert_eq!(
        legacy.evidence.schema_version,
        shared_types::EXECUTION_RUN_EVIDENCE_SCHEMA_VERSION
    );
    assert_eq!(
        legacy
            .evidence
            .hedge_ticket_view
            .as_ref()
            .and_then(|view| view.execution_run.as_ref())
            .and_then(|view| view.key.run_id.as_deref()),
        Some("legacy")
    );

    let future_version = shared_types::EXECUTION_RUN_EVIDENCE_SCHEMA_VERSION + 1;
    legacy.evidence.schema_version = future_version;
    refresh_workflow_view(&mut legacy);
    assert_eq!(legacy.evidence.schema_version, future_version);
}

#[test]
fn order_update_projects_fills_into_run() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.cost_reconciliation = Some(cost());
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");
    run.short_leg = leg_with_order(HedgeLegRole::Short, "short");

    assert!(apply_order_update(&mut run, &filled_record("long", 0.4)));
    assert_eq!(run.state, ExecutionRunState::SecondLegSubmitted);
    assert_eq!(run.long_leg.filled_quantity, Some(1.0));

    assert!(apply_order_update(&mut run, &filled_record("short", 0.6)));
    assert_eq!(run.state, ExecutionRunState::Hedged);
    assert_eq!(run.net_exposure_usd, 0.0);
    let cost = run.cost_reconciliation.ok_or("cost missing")?;
    assert_eq!(cost.filled_fee_usd, Some(1.0));
    assert_close_option(cost.actual_slippage_usd, 0.0)?;
    assert_close_option(cost.actual_open_cost_usd, 1.0)?;
    assert_eq!(cost.actual_cost_usd, None);
    assert_eq!(cost.cost_delta_usd, None);
    Ok(())
}

#[test]
fn order_update_records_exchange_order_identity() {
    let mut run = run("hedge", 1);
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");

    let mut record = filled_record("long", 0.0);
    record.identity = record.identity_snapshot();
    record.identity.record_venue_client_order_id("venue-long");

    assert!(apply_order_update(&mut run, &record));

    assert!(run.long_leg.order_ids.iter().any(|id| id == "long"));
    assert!(run.long_leg.order_ids.iter().any(|id| id == "client-long"));
    assert!(run.long_leg.order_ids.iter().any(|id| id == "venue-long"));
    assert!(run.long_leg.order_ids.iter().any(|id| id == "ex-long"));
    assert_eq!(
        run.long_leg
            .identity
            .as_ref()
            .map(|identity| identity.public_client_order_id.as_str()),
        Some("client-long")
    );
    assert_eq!(
        run.long_leg
            .identity
            .as_ref()
            .and_then(|identity| identity.venue_client_order_id.as_deref()),
        Some("venue-long")
    );
    assert_eq!(
        run.long_leg
            .identity
            .as_ref()
            .and_then(|identity| identity.exchange_order_id.as_deref()),
        Some("ex-long")
    );
    assert_eq!(
        run.long_leg.finality_source,
        Some(OrderUpdateSource::PrivateWs)
    );
    assert_eq!(run.long_leg.confirmed_filled_at_ms, None);
}

#[test]
fn order_update_matches_by_exchange_order_id() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");

    let mut record = filled_record("venue-normalized-long", 0.2);
    record.exchange_order_id = Some("ex-long".to_owned());

    assert!(apply_order_update(&mut run, &record));

    assert_eq!(run.long_leg.state, LiveOrderState::Filled);
    assert_eq!(run.long_leg.filled_fee, Some(0.2));
    assert!(run
        .long_leg
        .order_ids
        .iter()
        .any(|id| id == "venue-normalized-long"));
}

#[test]
fn order_update_marks_failed_hedge_for_recovery() {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "long");
    run.short_leg = leg_with_order(HedgeLegRole::Short, "short");

    apply_order_update(&mut run, &filled_record("long", 0.0));
    apply_order_update(&mut run, &failed_record("short"));

    assert_eq!(run.state, ExecutionRunState::UnwindRequired);
    assert_eq!(run.recovery_action, Some(RecoveryAction::UnwindLongLeg));
    assert_eq!(run.net_exposure_usd, 100.0);
}

#[test]
fn private_ws_fill_event_projects_into_run_leg_finality() -> Result<(), &'static str> {
    let mut run = run("hedge", 1);
    run.state = ExecutionRunState::SecondLegSubmitted;
    run.long_leg = leg_with_order(HedgeLegRole::Long, "ex-long");
    run.cost_reconciliation = Some(cost());
    let event = ledger_fill_event_row(&run, HedgeLegRole::Long, "ex-long", 0.4, 40.0, Some(0.02));
    let fill = ledger_fill_event(&event).ok_or("fill payload missing")?;

    assert!(apply_ledger_fill_update(&mut run, &event, fill));

    assert_eq!(run.long_leg.state, LiveOrderState::PartiallyFilled);
    assert_eq!(run.long_leg.filled_quantity, Some(0.4));
    assert_eq!(run.long_leg.filled_notional_usd, Some(40.0));
    assert_eq!(run.long_leg.filled_fee, Some(0.02));
    assert_eq!(
        run.long_leg.finality_source,
        Some(OrderUpdateSource::PrivateWs)
    );
    assert_eq!(run.long_leg.confirmed_filled_at_ms, Some(10));
    assert_eq!(run.net_exposure_usd, 40.0);
    Ok(())
}
