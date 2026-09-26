use super::*;
use shared_types::{ExecutionRunEvidence, ExecutionRunLeg, HedgeLegRole, LiveOrderState};

#[test]
fn tampered_artifact_is_blocked_and_ready_requires_validation() {
    assert_eq!(
        artifact_flow_state(ExecutionArtifactStatus::Tampered, false),
        DeterministicFlowState::Blocked
    );
    assert_eq!(
        artifact_flow_state(ExecutionArtifactStatus::Ready, false),
        DeterministicFlowState::Current
    );
    assert_eq!(
        artifact_flow_state(ExecutionArtifactStatus::Ready, true),
        DeterministicFlowState::Complete
    );
}

#[test]
fn historical_run_does_not_complete_a_new_ticket_flow() {
    let selection = execution_selection("opp-a");
    let run = execution_run("run-old", "ticket-old", "opp-a");
    let artifact = LoadState::Ready(None);

    assert!(current_artifact_run(&selection, Some("ticket-new"), &artifact, Some(&run)).is_none());
}

#[test]
fn matching_ticket_keeps_the_current_run_visible() {
    let selection = execution_selection("opp-a");
    let run = execution_run("run-current", "ticket-current", "opp-a");
    let artifact = LoadState::Ready(None);

    assert_eq!(
        current_artifact_run(&selection, Some("ticket-current"), &artifact, Some(&run))
            .map(|run| run.run_id.as_str()),
        Some("run-current")
    );
}

#[test]
fn empty_selection_never_promotes_a_historical_run() {
    let selection = ExecutionSelection::empty();
    let run = execution_run("run-old", "ticket-old", "opp-old");
    let artifact = LoadState::Ready(None);

    assert!(current_artifact_run(&selection, None, &artifact, Some(&run)).is_none());
}

#[test]
fn flow_summary_prioritizes_blockers_over_progress() {
    let selection = execution_selection("opp-a");
    let stages = vec![
        DeterministicFlowStage::new("资格判定", "已通过", DeterministicFlowState::Complete),
        DeterministicFlowStage::new("复查计划", "数据依据缺失", DeterministicFlowState::Blocked),
    ];

    assert_eq!(summarize_flow(&selection, &stages).label, "复查计划");
}

fn execution_selection(opportunity_id: &str) -> ExecutionSelection {
    let mut selection = ExecutionSelection::empty();
    selection.opportunity_id = opportunity_id.to_owned();
    selection.execution_blockers.clear();
    selection
}

fn execution_run(run_id: &str, ticket_id: &str, opportunity_id: &str) -> ExecutionRun {
    ExecutionRun {
        run_id: run_id.to_owned(),
        ticket_id: ticket_id.to_owned(),
        opportunity_id: opportunity_id.to_owned(),
        state: ExecutionRunState::Closed,
        long_leg: execution_leg(HedgeLegRole::Long),
        short_leg: execution_leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: Some(1),
        evidence: ExecutionRunEvidence::default(),
        recovery_action: None,
        status_reason: "closed".into(),
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}

fn execution_leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".into(),
        symbol: "BTCUSDT".into(),
        order_ids: vec!["order-1".into()],
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: Some(1),
        state: LiveOrderState::Filled,
        target_quantity: 1.0,
        filled_quantity: Some(1.0),
        target_notional_usd: 10.0,
        filled_notional_usd: Some(10.0),
        filled_fee: Some(0.0),
    }
}
