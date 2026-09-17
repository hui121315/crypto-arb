use super::*;

impl From<ExecutionRunState> for ExecutionRunPhase {
    fn from(state: ExecutionRunState) -> Self {
        match state {
            ExecutionRunState::Previewed => Self::Preview,
            ExecutionRunState::RiskChecked => Self::Confirming,
            ExecutionRunState::SubmittingFirstLeg | ExecutionRunState::SubmittingSecondLeg => {
                Self::Submitting
            }
            ExecutionRunState::FirstLegPartial | ExecutionRunState::SecondLegSubmitted => {
                Self::Working
            }
            ExecutionRunState::UnwindRequired | ExecutionRunState::Unwinding => Self::Closing,
            ExecutionRunState::Hedged | ExecutionRunState::Closed => Self::Settled,
            ExecutionRunState::FailedSafe => Self::Failed,
        }
    }
}

impl HedgeTicketView {
    /// Merge a durable run into the compact ticket view. Conflicting ticket or
    /// opportunity ids are rejected so a stale WS event cannot replace context.
    pub fn apply_execution_run(&mut self, run: &ExecutionRun) -> bool {
        if id_conflicts(&self.ticket_id, &run.ticket_id)
            || id_conflicts(&self.opportunity_id, &run.opportunity_id)
        {
            return false;
        }
        self.ticket_id = Some(run.ticket_id.clone());
        self.opportunity_id = Some(run.opportunity_id.clone());
        fill_leg_identity(&mut self.long_leg, &run.long_leg);
        fill_leg_identity(&mut self.short_leg, &run.short_leg);
        self.execution_run = Some(ExecutionRunView {
            key: ExecutionRunKey {
                ticket_id: Some(run.ticket_id.clone()),
                run_id: Some(run.run_id.clone()),
                order_id: primary_order_id(run),
            },
            phase: run.state.into(),
        });
        true
    }

    /// Recover the best compact view carried by a run, including legacy runs
    /// that predate embedded ticket health evidence.
    pub fn from_execution_run(run: &ExecutionRun) -> Self {
        let mut view = run.evidence.hedge_ticket_view.clone().unwrap_or_default();
        if !view.apply_execution_run(run) {
            view = Self::default();
            let _ = view.apply_execution_run(run);
        }
        view
    }
}

fn id_conflicts(current: &Option<String>, incoming: &str) -> bool {
    non_empty(current).is_some_and(|current| current != incoming)
}

fn fill_leg_identity(target: &mut Option<HedgeTicketLegView>, run: &crate::hedge::ExecutionRunLeg) {
    let leg = target.get_or_insert_with(|| HedgeTicketLegView {
        role: run.role,
        ..HedgeTicketLegView::default()
    });
    leg.role = run.role;
    if leg.venue.trim().is_empty() {
        leg.venue = run.exchange.clone();
    }
    if leg.symbol.trim().is_empty() {
        leg.symbol = run.symbol.clone();
    }
}

fn primary_order_id(run: &ExecutionRun) -> Option<String> {
    exchange_order_id(&run.long_leg)
        .or_else(|| exchange_order_id(&run.short_leg))
        .or_else(|| run.long_leg.order_ids.last().cloned())
        .or_else(|| run.short_leg.order_ids.last().cloned())
}

fn exchange_order_id(leg: &crate::hedge::ExecutionRunLeg) -> Option<String> {
    leg.identity
        .as_ref()
        .and_then(|identity| identity.exchange_order_id.clone())
        .filter(|value| !value.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LiveOrderState, RecoveryAction};

    #[test]
    fn execution_states_map_to_product_workflow_phases() {
        assert_eq!(
            ExecutionRunPhase::from(ExecutionRunState::SubmittingSecondLeg),
            ExecutionRunPhase::Submitting
        );
        assert_eq!(
            ExecutionRunPhase::from(ExecutionRunState::SecondLegSubmitted),
            ExecutionRunPhase::Working
        );
        assert_eq!(
            ExecutionRunPhase::from(ExecutionRunState::Unwinding),
            ExecutionRunPhase::Closing
        );
        assert_eq!(
            ExecutionRunPhase::from(ExecutionRunState::Hedged),
            ExecutionRunPhase::Settled
        );
    }

    #[test]
    fn run_projection_preserves_health_and_rejects_cross_ticket_updates() {
        let mut view = HedgeTicketView {
            ticket_id: Some("ticket-a".into()),
            opportunity_id: Some("opp-a".into()),
            long_leg: Some(HedgeTicketLegView {
                role: HedgeLegRole::Long,
                venue: "okx".into(),
                symbol: "BTC-USDT-SWAP".into(),
                market: WorkflowEvidenceHealth {
                    status: ResourceStatus::Ready,
                    ..WorkflowEvidenceHealth::default()
                },
                ..HedgeTicketLegView::default()
            }),
            ..HedgeTicketView::default()
        };
        let active_run = run("ticket-a", "opp-a", ExecutionRunState::SecondLegSubmitted);

        assert!(view.apply_execution_run(&active_run));
        assert_eq!(view.stable_key().as_deref(), Some("run-1"));
        assert_eq!(
            view.long_leg.as_ref().map(|leg| leg.market.status),
            Some(ResourceStatus::Ready)
        );
        assert!(!view.apply_execution_run(&run("ticket-b", "opp-a", ExecutionRunState::Hedged)));
    }

    fn run(ticket_id: &str, opportunity_id: &str, state: ExecutionRunState) -> ExecutionRun {
        ExecutionRun {
            run_id: "run-1".into(),
            ticket_id: ticket_id.into(),
            opportunity_id: opportunity_id.into(),
            state,
            long_leg: leg(HedgeLegRole::Long, "okx"),
            short_leg: leg(HedgeLegRole::Short, "binance"),
            net_exposure_usd: 0.0,
            cost_reconciliation: None,
            valuation_problem: None,
            unwind_problem: None,
            finality_problem: None,
            finality_checked_at_ms: None,
            evidence: Default::default(),
            recovery_action: None::<RecoveryAction>,
            status_reason: "working".into(),
            created_at_ms: 1,
            updated_at_ms: 2,
        }
    }

    fn leg(role: HedgeLegRole, exchange: &str) -> crate::hedge::ExecutionRunLeg {
        crate::hedge::ExecutionRunLeg {
            role,
            exchange: exchange.into(),
            symbol: "BTCUSDT".into(),
            order_ids: Vec::new(),
            identity: None,
            finality_source: None,
            confirmed_filled_at_ms: None,
            state: LiveOrderState::Created,
            target_quantity: 1.0,
            filled_quantity: None,
            target_notional_usd: 100.0,
            filled_notional_usd: None,
            filled_fee: None,
        }
    }
}
