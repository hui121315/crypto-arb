//! `close_runs_panel` 纯派生的单元测试与 `CloseRun` / 补偿候选夹具。

#[path = "testing/finality.rs"]
mod finality;
#[path = "testing/pending.rs"]
mod pending;

use super::derive::{
    can_submit_compensation, can_submit_manual_terminal, cancellable_compensation_attempts,
    close_run_cost_detail, close_run_cost_label, close_run_cost_title,
    close_run_next_action_detail, close_run_remaining_positions_detail,
    compensation_cancel_order_id,
};
use shared_types::{
    CloseLegStatus, CloseRun, CloseRunCompensationAttempt, CloseRunCostReconciliation,
    CloseRunNextAction, CloseRunNextActionKind, CloseRunScope, CloseRunStatus,
    CloseRunUnwindLegEvidence, CloseRunUnwindPlan, CloseRunUnwindPlanStatus,
    ExecutionLedgerQuality, ExecutionMode, LiveOrderState, MarginMode, OrderIntent, OrderRecord,
    OrderSide, OrderSource, OrderType, OrderUpdateSource, PositionSide, TimeInForce,
};

#[test]
fn compensation_submit_requires_phrase_and_candidate() {
    let run = close_run("close-1", CloseRunStatus::UnwindRequired, 1);

    assert!(!can_submit_compensation(&run, ""));
    assert!(can_submit_compensation(
        &run,
        shared_types::CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE
    ));
}

#[test]
fn manual_terminal_requires_failed_manual_action_phrase_and_reason() {
    let mut run = close_run("close-1", CloseRunStatus::CompensationFailed, 1);
    if let Some(plan) = run.unwind_plan.as_mut() {
        plan.next_actions = vec![CloseRunNextAction {
            kind: CloseRunNextActionKind::ManualIncidentReview,
            label: "人工复核事故".to_owned(),
            candidate_index: None,
            requires_confirmation: false,
            required_evidence: Vec::new(),
            reason: None,
        }];
    }

    assert!(!can_submit_manual_terminal(&run, "", "account flat"));
    assert!(!can_submit_manual_terminal(
        &run,
        shared_types::CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE,
        ""
    ));
    assert!(can_submit_manual_terminal(
        &run,
        shared_types::CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE,
        "account flat"
    ));
}

#[test]
fn cancellable_attempts_use_compensation_order_identity() {
    let mut run = close_run("close-1", CloseRunStatus::CompensationSubmitted, 1);
    if let Some(plan) = run.unwind_plan.as_mut() {
        plan.status = CloseRunUnwindPlanStatus::CompensationSubmitted;
        plan.compensation_attempts = vec![
            compensation_attempt("order-live", CloseLegStatus::Accepted),
            compensation_attempt("order-done", CloseLegStatus::Cancelled),
        ];
    }

    let attempts = cancellable_compensation_attempts(&run);

    assert_eq!(attempts.len(), 1);
    assert_eq!(
        compensation_cancel_order_id(&attempts[0]).as_deref(),
        Some("order-live")
    );
}

#[test]
fn remaining_positions_detail_keeps_naked_exposure_visible() {
    let mut run = close_run("close-1", CloseRunStatus::UnwindRequired, 1);
    let mut remaining = candidate_fixture();
    remaining.status = shared_types::CloseLegStatus::Cancelled;
    remaining.notional_quality = ExecutionLedgerQuality::Estimated;
    remaining.notional_source = "mark_price".to_owned();
    remaining.notional_missing_fields = vec!["fresh_position_snapshot".to_owned()];
    assert!(run.unwind_plan.is_some());
    if let Some(plan) = run.unwind_plan.as_mut() {
        plan.remaining_positions = vec![remaining];
    }

    let detail = close_run_remaining_positions_detail(&run);

    assert!(detail.contains("binance MUUSDT 多"));
    assert!(detail.contains("名义 估算"));
    assert!(detail.contains("fresh_position_snapshot"));
}

#[test]
fn cost_helpers_keep_missing_evidence_visible() {
    assert_eq!(close_run_cost_label(None), "成本待证据");
    assert_eq!(close_run_cost_detail(None), "等待费用 / 滑点回放");
    assert!(close_run_cost_title(None).contains("未生成"));

    let summary = CloseRunCostReconciliation {
        missing_fields: vec!["close_fee".to_owned()],
        evidence_order_ids: vec!["order-1".to_owned()],
        close_slippage_usd: Some(3.0),
        ..CloseRunCostReconciliation::default()
    };

    assert_eq!(close_run_cost_label(Some(&summary)), "成本待证据");
    assert!(close_run_cost_detail(Some(&summary)).contains("平仓滑点 $3"));
    assert!(close_run_cost_title(Some(&summary)).contains("缺证据 close_fee"));
    assert!(close_run_cost_title(Some(&summary)).contains("orders order-1"));
}

#[test]
fn cost_helpers_show_verified_total_and_components() {
    let summary = CloseRunCostReconciliation {
        close_fee_usd: Some(2.0),
        close_slippage_usd: Some(3.0),
        compensation_fee_usd: Some(5.0),
        compensation_slippage_usd: Some(7.0),
        funding_usd: Some(-4.0),
        manual_handling_usd: Some(4.0),
        total_actual_cost_usd: Some(17.0),
        evidence_order_ids: vec!["close-order".to_owned(), "comp-order".to_owned()],
        evidence_event_ids: vec![
            "close-fee-1".to_owned(),
            "close-slip-1".to_owned(),
            "comp-fee-1".to_owned(),
            "comp-slip-1".to_owned(),
            "funding-1".to_owned(),
            "manual-1".to_owned(),
        ],
        close_fee_event_ids: vec!["close-fee-1".to_owned()],
        close_slippage_event_ids: vec!["close-slip-1".to_owned()],
        compensation_fee_event_ids: vec!["comp-fee-1".to_owned()],
        compensation_slippage_event_ids: vec!["comp-slip-1".to_owned()],
        funding_event_ids: vec!["funding-1".to_owned()],
        manual_handling_event_ids: vec!["manual-1".to_owned()],
        missing_fields: Vec::new(),
    };

    assert_eq!(close_run_cost_label(Some(&summary)), "总成本 $17");
    let detail = close_run_cost_detail(Some(&summary));
    assert!(detail.contains("平仓费 $2"));
    assert!(detail.contains("平仓滑点 $3"));
    assert!(detail.contains("补偿费 $5"));
    assert!(detail.contains("补偿滑点 $7"));
    assert!(detail.contains("资金费 -$4"));
    assert!(detail.contains("人工处理 $4"));
    let title = close_run_cost_title(Some(&summary));
    assert!(title.contains("orders close-order,comp-order"));
    assert!(
        title.contains("events close-fee-1,close-slip-1,comp-fee-1,comp-slip-1,funding-1,manual-1")
    );
    assert!(title.contains("平仓滑点事件 close-slip-1"));
    assert!(title.contains("补偿滑点事件 comp-slip-1"));
    assert!(title.contains("资金费事件 funding-1"));
    assert!(title.contains("人工处理事件 manual-1"));
}

fn close_run(id: &str, status: CloseRunStatus, updated_at_ms: i64) -> CloseRun {
    CloseRun {
        id: id.to_owned(),
        scope: CloseRunScope::Pair,
        status,
        action_run_id: None,
        request_id: None,
        idempotency_key: None,
        snapshot_version: "pos-1".to_owned(),
        expected_leg_count: 2,
        reason: Some("positions.close_pair".to_owned()),
        legs: Vec::new(),
        submitted_order_count: 1,
        failed_leg_count: 1,
        naked_exposure_usd: 12.0,
        message: "unwind required".to_owned(),
        problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        unwind_plan: Some(CloseRunUnwindPlan {
            status: CloseRunUnwindPlanStatus::BlockedPendingManualRecheck,
            filled_legs: Vec::new(),
            failed_legs: Vec::new(),
            compensation_candidates: vec![candidate_fixture()],
            remaining_positions: Vec::new(),
            compensation_attempts: Vec::new(),
            manual_terminal_evidence: None,
            next_actions: vec![CloseRunNextAction {
                kind: CloseRunNextActionKind::SubmitCompensationOrder,
                label: "提交补买补偿单".to_owned(),
                candidate_index: Some(0),
                requires_confirmation: true,
                required_evidence: vec!["fresh_position_snapshot".to_owned()],
                reason: Some("manual confirmation required".to_owned()),
            }],
            required_evidence: Vec::new(),
        }),
        cost_events: Vec::new(),
        cost_reconciliation: None,
        started_at_ms: 1,
        updated_at_ms,
    }
}

fn candidate_fixture() -> CloseRunUnwindLegEvidence {
    CloseRunUnwindLegEvidence {
        venue: "binance".to_owned(),
        symbol: "MUUSDT".to_owned(),
        side: PositionSide::Long,
        status: shared_types::CloseLegStatus::Filled,
        target_quantity: 1.0,
        confirmed_quantity: Some(1.0),
        confirmed_price: Some(100.0),
        mark_price: 100.0,
        notional_usd: 100.0,
        notional_quality: ExecutionLedgerQuality::Actual,
        notional_source: "filled_quantity_x_filled_price".to_owned(),
        notional_missing_fields: Vec::new(),
        compensation_order_side: Some(OrderSide::Buy),
        order_id: None,
        client_order_id: None,
        exchange_order_id: None,
        finality_source: None,
        confirmed_filled_at_ms: Some(2),
        problem: None,
    }
}

fn compensation_attempt(order_id: &str, status: CloseLegStatus) -> CloseRunCompensationAttempt {
    CloseRunCompensationAttempt {
        action_run_id: Some("act-comp".to_owned()),
        venue: "binance".to_owned(),
        symbol: "MUUSDT".to_owned(),
        side: PositionSide::Long,
        compensation_order_side: OrderSide::Buy,
        target_quantity: 1.0,
        status,
        order: Some(order_record(order_id, LiveOrderState::Accepted)),
        finality_source: None,
        confirmed_filled_at_ms: None,
        problem: None,
        cost_events: Vec::new(),
        submitted_at_ms: 2,
        updated_at_ms: 3,
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
