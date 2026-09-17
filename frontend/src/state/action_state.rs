pub use shared_types::ActionState;
use shared_types::{
    ActionEvidence, ActionRun, ActionRunKind, ActionRunStatus, ApiProblem, ExecutionRun,
    ExecutionRunState, LiveOrderState, OrderRecord,
};

pub fn action_state_from_action_run(run: &ActionRun) -> ActionState {
    let evidence = ActionEvidence::from_action_run(run);
    match run.status {
        ActionRunStatus::Accepted => ActionState::accepted(run.message.clone()),
        ActionRunStatus::Succeeded => ActionState::succeeded(run.message.clone()),
        ActionRunStatus::Failed => ActionState::failed(
            run.message.clone(),
            run.problem.clone().unwrap_or_else(|| {
                ApiProblem::new("ACTION_RUN_FAILED", run.message.clone())
                    .with_request_id(run.request_id.clone())
                    .with_source("frontend.action_run_recovery")
            }),
        ),
    }
    .with_evidence(evidence)
}

pub fn latest_action_run<'a>(
    runs: &'a [ActionRun],
    kinds: &[ActionRunKind],
) -> Option<&'a ActionRun> {
    runs.iter()
        .filter(|run| kinds.contains(&run.kind))
        .max_by_key(|run| run.updated_at_ms)
}

pub fn action_state_from_execution_run(run: &ExecutionRun) -> Option<ActionState> {
    let evidence = ActionEvidence::from_execution_run(run);
    let state = match run.state {
        ExecutionRunState::Previewed => return None,
        ExecutionRunState::RiskChecked
        | ExecutionRunState::SubmittingFirstLeg
        | ExecutionRunState::FirstLegPartial
        | ExecutionRunState::SubmittingSecondLeg
        | ExecutionRunState::SecondLegSubmitted
        | ExecutionRunState::Unwinding => ActionState::accepted(run.status_reason.clone()),
        ExecutionRunState::Hedged | ExecutionRunState::Closed => {
            ActionState::succeeded(run.status_reason.clone())
        }
        ExecutionRunState::UnwindRequired | ExecutionRunState::FailedSafe => {
            ActionState::failed(run.status_reason.clone(), execution_run_problem(run))
        }
    };
    Some(state.with_evidence(evidence))
}

pub fn action_state_from_order_record(order: &OrderRecord) -> ActionState {
    let evidence = ActionEvidence::from_order_record(order);
    let label = order
        .message
        .clone()
        .unwrap_or_else(|| format!("订单状态 {:?}", order.state));
    let state = match order.state {
        LiveOrderState::Rejected | LiveOrderState::Failed => ActionState::failed(
            label.clone(),
            ApiProblem::new("ORDER_ACTION_FAILED", label)
                .with_source("frontend.order_snapshot_recovery"),
        ),
        LiveOrderState::Filled | LiveOrderState::Cancelled => ActionState::succeeded(label),
        _ => ActionState::accepted(label),
    };
    state.with_evidence(evidence)
}

pub fn merge_order_evidence(
    mut evidence: ActionEvidence,
    orders: &[OrderRecord],
) -> ActionEvidence {
    for order in orders {
        evidence.merge(ActionEvidence::from_order_record(order));
    }
    evidence
}

fn execution_run_problem(run: &ExecutionRun) -> ApiProblem {
    run.finality_problem
        .clone()
        .or_else(|| run.unwind_problem.clone())
        .or_else(|| run.valuation_problem.clone())
        .unwrap_or_else(|| {
            ApiProblem::new("EXECUTION_RUN_REQUIRES_RECOVERY", run.status_reason.clone())
                .with_request_id(run.evidence.request_id.clone())
                .with_source("frontend.execution_run_recovery")
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action_run(id: &str, kind: ActionRunKind, updated_at_ms: i64) -> ActionRun {
        ActionRun {
            id: id.into(),
            kind,
            status: ActionRunStatus::Succeeded,
            actor: "tester".into(),
            target: None,
            request_id: Some(format!("req-{id}")),
            idempotency_key: Some(format!("idem-{id}")),
            message: format!("done {id}"),
            problem: None,
            result: None,
            mutation: None,
            started_at_ms: updated_at_ms,
            updated_at_ms,
        }
    }

    #[test]
    fn action_run_recovery_keeps_structured_context() {
        let run = action_run("settings-1", ActionRunKind::VenueCredentialsUpdate, 2);
        let state = action_state_from_action_run(&run);
        let evidence = state.evidence().cloned().unwrap_or_default();

        assert!(matches!(state, ActionState::Succeeded { .. }));
        assert_eq!(evidence.action_run_id.as_deref(), Some("settings-1"));
        assert_eq!(evidence.request_id.as_deref(), Some("req-settings-1"));
        assert_eq!(evidence.idempotency_key.as_deref(), Some("idem-settings-1"));
    }

    #[test]
    fn latest_action_run_filters_kind_before_timestamp() {
        let rows = vec![
            action_run("risk-old", ActionRunKind::TradingRiskConfigUpdate, 2),
            action_run("other-new", ActionRunKind::TradingOrderCancel, 9),
            action_run("risk-new", ActionRunKind::TradingRiskConfigUpdate, 5),
        ];

        let latest = latest_action_run(&rows, &[ActionRunKind::TradingRiskConfigUpdate]);

        assert_eq!(latest.map(|run| run.id.as_str()), Some("risk-new"));
    }
}
