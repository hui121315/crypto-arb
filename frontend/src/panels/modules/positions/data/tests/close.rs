//! 平仓 / 配对键 / close-run 状态映射的行为测试。

#[path = "close/action_scope.rs"]
mod action_scope;
#[path = "close/replay_scope.rs"]
mod replay_scope;
#[path = "close/retry.rs"]
mod retry;
#[path = "close/retry_scope.rs"]
mod retry_scope;

use super::super::*;
use super::support::*;
use crate::state::action_state::ActionState;
use shared_types::{
    ApiProblem, CloseLegStatus, CloseRunNextAction, CloseRunNextActionKind, CloseRunStatus,
    CloseRunUnwindPlan, CloseRunUnwindPlanStatus, ExecutionMode, LiveOrderState, MarginMode,
    OrderIntent, OrderRecord, OrderSide, OrderSource, OrderType, OrderUpdateSource, PositionSide,
    TimeInForce,
};

#[test]
fn pair_close_key_is_order_stable() {
    let left = paired_row(
        "binance",
        "MUUSDT",
        PositionSide::Long,
        Some("okx@MU-USDT-SWAP"),
    );
    let right = paired_row(
        "okx",
        "MU-USDT-SWAP",
        PositionSide::Short,
        Some("binance@MUUSDT"),
    );

    assert_eq!(pair_close_key(&left), pair_close_key(&right));
    assert_eq!(pair_close_key(&left).as_deref(), Some("pair:run-1"));
}

#[test]
fn legacy_pair_string_does_not_enable_pair_close() {
    let row = row(
        "binance",
        "MUUSDT",
        PositionSide::Long,
        Some("okx@MU-USDT-SWAP"),
    );

    assert!(pair_close_key(&row).is_none());
}

#[test]
fn missing_pair_problem_is_not_success_state() {
    let problem = missing_pair_problem(&row("binance", "MUUSDT", PositionSide::Long, None));

    assert_eq!(problem.code, "PAIR_NOT_FOUND");
    assert_eq!(problem.source.as_deref(), Some("positions"));
}

#[test]
fn close_run_message_includes_idempotency_evidence() {
    let mut run = close_run(CloseRunStatus::Submitted, 2, 0, 0.0);
    run.idempotency_key = Some("idem-1".to_owned());

    let state = close_run_action_state("配对平仓", &run);
    let message = state.message("");
    let label = state.label().unwrap_or_default();
    let evidence = state.evidence().cloned().unwrap_or_default();
    assert!(message.contains("action_run_id act-1"));
    assert!(message.contains("request_id req-1"));
    assert!(message.contains("idempotency idem-1"));
    assert!(!label.contains("act-1"));
    assert!(!label.contains("req-1"));
    assert!(!label.contains("idem-1"));
    assert_eq!(evidence.run_id.as_deref(), Some("close-test"));
    assert_eq!(evidence.action_run_id.as_deref(), Some("act-1"));
    assert_eq!(evidence.request_id.as_deref(), Some("req-1"));
    assert_eq!(evidence.idempotency_key.as_deref(), Some("idem-1"));
}

#[test]
fn close_run_failure_label_keeps_naked_exposure() {
    let run = close_run(CloseRunStatus::PartiallySubmitted, 1, 1, 1250.0);

    let label = close_run_failure_label("配对平仓", &run);

    assert!(label.contains("配对平仓未完全完成，裸露 $1250"));
    assert!(!label.contains("Action act-1"));
    assert!(!label.contains("Request req-1"));
}

#[test]
fn close_run_unwind_label_exposes_compensation_state() {
    let run = close_run(CloseRunStatus::UnwindRequired, 2, 1, 1250.0);

    let label = close_run_failure_label("配对平仓", &run);

    assert!(label.contains("配对平仓需补偿处理，裸露 $1250"));
}

#[test]
fn submitted_close_run_maps_to_accepted_action_state() {
    let run = close_run(CloseRunStatus::Submitted, 2, 0, 0.0);
    let state = close_run_action_state("全部平仓", &run);

    assert!(matches!(state, ActionState::Accepted { .. }));
    let label = state.label().unwrap_or_default();

    assert!(label.contains("全部平仓：partial close"));
    assert!(!label.contains("Action act-1"));
    assert!(!label.contains("Request req-1"));
}

#[test]
fn succeeded_close_run_maps_to_succeeded_action_state() {
    let run = close_run(CloseRunStatus::Succeeded, 2, 0, 0.0);
    let state = close_run_action_state("全部平仓", &run);

    assert!(matches!(state, ActionState::Succeeded { .. }));
}

#[test]
fn compensation_submitted_close_run_maps_to_accepted_action_state() {
    let run = close_run(CloseRunStatus::CompensationSubmitted, 2, 1, 1250.0);
    let state = close_run_action_state("配对平仓", &run);

    assert!(matches!(state, ActionState::Accepted { .. }));
}

#[test]
fn compensated_close_run_maps_to_succeeded_action_state() {
    let run = close_run(CloseRunStatus::Compensated, 2, 1, 0.0);
    let state = close_run_action_state("配对平仓", &run);

    assert!(matches!(state, ActionState::Succeeded { .. }));
}

#[test]
fn close_run_problem_falls_back_to_failed_code() {
    let run = close_run(CloseRunStatus::Failed, 0, 2, 0.0);
    let problem = close_run_problem(&run);

    assert_eq!(problem.code, shared_types::problem::codes::CLOSE_RUN_FAILED);
    assert_eq!(problem.source.as_deref(), Some("positions.close_run"));
    assert_eq!(problem.request_id.as_deref(), Some("req-1"));
}

#[test]
fn close_run_problem_keeps_existing_request_id() {
    let mut run = close_run(CloseRunStatus::Failed, 0, 2, 0.0);
    run.problem =
        Some(ApiProblem::new("UPSTREAM", "failed").with_request_id(Some("inner-req".into())));

    let problem = close_run_problem(&run);

    assert_eq!(problem.request_id.as_deref(), Some("inner-req"));
}

fn attach_manual_action(run: &mut shared_types::CloseRun) {
    if run.unwind_plan.is_none() {
        run.unwind_plan = Some(CloseRunUnwindPlan {
            status: CloseRunUnwindPlanStatus::CompensationFailed,
            filled_legs: Vec::new(),
            failed_legs: Vec::new(),
            compensation_candidates: Vec::new(),
            remaining_positions: Vec::new(),
            compensation_attempts: Vec::new(),
            manual_terminal_evidence: None,
            next_actions: Vec::new(),
            required_evidence: Vec::new(),
        });
    }
    if let Some(plan) = run.unwind_plan.as_mut() {
        plan.status = CloseRunUnwindPlanStatus::CompensationFailed;
        plan.next_actions = vec![CloseRunNextAction {
            kind: CloseRunNextActionKind::ManualIncidentReview,
            label: "人工复核事故".to_owned(),
            candidate_index: None,
            requires_confirmation: false,
            required_evidence: Vec::new(),
            reason: None,
        }];
    }
}

fn attach_compensation_attempt(run: &mut shared_types::CloseRun, order_id: &str) {
    if run.unwind_plan.is_none() {
        run.unwind_plan = Some(CloseRunUnwindPlan {
            status: CloseRunUnwindPlanStatus::CompensationSubmitted,
            filled_legs: Vec::new(),
            failed_legs: Vec::new(),
            compensation_candidates: Vec::new(),
            remaining_positions: Vec::new(),
            compensation_attempts: Vec::new(),
            manual_terminal_evidence: None,
            next_actions: Vec::new(),
            required_evidence: Vec::new(),
        });
    }
    if let Some(plan) = run.unwind_plan.as_mut() {
        plan.status = CloseRunUnwindPlanStatus::CompensationSubmitted;
        plan.compensation_attempts
            .push(shared_types::CloseRunCompensationAttempt {
                action_run_id: Some("act-comp".to_owned()),
                venue: "binance".to_owned(),
                symbol: "MUUSDT".to_owned(),
                side: PositionSide::Long,
                compensation_order_side: OrderSide::Buy,
                target_quantity: 1.0,
                status: CloseLegStatus::Accepted,
                order: Some(order_record(order_id, LiveOrderState::Accepted)),
                finality_source: None,
                confirmed_filled_at_ms: None,
                problem: None,
                cost_events: Vec::new(),
                submitted_at_ms: 1,
                updated_at_ms: 2,
            });
    }
}

fn order_record(order_id: &str, state: LiveOrderState) -> OrderRecord {
    OrderRecord {
        intent: OrderIntent {
            id: order_id.to_owned(),
            source: OrderSource::CloseRunCompensation,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "binance".to_owned(),
            symbol: "MUUSDT".to_owned(),
            side: OrderSide::Buy,
            order_type: OrderType::Market,
            quantity: 1.0,
            price: Some(100.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: format!("client-{order_id}"),
            client_order_id_policy: None,
            created_at_ms: 1,
        },
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: OrderUpdateSource::OrderQuery,
        exchange_order_id: Some(format!("ex-{order_id}")),
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 2,
    }
}
