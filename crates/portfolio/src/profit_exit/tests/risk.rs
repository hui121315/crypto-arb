use super::*;

#[test]
fn stop_loss_triggers_on_net_loss_or_loss_ratio() {
    let mut stop = config();
    stop.enabled = false;
    stop.stop_loss_enabled = true;
    stop.max_net_loss_usd = 10.0;
    stop.max_loss_roi_bps = 500.0;

    let candidates = profit_exit_candidates(
        &snapshot(1_000, -4.0, -4.0),
        &stop,
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(run(0.0)),
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].trigger, ProfitExitTrigger::StopLoss);
    assert!(candidates[0]
        .valuation
        .as_ref()
        .is_some_and(|value| value.estimated_net_profit_usd <= -10.0));
}

#[test]
fn live_liquidation_guard_requires_actual_evidence_on_the_triggering_leg() {
    let mut protection = config();
    protection.enabled = false;
    protection.liquidation_guard_enabled = true;
    protection.liquidation_exit_distance_pct = 8.0;
    let missing_evidence = with_liquidation_distances(snapshot(1_000, 0.0, 0.0), 7.0, 20.0, false);
    let mut one_actual = with_liquidation_distances(snapshot(1_000, 0.0, 0.0), 7.0, 20.0, false);
    one_actual.positions[1].liquidation_distance_pct = None;
    let one_actual = with_actual_liquidation_evidence(one_actual, &[0]);

    assert!(profit_exit_candidates(
        &missing_evidence,
        &protection,
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(run(0.0))
    )
    .is_empty());

    let candidates = profit_exit_candidates(
        &one_actual,
        &protection,
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(run(0.0)),
    );
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].trigger, ProfitExitTrigger::LiquidationGuard);
    assert_eq!(candidates[0].minimum_liquidation_distance_pct, Some(7.0));
    assert_eq!(candidates[0].risk_venue.as_deref(), Some("okx"));
}

#[test]
fn liquidation_guard_does_not_wait_for_profit_cost_reconciliation() {
    let mut protection = config();
    protection.enabled = false;
    protection.min_net_profit_usd = 0.0;
    protection.min_roi_bps = 0.0;
    protection.max_net_loss_usd = 0.0;
    protection.max_loss_roi_bps = 0.0;
    protection.liquidation_guard_enabled = true;
    protection.liquidation_exit_distance_pct = 8.0;
    let snapshot = with_liquidation_distances(snapshot(1_000, 0.0, 0.0), 7.0, 20.0, true);
    let mut incomplete_run = run(0.0);
    incomplete_run.cost_reconciliation = None;
    incomplete_run.valuation_problem = Some(shared_types::ApiProblem::new(
        "VALUATION_INCOMPLETE",
        "test valuation is unavailable",
    ));

    let candidates = profit_exit_candidates(
        &snapshot,
        &protection,
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(incomplete_run.clone()),
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].trigger, ProfitExitTrigger::LiquidationGuard);
    assert!(candidates[0].valuation.is_none());
    assert!(candidates[0].close_reason().contains("netUsd=unknown"));
}

#[test]
fn live_liquidation_guard_rejects_stale_field_evidence() {
    let mut protection = config();
    protection.enabled = false;
    protection.liquidation_guard_enabled = true;
    protection.liquidation_exit_distance_pct = 8.0;
    let mut snapshot = with_liquidation_distances(snapshot(10_000, 0.0, 0.0), 7.0, 20.0, true);
    for quality in &mut snapshot.account_state.field_quality {
        quality.observed_at_ms = Some(3_999);
    }

    assert!(profit_exit_candidates(
        &snapshot,
        &protection,
        shared_types::ExecutionEnvironment::Live,
        10_100,
        |_| Some(run(0.0)),
    )
    .is_empty());
}

#[test]
fn liquidation_guard_has_priority_over_take_profit() {
    let mut protection = config();
    protection.liquidation_guard_enabled = true;
    protection.liquidation_exit_distance_pct = 8.0;
    let snapshot = with_liquidation_distances(snapshot(1_000, 8.0, 7.0), 12.0, 6.0, true);

    let candidates = profit_exit_candidates(
        &snapshot,
        &protection,
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(run(0.0)),
    );

    assert_eq!(candidates[0].trigger, ProfitExitTrigger::LiquidationGuard);
    assert_eq!(candidates[0].risk_venue.as_deref(), Some("binance"));
}

#[test]
fn breached_liquidation_distance_is_not_discarded() {
    let mut protection = config();
    protection.enabled = false;
    protection.liquidation_guard_enabled = true;
    protection.liquidation_exit_distance_pct = 8.0;
    let snapshot = with_liquidation_distances(snapshot(1_000, 0.0, 0.0), -0.5, 20.0, true);

    let candidates = profit_exit_candidates(
        &snapshot,
        &protection,
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(run(0.0)),
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].trigger, ProfitExitTrigger::LiquidationGuard);
    assert_eq!(candidates[0].minimum_liquidation_distance_pct, Some(-0.5));
    assert_eq!(candidates[0].risk_venue.as_deref(), Some("okx"));
}
