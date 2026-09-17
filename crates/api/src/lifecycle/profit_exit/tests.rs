use super::*;
use portfolio::ProfitExitTrigger;
use shared_types::PositionSide;

#[path = "tests/paper_e2e.rs"]
mod paper_e2e;

#[path = "tests/paper_e2e_support.rs"]
mod paper_e2e_support;

#[path = "tests/paper_fixture.rs"]
mod paper_fixture;

#[test]
fn requires_distinct_consecutive_snapshots() {
    let config = config(3, 60);
    let mut tracker = ProfitExitTracker::default();
    let first = candidate("run-1", 1_000, 10.0);
    let second = candidate("run-1", 3_000, 11.0);
    let third = candidate("run-1", 5_000, 12.0);

    assert!(tracker
        .ready_candidates(std::slice::from_ref(&first), &config, 1_000)
        .is_empty());
    assert!(tracker
        .ready_candidates(&[first], &config, 2_000)
        .is_empty());
    assert!(tracker
        .ready_candidates(&[second], &config, 3_000)
        .is_empty());
    assert_eq!(
        tracker
            .ready_candidates(&[third], &config, 5_000)
            .into_iter()
            .map(|row| row.run_id)
            .collect::<Vec<_>>(),
        vec!["run-1".to_owned()]
    );
}

#[test]
fn mark_only_snapshot_versions_do_not_multiply_one_account_sample() {
    let config = config(2, 60);
    let mut tracker = ProfitExitTracker::default();
    let first = candidate("run-1", 1_000, 10.0);
    let mut mark_only = first.clone();
    mark_only.snapshot_version = "mark-only-2".to_owned();
    if let Some(value) = mark_only.valuation.as_mut() {
        value.estimated_net_profit_usd = 11.0;
    }
    let next_account_sample = candidate("run-1", 3_000, 11.0);

    assert!(tracker
        .ready_candidates(&[first], &config, 1_000)
        .is_empty());
    assert!(tracker
        .ready_candidates(&[mark_only], &config, 2_000)
        .is_empty());
    assert_eq!(
        tracker
            .ready_candidates(&[next_account_sample], &config, 3_000)
            .len(),
        1
    );
}

#[test]
fn disappearance_and_long_gap_reset_confirmation() {
    let config = config(2, 60);
    let mut tracker = ProfitExitTracker::default();

    assert!(tracker
        .ready_candidates(&[candidate("run-1", 1_000, 10.0)], &config, 1_000)
        .is_empty());
    assert!(tracker.ready_candidates(&[], &config, 2_000).is_empty());
    assert!(tracker
        .ready_candidates(&[candidate("run-1", 3_000, 10.0)], &config, 3_000)
        .is_empty());
    assert!(tracker
        .ready_candidates(&[candidate("run-1", 10_000, 10.0)], &config, 10_000)
        .is_empty());
}

#[test]
fn trigger_change_restarts_confirmation_for_the_same_pair() {
    let config = config(2, 60);
    let mut tracker = ProfitExitTracker::default();
    let take_profit = candidate("run-1", 1_000, 10.0);
    let mut stop_loss = candidate("run-1", 3_000, -20.0);
    stop_loss.trigger = ProfitExitTrigger::StopLoss;
    let mut stop_loss_confirmed = stop_loss.clone();
    stop_loss_confirmed.observed_at_ms = 5_000;

    assert!(tracker
        .ready_candidates(&[take_profit], &config, 1_000)
        .is_empty());
    assert!(tracker
        .ready_candidates(&[stop_loss], &config, 3_000)
        .is_empty());
    assert!(
        tracker
            .ready_candidates(&[stop_loss_confirmed], &config, 5_000)
            .len()
            == 1
    );
}

#[test]
fn cooldown_is_scoped_to_the_attempted_execution_run() {
    let config = config(1, 60);
    let mut tracker = ProfitExitTracker::default();
    let first = candidate("run-1", 1_000, 12.0);
    let second = candidate("run-2", 2_000, 11.0);
    let same_run = candidate("run-1", 2_000, 11.0);

    let ready = tracker
        .ready_candidates(&[first], &config, 1_000)
        .into_iter()
        .next();
    let ready_run_id = ready
        .as_ref()
        .map(|candidate| candidate.run_id.as_str())
        .unwrap_or_default();
    assert_eq!(ready_run_id, "run-1");
    let Some(ready) = ready else {
        return;
    };
    tracker.record_attempt(&ready, &config, 1_000);

    assert!(tracker
        .ready_candidates(std::slice::from_ref(&same_run), &config, 2_000)
        .is_empty());
    assert_eq!(
        tracker
            .ready_candidates(std::slice::from_ref(&second), &config, 2_000)
            .into_iter()
            .map(|candidate| candidate.run_id)
            .collect::<Vec<_>>(),
        vec!["run-2".to_owned()]
    );
    assert!(tracker.ready_candidates(&[same_run], &config, 61_000).len() == 1);
}

#[test]
fn simultaneous_candidates_are_returned_in_risk_order() {
    let config = config(2, 60);
    let mut tracker = ProfitExitTracker::default();
    let first = [
        candidate("run-riskier", 1_000, -20.0),
        candidate("run-other", 1_000, 10.0),
    ];
    let second = [
        candidate("run-riskier", 3_000, -21.0),
        candidate("run-other", 3_000, 11.0),
    ];

    assert!(tracker.ready_candidates(&first, &config, 1_000).is_empty());
    assert_eq!(
        tracker
            .ready_candidates(&second, &config, 3_000)
            .into_iter()
            .map(|candidate| candidate.run_id)
            .collect::<Vec<_>>(),
        vec!["run-riskier".to_owned(), "run-other".to_owned()]
    );
}

#[test]
fn breached_liquidation_guard_does_not_wait_for_confirmation_samples() {
    let config = config(3, 60);
    let mut tracker = ProfitExitTracker::default();
    let mut breached = candidate("run-1", 1_000, -10.0);
    breached.trigger = ProfitExitTrigger::LiquidationGuard;
    breached.minimum_liquidation_distance_pct = Some(-0.1);

    assert!(tracker.ready_candidates(&[breached], &config, 1_000).len() == 1);
}

#[test]
fn approaching_liquidation_guard_keeps_confirmation_filter() {
    let config = config(2, 60);
    let mut tracker = ProfitExitTracker::default();
    let mut first = candidate("run-1", 1_000, -10.0);
    first.trigger = ProfitExitTrigger::LiquidationGuard;
    first.minimum_liquidation_distance_pct = Some(2.0);
    let mut second = first.clone();
    second.observed_at_ms = 3_000;

    assert!(tracker
        .ready_candidates(&[first], &config, 1_000)
        .is_empty());
    assert_eq!(tracker.ready_candidates(&[second], &config, 3_000).len(), 1);
}

fn config(confirmation_samples: u16, cooldown_secs: u64) -> AutoProfitCloseConfig {
    AutoProfitCloseConfig {
        enabled: true,
        confirmation_samples,
        cooldown_secs,
        ..Default::default()
    }
}

fn candidate(run_id: &str, observed_at_ms: i64, net_profit: f64) -> ProfitExitCandidate {
    ProfitExitCandidate {
        run_id: run_id.to_owned(),
        venue: "binance".to_owned(),
        symbol: "BTCUSDT".to_owned(),
        side: PositionSide::Short,
        snapshot_version: format!("pos-{observed_at_ms}"),
        observed_at_ms,
        valuation: Some(portfolio::ProfitExitValuation {
            matched_notional_usd: 10.0,
            gross_unrealized_pnl_usd: net_profit + 2.0,
            open_fee_usd: 1.0,
            funding_pnl_usd: 0.0,
            estimated_exit_cost_usd: 0.5,
            safety_buffer_usd: 0.5,
            estimated_net_profit_usd: net_profit,
            estimated_roi_bps: 100.0,
        }),
        trigger: ProfitExitTrigger::TakeProfit,
        minimum_liquidation_distance_pct: None,
        risk_venue: None,
    }
}
