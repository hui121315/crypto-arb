use super::super::components::SectionData;
use super::*;
use crate::state::action_state::ActionState;
use shared_types::{
    AccountStateSnapshot, ActionEvidence, ApiProblem, ListStatus, PositionSide,
    VenuePositionEnvelope,
};

#[path = "tests/execution_projection.rs"]
mod execution_projection;
#[path = "tests/support.rs"]
mod support;
use support::{portfolio_snapshot, snapshot_with_balance_status, snapshot_with_position_status};

#[test]
fn position_action_message_keeps_typed_problem_in_evidence() -> Result<(), &'static str> {
    let state = ActionState::failed(
        "平仓失败",
        ApiProblem::new("RATE_LIMITED", "rate limited")
            .with_status(429)
            .with_request_id(Some("req-close-1".into()))
            .with_retry_after_ms(Some(2_000))
            .with_source("trade-rest"),
    );

    let message = position_action_message(&state);

    assert_eq!(message, "平仓失败：rate limited");
    let evidence = position_action_evidence(&state).ok_or("problem evidence")?;
    assert!(evidence.contains("code RATE_LIMITED"));
    assert!(evidence.contains("source trade-rest"));
    assert!(evidence.contains("HTTP 429"));
    assert!(evidence.contains("request_id req-close-1"));
    assert!(evidence.contains("retry 2000ms"));
    Ok(())
}

#[test]
fn position_action_message_translates_known_risk_blockers() -> Result<(), &'static str> {
    let state = ActionState::failed(
        "上一笔平仓未完成，裸露 $452",
        ApiProblem::new(
            shared_types::problem::codes::RISK_BLOCKED,
            "risk blocked order: [ProtectedPosition, ExchangeNotAllowed]",
        ),
    );

    let message = position_action_message(&state);

    assert!(message.contains("该仓位受保护"));
    assert!(message.contains("当前交易所不在允许平仓范围"));
    assert!(!message.contains("ProtectedPosition"));
    assert!(position_action_evidence(&state)
        .ok_or("risk evidence")?
        .contains("code RISK_BLOCKED"));
    Ok(())
}

#[test]
fn position_action_message_keeps_machine_evidence_out_of_primary_copy() -> Result<(), &'static str>
{
    let state = ActionState::succeeded("平仓已完成：2 条订单已确认成交").with_evidence(
        ActionEvidence::client_request("req-close-1", Some("idem-close-1".into()))
            .with_run_id(Some("run-close-1".into())),
    );

    assert_eq!(
        position_action_message(&state),
        "平仓已完成：2 条订单已确认成交"
    );
    let evidence = position_action_evidence(&state).ok_or("close evidence")?;
    assert!(evidence.contains("request_id req-close-1"));
    assert!(evidence.contains("idempotency idem-close-1"));
    assert!(evidence.contains("run_id run-close-1"));
    Ok(())
}

#[test]
fn completed_previous_close_is_history_not_current_position_status() {
    let recovered = ActionState::succeeded("上一笔平仓：平仓已完成：2 条订单已确认成交");
    let current = ActionState::succeeded("平仓：平仓已完成：2 条订单已确认成交");
    let unresolved = ActionState::accepted("上一笔平仓：订单已提交，等待终态");

    assert_eq!(
        completed_previous_close_summary(&recovered),
        Some("平仓已完成：2 条订单已确认成交")
    );
    assert!(completed_previous_close_summary(&current).is_none());
    assert!(completed_previous_close_summary(&unresolved).is_none());
}

#[test]
fn snapshot_section_error_keeps_problem_visible() {
    let problem = ApiProblem::new("PORTFOLIO_DOWN", "portfolio failed");
    let section: SectionData<Vec<u8>> = snapshot_section(&LoadState::Error(problem), |_| vec![1]);

    assert_eq!(
        section.status.empty_text("暂无", "读取中", "读取失败"),
        "读取失败：portfolio failed"
    );
}

#[test]
fn current_partial_snapshot_is_not_labeled_as_old_data() {
    let state = LoadState::Stale {
        value: portfolio_snapshot(AccountStateSnapshot::default(), Vec::new()),
        problem: ApiProblem::new(
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_DEGRADED,
            "current snapshot contains partial field evidence",
        ),
    };

    let section = snapshot_section(&state, |snapshot| snapshot.summary.total_nav_usd);

    assert!(section.has_fresh_value());
    assert!(section.status.stale_note("显示上次快照").is_none());
    assert!(actionable_snapshot_problem(&state).is_none());
}

#[test]
fn transport_stale_snapshot_keeps_actionable_problem() {
    let state = LoadState::Stale {
        value: portfolio_snapshot(AccountStateSnapshot::default(), Vec::new()),
        problem: ApiProblem::new(
            shared_types::problem::codes::PORTFOLIO_SNAPSHOT_STALE,
            "portfolio transport is stale",
        ),
    };

    assert_eq!(
        actionable_snapshot_problem(&state).map(|problem| problem.code),
        Some(shared_types::problem::codes::PORTFOLIO_SNAPSHOT_STALE.to_owned())
    );
}

#[test]
fn close_runs_surface_hides_all_empty_states() {
    assert!(!should_render_close_runs_surface(
        &SectionData::<Vec<u8>>::loading()
    ));
    assert!(!should_render_close_runs_surface(
        &SectionData::<Vec<u8>>::error(&ApiProblem::new("PORTFOLIO_DOWN", "portfolio failed"))
    ));
    assert!(!should_render_close_runs_surface(&SectionData::stale(
        Vec::<u8>::new(),
        &ApiProblem::new("TIMEOUT", "slow")
    )));
}

#[test]
fn close_runs_surface_hides_ready_empty_state() {
    assert!(!should_render_close_runs_surface(
        &SectionData::<Vec<u8>>::ready(Vec::new())
    ));
    assert!(should_render_close_runs_surface(&SectionData::ready(vec![
        1_u8
    ])));
}

#[test]
fn fresh_balance_envelope_is_not_staled_by_position_problem() {
    let snapshot = snapshot_with_balance_status(ListStatus::Fresh, Vec::new());
    let state = LoadState::Stale {
        value: snapshot,
        problem: ApiProblem::new("POSITION_READ_DEGRADED", "gate positions failed"),
    };

    let section = balance_snapshot_section(&state);

    assert!(section.has_fresh_value());
    assert_eq!(section.value.len(), 1);
    assert!(section.status.stale_note("余额过期").is_none());
}

#[test]
fn degraded_balance_envelope_keeps_its_own_problem() {
    let problem = ApiProblem::new("BALANCE_READ_DEGRADED", "okx balance failed");
    let snapshot = snapshot_with_balance_status(ListStatus::Degraded, vec![problem]);

    let section = balance_snapshot_section(&LoadState::Ready(snapshot));

    assert!(!section.has_fresh_value());
    assert_eq!(
        section.status.stale_note("余额过期").as_deref(),
        Some("余额过期：okx balance failed")
    );
}

#[test]
fn fresh_position_envelope_is_not_staled_by_unrelated_snapshot_problem() {
    let snapshot = snapshot_with_position_status(ListStatus::Fresh, Vec::new());
    let state = LoadState::Stale {
        value: snapshot,
        problem: ApiProblem::new("BALANCE_READ_DEGRADED", "okx balance failed"),
    };

    let section = position_snapshot_section(&state);

    assert!(section.has_fresh_value());
    assert_eq!(section.value.len(), 1);
    assert!(section.status.stale_note("持仓过期").is_none());
}

#[test]
fn degraded_position_envelope_keeps_request_context() {
    let problem = ApiProblem::new("RATE_LIMITED", "gate positions rate limited")
        .with_status(429)
        .with_request_id(Some("req-gate-positions-429".into()))
        .with_retry_after_ms(Some(30_000))
        .with_source("account_position_runtime");
    let snapshot = snapshot_with_position_status(ListStatus::Degraded, vec![problem]);

    let section = position_snapshot_section(&LoadState::Ready(snapshot));

    assert!(!section.has_fresh_value());
    assert_eq!(section.value.len(), 1);
    assert_eq!(
        section.status.stale_note("持仓过期").as_deref(),
        Some(
            "持仓过期：gate positions rate limited · HTTP 429 · request_id req-gate-positions-429 · retry 30000ms"
        )
    );
}

#[test]
fn degraded_position_fields_keep_current_rows_ready() {
    let snapshot = snapshot_with_position_status(ListStatus::Degraded, Vec::new());

    let section = position_snapshot_section(&LoadState::Ready(snapshot));

    assert!(section.has_fresh_value());
    assert_eq!(section.value.len(), 1);
    assert!(section.status.stale_note("持仓过期").is_none());
}

#[test]
fn completed_execution_history_keeps_empty_position_surface_ready() {
    let envelope = VenuePositionEnvelope::new(
        Vec::new(),
        ListStatus::Degraded,
        "account_position_runtime",
        42,
        vec![ApiProblem::new(
            "POSITION_EVIDENCE_MISSING",
            "private position rows are unavailable",
        )],
        Vec::new(),
    );

    let section = position_envelope_section(&[], &envelope, true);

    assert!(section.has_fresh_value());
    assert!(section.value.is_empty());
}
