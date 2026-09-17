use super::*;
use shared_types::ListStatus;

#[path = "tests/support.rs"]
mod support;
use support::*;

#[path = "tests/risk.rs"]
mod risk;

#[test]
fn returns_cost_adjusted_profitable_pair() {
    let snapshot = snapshot(1_000, 8.0, 7.0);
    let run = run(-2.0);

    let candidates = profit_exit_candidates(
        &snapshot,
        &config(),
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |id| (id == "run-1").then(|| run.clone()),
    );

    assert_eq!(candidates.len(), 1);
    let candidate = &candidates[0];
    let valuation = candidate
        .valuation
        .as_ref()
        .expect("take-profit must include complete valuation");
    assert_eq!(candidate.run_id, "run-1");
    assert!((valuation.gross_unrealized_pnl_usd - 15.0).abs() < 1e-9);
    assert!((valuation.open_fee_usd - 1.0).abs() < 1e-9);
    assert!((valuation.funding_pnl_usd + 2.0).abs() < 1e-9);
    assert!((valuation.estimated_exit_cost_usd - 3.0).abs() < 1e-9);
    assert!((valuation.safety_buffer_usd - 0.1).abs() < 1e-9);
    assert!((valuation.estimated_net_profit_usd - 8.9).abs() < 1e-9);
    assert!(valuation.estimated_roi_bps > 400.0);
    assert_eq!(candidate.trigger, ProfitExitTrigger::TakeProfit);
}

#[test]
fn rejects_gross_profit_that_does_not_cover_costs() {
    let snapshot = snapshot(1_000, 2.0, 2.0);

    let candidates = profit_exit_candidates(
        &snapshot,
        &config(),
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(run(0.0)),
    );

    assert!(candidates.is_empty());
}

#[test]
fn unrelated_degradation_does_not_block_a_fresh_pair() {
    let mut degraded = snapshot(1_000, 8.0, 7.0);
    degraded.degraded = true;
    degraded.account_state.status = ListStatus::Degraded;

    let candidates = profit_exit_candidates(
        &degraded,
        &config(),
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(run(0.0)),
    );

    assert_eq!(candidates.len(), 1);
}

#[test]
fn rejects_stale_or_future_snapshot() {
    let stale = snapshot(1_000, 8.0, 7.0);
    let future = snapshot(2_000, 8.0, 7.0);

    assert!(profit_exit_candidates(
        &stale,
        &config(),
        shared_types::ExecutionEnvironment::Live,
        8_000,
        |_| Some(run(0.0))
    )
    .is_empty());
    assert!(profit_exit_candidates(
        &future,
        &config(),
        shared_types::ExecutionEnvironment::Live,
        1_000,
        |_| Some(run(0.0))
    )
    .is_empty());
}

#[test]
fn degraded_position_fanout_accepts_only_fresh_pair_rows() {
    let fresh = with_position_health(snapshot(10_000, 8.0, 7.0), 9_500);
    let stale = with_position_health(snapshot(10_000, 8.0, 7.0), 3_999);

    assert_eq!(
        profit_exit_candidates(
            &fresh,
            &config(),
            shared_types::ExecutionEnvironment::Live,
            10_100,
            |_| Some(run(0.0)),
        )
        .len(),
        1
    );
    assert!(profit_exit_candidates(
        &stale,
        &config(),
        shared_types::ExecutionEnvironment::Live,
        10_100,
        |_| Some(run(0.0)),
    )
    .is_empty());
}

#[test]
fn live_candidate_uses_the_older_bilateral_account_sample_time() {
    let fresh = with_position_health(snapshot(10_000, 8.0, 7.0), 9_500);

    let candidates = profit_exit_candidates(
        &fresh,
        &config(),
        shared_types::ExecutionEnvironment::Live,
        10_100,
        |_| Some(run(0.0)),
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].observed_at_ms, 9_500);
}

#[test]
fn paper_pair_ignores_unconfigured_account_degradation() {
    let mut paper = snapshot(1_000, 8.0, 7.0);
    paper.degraded = true;
    paper.account_state.status = ListStatus::Degraded;
    paper.account_state.positions.status = ListStatus::Degraded;

    let candidates = profit_exit_candidates(
        &paper,
        &config(),
        shared_types::ExecutionEnvironment::Paper,
        1_100,
        |_| Some(run(0.0)),
    );

    assert_eq!(candidates.len(), 1);
}

#[test]
fn rejects_missing_reciprocal_pair_or_run_cost() {
    let mut one_sided = snapshot(1_000, 8.0, 7.0);
    one_sided.positions[1].pair_evidence = None;
    let mut missing_cost = run(0.0);
    missing_cost.cost_reconciliation = None;

    assert!(profit_exit_candidates(
        &one_sided,
        &config(),
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(run(0.0))
    )
    .is_empty());
    assert!(profit_exit_candidates(
        &snapshot(1_000, 8.0, 7.0),
        &config(),
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| { Some(missing_cost.clone()) }
    )
    .is_empty());
}

#[test]
fn rejects_missing_actual_open_cost() {
    let mut missing_actual_cost = run(0.0);
    let Some(cost) = missing_actual_cost.cost_reconciliation.as_mut() else {
        panic!("test fixture must include cost reconciliation");
    };
    cost.actual_open_cost_usd = None;

    let candidates = profit_exit_candidates(
        &snapshot(1_000, 8.0, 7.0),
        &config(),
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(missing_actual_cost.clone()),
    );

    assert!(candidates.is_empty());
}

#[test]
fn opening_slippage_is_not_subtracted_twice() {
    let mut completed = run(0.0);
    let cost = completed
        .cost_reconciliation
        .as_mut()
        .expect("test fixture must include cost reconciliation");
    cost.actual_slippage_usd = Some(2.0);
    cost.actual_open_cost_usd = Some(3.0);

    let candidates = profit_exit_candidates(
        &snapshot(1_000, 8.0, 7.0),
        &config(),
        shared_types::ExecutionEnvironment::Live,
        1_100,
        |_| Some(completed.clone()),
    );

    let valuation = candidates[0]
        .valuation
        .as_ref()
        .expect("take-profit must include complete valuation");
    assert!((valuation.open_fee_usd - 1.0).abs() < 1e-9);
    assert!((valuation.estimated_net_profit_usd - 10.9).abs() < 1e-9);
}
