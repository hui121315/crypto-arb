//! `CloseRun` 终态、排序和候选证据的行为测试。

use super::super::super::section_state::SectionData;
use super::super::derive::{
    close_candidate_evidence, close_candidate_title, close_run_next_action_detail, close_run_rows,
    close_run_status_detail, close_run_status_title, RECENT_CLOSE_RUN_TABLE_LIMIT,
};
use super::{candidate_fixture, close_run};
use shared_types::{CloseRunStatus, ExecutionLedgerQuality, OrderUpdateSource};

#[test]
fn close_run_rows_keep_only_actionable_runs_newest_first() {
    let mut finality = close_run("finality", CloseRunStatus::Submitted, 40);
    finality.finality_problem = Some(shared_types::ApiProblem::new(
        "HEDGE_ORDER_FINALITY_FAILED",
        "order query failed",
    ));
    let rows = SectionData::ready(vec![
        close_run("done", CloseRunStatus::Succeeded, 10),
        close_run("older", CloseRunStatus::UnwindRequired, 20),
        close_run("newer", CloseRunStatus::CompensationFailed, 30),
        finality,
    ]);

    let rows = close_run_rows(rows);

    assert_eq!(rows.value.len(), 3);
    assert_eq!(rows.value[0].id, "finality");
    assert_eq!(rows.value[1].id, "newer");
    assert_eq!(rows.value[2].id, "older");
}

#[test]
fn status_detail_surfaces_finality_problem_and_checked_time() {
    let mut run = close_run("close-1", CloseRunStatus::Submitted, 1);
    run.finality_checked_at_ms = Some(42);
    assert!(close_run_status_detail(&run).contains("终态回查 42ms"));

    run.finality_problem = Some(shared_types::ApiProblem::new(
        "HEDGE_ORDER_FINALITY_FAILED",
        "order query failed",
    ));

    assert!(close_run_status_detail(&run).contains("终态回查异常"));
    assert!(close_run_status_title(&run).contains("HEDGE_ORDER_FINALITY_FAILED"));
}

#[test]
fn close_run_rows_are_bounded_for_table_budget() {
    let rows = SectionData::ready(
        (0..RECENT_CLOSE_RUN_TABLE_LIMIT + 2)
            .map(|index| {
                close_run(
                    &format!("close-{index}"),
                    CloseRunStatus::UnwindRequired,
                    index as i64,
                )
            })
            .collect(),
    );

    let rows = close_run_rows(rows);

    assert_eq!(rows.value.len(), RECENT_CLOSE_RUN_TABLE_LIMIT);
    assert_eq!(rows.value[0].id, "close-9");
}

#[test]
fn next_action_detail_tracks_unwind_plan_state() {
    let mut run = close_run("close-1", CloseRunStatus::UnwindRequired, 1);

    assert!(close_run_next_action_detail(&run).contains("提交补买补偿单 #1"));
    assert!(close_run_next_action_detail(&run).contains("需确认"));
    assert!(close_run_next_action_detail(&run).contains("fresh_position_snapshot"));

    assert!(run.unwind_plan.is_some());
    if let Some(plan) = run.unwind_plan.as_mut() {
        plan.next_actions = vec![shared_types::CloseRunNextAction {
            kind: shared_types::CloseRunNextActionKind::CancelCompensationOrder,
            label: "撤销补买补偿单".to_owned(),
            candidate_index: Some(0),
            requires_confirmation: false,
            required_evidence: Vec::new(),
            reason: None,
        }];
    }
    assert_eq!(close_run_next_action_detail(&run), "撤销补买补偿单 #1");

    if let Some(plan) = run.unwind_plan.as_mut() {
        plan.next_actions = vec![shared_types::CloseRunNextAction {
            kind: shared_types::CloseRunNextActionKind::ManualIncidentReview,
            label: "人工复核事故".to_owned(),
            candidate_index: None,
            requires_confirmation: false,
            required_evidence: Vec::new(),
            reason: None,
        }];
    }
    assert_eq!(close_run_next_action_detail(&run), "人工复核事故");
}

#[test]
fn candidate_evidence_names_finality_source_or_unknown_gap() {
    let run = close_run("close-1", CloseRunStatus::UnwindRequired, 1);
    let mut candidate = run
        .unwind_plan
        .as_ref()
        .and_then(|plan| plan.compensation_candidates.first())
        .cloned()
        .unwrap_or_else(candidate_fixture);

    assert!(close_candidate_evidence(&candidate).contains("确认"));
    assert!(close_candidate_evidence(&candidate).contains("名义 实际 $100"));
    assert!(close_candidate_evidence(&candidate).contains("filled_quantity_x_filled_price"));

    candidate.finality_source = Some(OrderUpdateSource::PrivateWs);
    candidate.order_id = Some("order-1".to_owned());
    candidate.client_order_id = Some("client-1".to_owned());
    candidate.exchange_order_id = Some("exchange-1".to_owned());

    assert!(close_candidate_evidence(&candidate).contains("终态 私有WS"));
    assert!(close_candidate_title(&candidate).contains("order order-1"));
    assert!(close_candidate_title(&candidate).contains("client client-1"));
    assert!(close_candidate_title(&candidate).contains("exchange exchange-1"));

    candidate.notional_quality = ExecutionLedgerQuality::Missing;
    candidate.notional_source = "missing_notional".to_owned();
    candidate.notional_missing_fields = vec!["filled_price".to_owned()];

    assert!(close_candidate_evidence(&candidate).contains("名义 缺证据"));
    assert!(close_candidate_evidence(&candidate).contains("缺证据 filled_price"));
}
