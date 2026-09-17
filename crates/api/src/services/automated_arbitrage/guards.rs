use crate::state::AppState;
use shared_types::{
    AutomationRuntimeState, AutomationRuntimeStatus, ExecutionEnvironment, ExecutionRun,
    ExecutionRunLeg, ExecutionRunState, HedgeLegRole, ListStatus, PortfolioSnapshot, PositionRow,
    PositionSide,
};

const IN_FLIGHT_GRACE_MS: i64 = 120_000;
const MAX_PORTFOLIO_EVIDENCE_AGE_MS: i64 = 6_000;
pub(super) const EXIT_PROTECTION_REQUIRED: &str =
    "automatic entry requires take-profit, stop-loss, or liquidation protection";
pub(super) const TAKE_PROFIT_CAPITAL_MISMATCH: &str =
    "automatic take-profit amount exceeds 10% of automation capital";
pub(super) const STOP_LOSS_CAPITAL_MISMATCH: &str =
    "automatic stop-loss amount exceeds automation capital";

const MAX_TAKE_PROFIT_CAPITAL_RATIO: f64 = 0.10;
const MAX_STOP_LOSS_CAPITAL_RATIO: f64 = 1.0;

pub(super) fn active_runs(state: &AppState, now_ms: i64) -> Vec<ExecutionRun> {
    let portfolio = state.portfolio_snapshot().get_arc_now();
    let paired_run_ids = portfolio
        .iter()
        .flat_map(|entry| entry.value.positions.iter())
        .filter_map(|position| {
            position
                .pair_evidence
                .as_ref()
                .map(|evidence| evidence.run_id.clone())
        })
        .collect::<std::collections::BTreeSet<_>>();
    let portfolio_absence_proven = portfolio
        .as_ref()
        .is_some_and(|entry| fresh_complete_portfolio(&entry.value, now_ms));
    state
        .execution_runs()
        .iter()
        .filter(|entry| {
            active_state(
                entry.state,
                paired_run_ids.contains(&entry.run_id),
                portfolio.as_ref().is_some_and(|snapshot| {
                    run_has_matching_position(&snapshot.value.positions, entry.value())
                }),
                portfolio_absence_proven,
                entry.updated_at_ms,
                now_ms,
            )
        })
        .map(|entry| entry.value().clone())
        .collect()
}

pub(super) fn entry_blocker(
    state: &AppState,
    status: &AutomationRuntimeStatus,
    active_run_count: usize,
    now_ms: i64,
) -> Option<(AutomationRuntimeState, &'static str)> {
    entry_safety_blocker(state, status)
        .or_else(|| cooldown_blocker(status, now_ms))
        .or_else(|| {
            (active_run_count >= status.config.max_concurrent_runs).then_some((
                AutomationRuntimeState::Blocked,
                "maximum concurrent automated positions reached",
            ))
        })
}

pub(super) fn entry_safety_blocker(
    state: &AppState,
    status: &AutomationRuntimeStatus,
) -> Option<(AutomationRuntimeState, &'static str)> {
    if !status.config.enabled {
        return Some((AutomationRuntimeState::Disabled, "automation is disabled"));
    }
    if status.config.paused {
        return Some((AutomationRuntimeState::Paused, "automation is paused"));
    }
    let actual_environment = if state.trading_service().risk_config().live_trading_enabled {
        ExecutionEnvironment::Live
    } else {
        ExecutionEnvironment::Paper
    };
    if actual_environment != status.config.environment {
        return Some((
            AutomationRuntimeState::Blocked,
            "automation mode does not match the trading runtime environment",
        ));
    }
    if state.trading_service().risk_config().kill_switch_active {
        return Some((
            AutomationRuntimeState::Blocked,
            "trading kill switch blocks new automated entries",
        ));
    }
    if let Some(reason) = exit_protection_blocker(state, status.config.capital_usd) {
        return Some((AutomationRuntimeState::Blocked, reason));
    }
    None
}

pub(super) fn cooldown_blocker(
    status: &AutomationRuntimeStatus,
    now_ms: i64,
) -> Option<(AutomationRuntimeState, &'static str)> {
    if status
        .cooldown_until_ms
        .is_some_and(|deadline| now_ms < deadline)
    {
        return Some((
            AutomationRuntimeState::CoolingDown,
            "automation cooldown is active",
        ));
    }
    None
}

pub(super) fn exit_protection_blocker(
    state: &AppState,
    automation_capital_usd: f64,
) -> Option<&'static str> {
    let exit = state.trading_service().risk_config().auto_profit_close;
    if !exit.enabled && !exit.stop_loss_enabled && !exit.liquidation_guard_enabled {
        return Some(EXIT_PROTECTION_REQUIRED);
    }
    if exit.enabled
        && exit.min_net_profit_usd > automation_capital_usd * MAX_TAKE_PROFIT_CAPITAL_RATIO
    {
        return Some(TAKE_PROFIT_CAPITAL_MISMATCH);
    }
    if exit.stop_loss_enabled
        && exit.max_net_loss_usd > automation_capital_usd * MAX_STOP_LOSS_CAPITAL_RATIO
    {
        return Some(STOP_LOSS_CAPITAL_MISMATCH);
    }
    None
}

fn active_state(
    state: ExecutionRunState,
    paired_position: bool,
    matching_position: bool,
    portfolio_absence_proven: bool,
    updated_at_ms: i64,
    now_ms: i64,
) -> bool {
    match state {
        ExecutionRunState::SubmittingFirstLeg
        | ExecutionRunState::FirstLegPartial
        | ExecutionRunState::SubmittingSecondLeg
        | ExecutionRunState::SecondLegSubmitted => {
            now_ms.saturating_sub(updated_at_ms) <= IN_FLIGHT_GRACE_MS
        }
        ExecutionRunState::Hedged => {
            paired_position || matching_position || !portfolio_absence_proven
        }
        ExecutionRunState::UnwindRequired | ExecutionRunState::Unwinding => true,
        ExecutionRunState::Previewed
        | ExecutionRunState::RiskChecked
        | ExecutionRunState::FailedSafe
        | ExecutionRunState::Closed => false,
    }
}

fn fresh_complete_portfolio(snapshot: &PortfolioSnapshot, now_ms: i64) -> bool {
    !snapshot.degraded
        && snapshot.problems.is_empty()
        && snapshot.account_state.status == ListStatus::Fresh
        && snapshot.account_state.positions.status == ListStatus::Fresh
        && now_ms
            .checked_sub(snapshot.server_now_ms)
            .is_some_and(|age| (0..=MAX_PORTFOLIO_EVIDENCE_AGE_MS).contains(&age))
}

fn run_has_matching_position(positions: &[PositionRow], run: &ExecutionRun) -> bool {
    positions.iter().any(|position| {
        position_matches_leg(position, &run.long_leg)
            || position_matches_leg(position, &run.short_leg)
    })
}

fn position_matches_leg(position: &PositionRow, leg: &ExecutionRunLeg) -> bool {
    shared_types::venue_names_equal(&position.venue, &leg.exchange)
        && position_symbol_matches(&position.symbol, &leg.symbol)
        && position.side == leg_side(leg.role)
}

fn position_symbol_matches(left: &str, right: &str) -> bool {
    left.trim().eq_ignore_ascii_case(right.trim())
        || exchange::strip_common_suffixes(left)
            .eq_ignore_ascii_case(&exchange::strip_common_suffixes(right))
}

const fn leg_side(role: HedgeLegRole) -> PositionSide {
    match role {
        HedgeLegRole::Long => PositionSide::Long,
        HedgeLegRole::Short => PositionSide::Short,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_replayed_runs_do_not_consume_automation_concurrency() {
        assert!(!active_state(
            ExecutionRunState::Previewed,
            false,
            false,
            false,
            1,
            1_000_000
        ));
        assert!(!active_state(
            ExecutionRunState::SecondLegSubmitted,
            false,
            false,
            false,
            1,
            1_000_000
        ));
        assert!(active_state(
            ExecutionRunState::SecondLegSubmitted,
            false,
            false,
            false,
            990_000,
            1_000_000
        ));
    }

    #[test]
    fn hedged_run_requires_fresh_complete_absence_before_releasing_concurrency() {
        assert!(active_state(
            ExecutionRunState::Hedged,
            false,
            false,
            false,
            1,
            2
        ));
        assert!(!active_state(
            ExecutionRunState::Hedged,
            false,
            false,
            true,
            1,
            2
        ));
        assert!(active_state(
            ExecutionRunState::Hedged,
            true,
            false,
            true,
            1,
            2
        ));
        assert!(active_state(
            ExecutionRunState::Hedged,
            false,
            true,
            true,
            1,
            2
        ));
        assert!(active_state(
            ExecutionRunState::UnwindRequired,
            false,
            false,
            true,
            1,
            i64::MAX
        ));
    }
}
