use shared_types::{ExecutionRun, ExecutionRunState, LiveOrderState};

pub fn execution_result_event_id(run: &ExecutionRun) -> Option<String> {
    execution_result_alert_state(run)
        .map(|state| format!("execution-{}-{}", run.run_id, execution_state_token(state)))
}

pub const fn execution_result_alert_state(run: &ExecutionRun) -> Option<ExecutionRunState> {
    match run.state {
        ExecutionRunState::Hedged
            if matches!(run.long_leg.state, LiveOrderState::Filled)
                && matches!(run.short_leg.state, LiveOrderState::Filled) =>
        {
            Some(ExecutionRunState::Hedged)
        }
        ExecutionRunState::FailedSafe => Some(ExecutionRunState::FailedSafe),
        ExecutionRunState::Closed => Some(ExecutionRunState::Closed),
        _ => None,
    }
}

const fn execution_state_token(state: ExecutionRunState) -> &'static str {
    match state {
        ExecutionRunState::Hedged => "hedged",
        ExecutionRunState::FailedSafe => "failed-safe",
        ExecutionRunState::Closed => "closed",
        _ => "non-alerting",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{ExecutionRunLeg, HedgeLegRole};

    #[test]
    fn only_actionable_terminal_states_receive_stable_ids() {
        let mut run = execution_run(ExecutionRunState::Hedged);
        assert_eq!(
            execution_result_event_id(&run).as_deref(),
            Some("execution-run-1-hedged")
        );

        run.short_leg.state = LiveOrderState::Submitted;
        assert!(execution_result_event_id(&run).is_none());

        run.state = ExecutionRunState::FailedSafe;
        assert_eq!(
            execution_result_event_id(&run).as_deref(),
            Some("execution-run-1-failed-safe")
        );
    }

    fn execution_run(state: ExecutionRunState) -> ExecutionRun {
        ExecutionRun {
            run_id: "run-1".to_owned(),
            ticket_id: "ticket-1".to_owned(),
            opportunity_id: "opportunity-1".to_owned(),
            state,
            long_leg: leg(HedgeLegRole::Long),
            short_leg: leg(HedgeLegRole::Short),
            net_exposure_usd: 0.0,
            cost_reconciliation: None,
            valuation_problem: None,
            unwind_problem: None,
            finality_problem: None,
            finality_checked_at_ms: Some(1),
            evidence: Default::default(),
            recovery_action: None,
            status_reason: "fixture".to_owned(),
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }

    fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
        ExecutionRunLeg {
            role,
            exchange: "paper".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            order_ids: vec![format!("order-{role:?}")],
            identity: None,
            finality_source: None,
            confirmed_filled_at_ms: Some(1),
            state: LiveOrderState::Filled,
            target_quantity: 1.0,
            filled_quantity: Some(1.0),
            target_notional_usd: 10.0,
            filled_notional_usd: Some(10.0),
            filled_fee: Some(0.01),
        }
    }
}
