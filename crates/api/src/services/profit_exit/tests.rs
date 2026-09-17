use super::*;

mod fixtures;
use fixtures::*;

#[test]
fn idempotency_is_stable_per_fresh_exit_observation() {
    let candidate = test_stop_candidate(-0.05);
    assert_eq!(
        idempotency_key(&candidate),
        "auto-pair-exit:run-convergence:stop_loss:2000".to_owned()
    );
    assert_eq!(idempotency_key(&candidate), idempotency_key(&candidate));
}

#[test]
fn only_proven_zero_submission_or_compensated_close_runs_allow_retry() {
    for status in [CloseRunStatus::Failed, CloseRunStatus::Compensated] {
        assert!(!close_run_blocks_new_attempt(status));
    }
    for status in [
        CloseRunStatus::Submitted,
        CloseRunStatus::Succeeded,
        CloseRunStatus::PartiallySubmitted,
        CloseRunStatus::UnwindRequired,
        CloseRunStatus::CompensationSubmitted,
        CloseRunStatus::CompensationFailed,
        CloseRunStatus::ManuallyResolved,
    ] {
        assert!(close_run_blocks_new_attempt(status));
    }
}

#[test]
fn only_proven_failed_unlinked_action_allows_retry() {
    assert!(!unlinked_action_blocks_new_attempt(ActionRunStatus::Failed));
    assert!(unlinked_action_blocks_new_attempt(
        ActionRunStatus::Accepted
    ));
    assert!(unlinked_action_blocks_new_attempt(
        ActionRunStatus::Succeeded
    ));
}

#[tokio::test]
async fn unlinked_inflight_action_blocks_retry_but_compensated_link_allows_it() -> anyhow::Result<()>
{
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let action = action_runs::begin(
        &state,
        ActionRunStart {
            kind: ActionRunKind::PortfolioClosePair,
            actor: AUTO_ACTOR.to_owned(),
            target: Some("run-convergence".to_owned()),
            idempotency_key: Some("auto-exit-test".to_owned()),
            message: "automatic exit test".to_owned(),
        },
    )?;

    assert!(!automatic_exit_attempt_ready(&state, "run-convergence"));

    let close_run = test_close_run(CloseRunStatus::Compensated, Some(action.id));
    state.close_runs().insert(close_run.id.clone(), close_run);

    assert!(automatic_exit_attempt_ready(&state, "run-convergence"));
    Ok(())
}

#[tokio::test]
async fn submitted_close_run_keeps_new_exit_attempt_blocked() -> anyhow::Result<()> {
    let state = AppState::new(common::config::AppConfig::default()).await?;
    let close_run = test_close_run(CloseRunStatus::Submitted, None);
    state.close_runs().insert(close_run.id.clone(), close_run);

    assert!(!automatic_exit_attempt_ready(&state, "run-convergence"));
    Ok(())
}

#[test]
fn cost_recovery_hold_ignores_initial_cost_but_not_market_loss() -> serde_json::Result<()> {
    let config = AutoProfitCloseConfig {
        max_net_loss_usd: 0.25,
        max_loss_roi_bps: 1_000.0,
        ..AutoProfitCloseConfig::default()
    };
    let run = test_execution_run(1_000);
    let ticket = test_ticket(StrategyKind::PerpPriceSpread, 1)?;
    let mut candidate = test_stop_candidate(-0.05);

    assert!(!cost_recovery_exit_ready_for(
        &candidate, &config, &run, &ticket, 30_000,
    ));

    set_gross_unrealized_pnl(&mut candidate, -0.30);
    assert!(cost_recovery_exit_ready_for(
        &candidate, &config, &run, &ticket, 30_000,
    ));

    set_gross_unrealized_pnl(&mut candidate, -0.05);
    assert!(cost_recovery_exit_ready_for(
        &candidate, &config, &run, &ticket, 61_000,
    ));
    Ok(())
}

#[test]
fn recurring_cost_recovery_hold_uses_ticket_periods() -> serde_json::Result<()> {
    let config = AutoProfitCloseConfig {
        max_net_loss_usd: 1.0,
        max_loss_roi_bps: 1_000.0,
        ..AutoProfitCloseConfig::default()
    };
    let run = test_execution_run(1_000);
    let ticket = test_ticket(StrategyKind::PerpCross, 2)?;

    assert!(!cost_recovery_exit_ready_for(
        &test_stop_candidate(-0.05),
        &config,
        &run,
        &ticket,
        30_000,
    ));
    Ok(())
}

#[test]
fn cost_recovery_hold_never_delays_market_roi_loss() -> serde_json::Result<()> {
    let config = AutoProfitCloseConfig {
        max_net_loss_usd: 1.0,
        max_loss_roi_bps: 25.0,
        ..AutoProfitCloseConfig::default()
    };
    let run = test_execution_run(1_000);
    let ticket = test_ticket(StrategyKind::PerpCross, 2)?;
    let mut candidate = test_stop_candidate(-0.03);
    set_gross_unrealized_pnl(&mut candidate, -0.03);

    assert!(cost_recovery_exit_ready_for(
        &candidate, &config, &run, &ticket, 30_000,
    ));
    Ok(())
}

#[test]
fn instant_spread_stop_loss_is_not_delayed() -> serde_json::Result<()> {
    let config = AutoProfitCloseConfig::default();
    let run = test_execution_run(1_000);
    let ticket = test_ticket(StrategyKind::SpotCross, 1)?;

    assert!(cost_recovery_exit_ready_for(
        &test_stop_candidate(-0.05),
        &config,
        &run,
        &ticket,
        2_000,
    ));
    Ok(())
}

#[test]
fn cost_recovery_hold_never_delays_take_profit_or_liquidation_guard() -> serde_json::Result<()> {
    let config = AutoProfitCloseConfig::default();
    let run = test_execution_run(1_000);
    let ticket = test_ticket(StrategyKind::PerpPriceSpread, 1)?;
    for trigger in [
        ProfitExitTrigger::TakeProfit,
        ProfitExitTrigger::LiquidationGuard,
    ] {
        let mut candidate = test_stop_candidate(0.10);
        candidate.trigger = trigger;
        assert!(cost_recovery_exit_ready_for(
            &candidate, &config, &run, &ticket, 2_000,
        ));
    }
    Ok(())
}

fn set_gross_unrealized_pnl(candidate: &mut ProfitExitCandidate, value: f64) {
    assert!(
        candidate.valuation.is_some(),
        "stop fixture must include valuation"
    );
    if let Some(valuation) = candidate.valuation.as_mut() {
        valuation.gross_unrealized_pnl_usd = value;
    }
}
