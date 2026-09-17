use crate::state::AppState;
use shared_types::{ExecutionEnvironment, ExecutionRun, ExecutionRunLeg};

pub(crate) fn active(state: &AppState) -> ExecutionEnvironment {
    if state.trading_service().risk_config().live_trading_enabled {
        ExecutionEnvironment::Live
    } else {
        ExecutionEnvironment::Paper
    }
}

pub(crate) fn run_matches(
    state: &AppState,
    run: &ExecutionRun,
    environment: ExecutionEnvironment,
) -> bool {
    run_matches_with(run, environment, |order_id| {
        state
            .trading_service()
            .get_order(order_id)
            .map(|order| order.intent.mode.environment())
    })
}

fn run_matches_with(
    run: &ExecutionRun,
    environment: ExecutionEnvironment,
    mut order_environment: impl FnMut(&str) -> Option<ExecutionEnvironment>,
) -> bool {
    leg_matches(&run.long_leg, environment, &mut order_environment)
        && leg_matches(&run.short_leg, environment, &mut order_environment)
}

fn leg_matches(
    leg: &ExecutionRunLeg,
    environment: ExecutionEnvironment,
    order_environment: &mut impl FnMut(&str) -> Option<ExecutionEnvironment>,
) -> bool {
    leg.order_ids
        .iter()
        .any(|order_id| order_environment(order_id) == Some(environment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionMode, ExecutionRunState, HedgeLegRole, LiveOrderState};

    #[test]
    fn both_legs_must_belong_to_the_active_environment() {
        let run = execution_run();
        let environment = |order_id: &str| match order_id {
            "long-internal" => Some(ExecutionEnvironment::Live),
            "short-internal" => Some(ExecutionEnvironment::Paper),
            _ => None,
        };

        assert!(!run_matches_with(
            &run,
            ExecutionEnvironment::Live,
            environment
        ));
    }

    #[test]
    fn dry_run_and_testnet_are_both_paper_environment_evidence() {
        let run = execution_run();
        let environment = |order_id: &str| match order_id {
            "long-internal" => Some(ExecutionMode::DryRun.environment()),
            "short-internal" => Some(ExecutionMode::Testnet.environment()),
            _ => None,
        };

        assert!(run_matches_with(
            &run,
            ExecutionEnvironment::Paper,
            environment
        ));
    }

    #[test]
    fn missing_order_records_never_infer_an_environment() {
        assert!(!run_matches_with(
            &execution_run(),
            ExecutionEnvironment::Live,
            |_| None
        ));
    }

    fn execution_run() -> ExecutionRun {
        ExecutionRun {
            run_id: "run-environment".into(),
            ticket_id: "ticket-environment".into(),
            opportunity_id: "opportunity-environment".into(),
            state: ExecutionRunState::Hedged,
            long_leg: execution_leg(HedgeLegRole::Long, ["long-internal", "long-alias"]),
            short_leg: execution_leg(HedgeLegRole::Short, ["short-internal", "short-alias"]),
            net_exposure_usd: 0.0,
            cost_reconciliation: None,
            valuation_problem: None,
            unwind_problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            evidence: Default::default(),
            recovery_action: None,
            status_reason: "hedged".into(),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn execution_leg(role: HedgeLegRole, order_ids: [&str; 2]) -> ExecutionRunLeg {
        ExecutionRunLeg {
            role,
            exchange: "venue".into(),
            symbol: "BTC".into(),
            order_ids: order_ids.into_iter().map(str::to_owned).collect(),
            identity: None,
            finality_source: None,
            confirmed_filled_at_ms: Some(1),
            state: LiveOrderState::Filled,
            target_quantity: 1.0,
            filled_quantity: Some(1.0),
            target_notional_usd: 100.0,
            filled_notional_usd: Some(100.0),
            filled_fee: Some(0.0),
        }
    }
}
