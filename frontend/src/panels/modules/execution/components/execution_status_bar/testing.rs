//! `execution_status_bar` 运行态/文案派生的单元测试与 ExecutionRun/腿夹具。

#[path = "testing/format_tests.rs"]
mod format_tests;

use super::state::{
    reason_text, reason_text_with_channel, run_requires_attention, run_stage, run_state_label,
    state_notice_class, state_notice_text, status_meta_text, status_meta_text_with_channel,
};
use super::visible_run_for_context;
use crate::api::ws::{WsChannelState, WsStatus};
use shared_types::{
    ApiProblem, ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole, LiveOrderState,
    RecoveryAction,
};

#[test]
fn stream_problem_label_is_not_rest_recovery() {
    let seed = ApiProblem::new("REST_SEED", "snapshot failed");
    let stream = ApiProblem::new("WS_READ_ERROR", "stream failed");

    assert_eq!(
        status_meta_text(None, Some(&seed), Some(&stream)),
        "执行流异常"
    );
    assert!(reason_text(None, Some(&seed), Some(&stream)).contains("执行流异常"));
}

#[test]
fn channel_problem_keeps_request_and_retry_context() {
    let mut channel = WsChannelState::new("execution");
    channel.status = WsStatus::Connected;
    channel.subscribed = true;
    channel.last_error = Some(
        ApiProblem::new("WS_SERVER_ERROR", "server refused")
            .with_request_id(Some("req-ws-1".into()))
            .with_retry_after_ms(Some(2_000)),
    );
    channel.retry_after_ms = Some(2_000);
    channel.problem_count = 1;

    let meta = status_meta_text_with_channel(None, None, None, Some(&channel));
    let reason = reason_text_with_channel(None, None, None, Some(&channel));

    assert!(meta.contains("WS异常"));
    assert!(meta.contains("request_id req-ws-1"));
    assert!(meta.contains("retry 2000ms"));
    assert!(meta.contains("帧 0 · 错误 1"));
    assert!(reason.contains("通道异常"));
    assert!(reason.contains("request_id req-ws-1"));
}

#[test]
fn channel_meta_surfaces_subscription_and_last_frame() {
    let mut channel = WsChannelState::new("execution");
    channel.status = WsStatus::Connected;

    assert_eq!(
        status_meta_text_with_channel(None, None, None, Some(&channel)),
        "execution 通道 · WS未订阅 · 帧 0 · 错误 0"
    );

    channel.subscribed = true;
    channel.last_message_at_ms = Some(1_800_000);
    channel.message_count = 3;

    let meta = status_meta_text_with_channel(
        Some(&run(ExecutionRunState::RiskChecked, None, 0.0)),
        None,
        None,
        Some(&channel),
    );

    assert!(meta.contains("裸露 $0"));
    assert!(meta.contains("execution 通道"));
    assert!(meta.contains("WS最后帧"));
    assert!(meta.contains("帧 3 · 错误 0"));
    assert!(meta.contains("1800000ms") || meta.contains(":"));
}

#[test]
fn run_stage_has_explicit_recovery_and_terminal_steps() {
    assert_eq!(run_stage(ExecutionRunState::Previewed), 1);
    assert_eq!(run_stage(ExecutionRunState::FirstLegPartial), 2);
    assert_eq!(run_stage(ExecutionRunState::SecondLegSubmitted), 3);
    assert_eq!(run_stage(ExecutionRunState::UnwindRequired), 4);
    assert_eq!(run_stage(ExecutionRunState::FailedSafe), 5);
    assert_eq!(run_stage(ExecutionRunState::Closed), 5);
}

#[test]
fn notice_surfaces_partial_unwind_failed_and_closed_states() {
    let partial = run(ExecutionRunState::FirstLegPartial, None, 120.0);
    let submitted = run(ExecutionRunState::SecondLegSubmitted, None, 0.0);
    let unwind = run(
        ExecutionRunState::UnwindRequired,
        Some(RecoveryAction::UnwindShortLeg),
        -80.0,
    );
    let failed = run(ExecutionRunState::FailedSafe, None, 42.0);
    let closed = run(ExecutionRunState::Closed, None, 0.0);

    assert!(state_notice_text(&partial)
        .is_some_and(|text| text.contains("部分成交") && text.contains("$120")));
    assert!(state_notice_text(&submitted).is_some_and(|text| text.contains("等待私有 WS")));
    assert!(state_notice_text(&unwind)
        .is_some_and(|text| text.contains("反向处理空腿") && text.contains("-$80")));
    assert_eq!(state_notice_class(&failed), "execution-state-notice danger");
    assert_eq!(state_notice_class(&closed), "execution-state-notice closed");
    assert_eq!(run_state_label(&closed), "执行已收口");
    assert!(state_notice_text(&closed).is_some_and(|text| text.contains("成交回报不完整")));
}

#[test]
fn hedged_label_requires_both_legs_filled() {
    let mut run = run(ExecutionRunState::Hedged, None, 0.0);

    assert_eq!(run_state_label(&run), "等待成交确认");
    assert!(state_notice_text(&run).is_some_and(|text| text.contains("未完整确认")));

    run.long_leg.state = LiveOrderState::Filled;
    run.short_leg.state = LiveOrderState::Filled;

    assert_eq!(run_state_label(&run), "双腿完成");
    assert_eq!(state_notice_text(&run), None);
}

#[test]
fn closed_execution_history_collapses_only_after_safe_terminal_evidence() {
    let mut closed = run(ExecutionRunState::Closed, None, 0.0);

    assert!(run_requires_attention(&closed));

    closed.long_leg.state = LiveOrderState::Filled;
    closed.short_leg.state = LiveOrderState::Filled;
    assert!(!run_requires_attention(&closed));
    assert!(state_notice_text(&closed).is_some_and(|text| text.contains("成交回报已确认")));

    closed.finality_problem = Some(ApiProblem::new(
        "HEDGE_ORDER_FINALITY_FAILED",
        "terminal evidence expired",
    ));
    assert!(run_requires_attention(&closed));

    let mut hedged = run(ExecutionRunState::Hedged, None, 0.0);
    hedged.long_leg.state = LiveOrderState::Filled;
    hedged.short_leg.state = LiveOrderState::Filled;
    assert!(run_requires_attention(&hedged));
}

#[test]
fn reason_prefers_run_unwind_problem_over_status_reason() {
    let mut run = run(
        ExecutionRunState::UnwindRequired,
        Some(RecoveryAction::ManualReview),
        88.0,
    );
    run.status_reason = "string fallback".into();
    run.unwind_problem = Some(ApiProblem::new(
        "HEDGE_UNWIND_SUBMIT_FAILED",
        "typed unwind failed",
    ));

    let reason = reason_text(Some(&run), None, None);

    assert!(reason.contains("补救异常"));
    assert!(reason.contains("typed unwind failed"));
    assert!(!reason.contains("string fallback"));
}

#[test]
fn state_notice_unwind_problem_keeps_typed_context() {
    let mut run = run(
        ExecutionRunState::UnwindRequired,
        Some(RecoveryAction::ManualReview),
        88.0,
    );
    run.unwind_problem = Some(
        ApiProblem::new("HEDGE_UNWIND_SUBMIT_FAILED", "typed unwind failed")
            .with_status(502)
            .with_request_id(Some("req-unwind-1".into()))
            .with_retry_after_ms(Some(2_500)),
    );

    let notice = state_notice_text(&run).unwrap_or_default();

    assert!(notice.contains("人工复核"));
    assert!(notice.contains("typed unwind failed"));
    assert!(notice.contains("code HEDGE_UNWIND_SUBMIT_FAILED"));
    assert!(notice.contains("HTTP 502"));
    assert!(notice.contains("request_id req-unwind-1"));
    assert!(notice.contains("retry 2500ms"));
}

#[test]
fn reason_labels_finality_problem_before_unwind_problem() {
    let mut run = run(
        ExecutionRunState::SecondLegSubmitted,
        Some(RecoveryAction::ManualReview),
        88.0,
    );
    run.finality_problem = Some(ApiProblem::new(
        "HEDGE_ORDER_FINALITY_FAILED",
        "order query failed",
    ));
    run.unwind_problem = Some(ApiProblem::new(
        "HEDGE_UNWIND_SUBMIT_FAILED",
        "unwind failed",
    ));

    let reason = reason_text(Some(&run), None, None);

    assert!(reason.contains("终态回查异常"));
    assert!(reason.contains("order query failed"));
    assert!(!reason.contains("补救异常"));
    assert_eq!(status_meta_text(Some(&run), None, None), "终态回查异常");
}

#[test]
fn status_meta_shows_finality_checked_time_without_problem() {
    let mut run = run(ExecutionRunState::SecondLegSubmitted, None, 0.0);
    run.finality_checked_at_ms = Some(1_800_000);

    let meta = status_meta_text(Some(&run), None, None);

    assert!(meta.contains("终态回查"));
    assert!(meta.contains("1800000ms") || meta.contains(":"));
}

#[test]
fn exposure_meta_marks_missing_fee_evidence_instead_of_zero() {
    let mut run = run(ExecutionRunState::FirstLegPartial, None, 42.0);
    run.long_leg.state = LiveOrderState::Filled;
    run.long_leg.filled_quantity = Some(1.0);
    run.long_leg.filled_notional_usd = Some(100.0);

    let meta = status_meta_text(Some(&run), None, None);

    assert!(meta.contains("裸露 $42"));
    assert!(meta.contains("成交费缺证据"));
    assert!(!meta.contains("成交费 $0"));
}

#[test]
fn exposure_meta_shows_fee_only_when_both_leg_fees_exist() {
    let mut run = run(ExecutionRunState::Hedged, None, 0.0);
    run.long_leg.filled_fee = Some(1.2);
    run.short_leg.filled_fee = Some(0.8);

    let meta = status_meta_text(Some(&run), None, None);

    assert_eq!(meta, "裸露 $0 · 成交费 $2");
}

#[test]
fn visible_run_uses_preview_ticket_or_current_workflow_run() {
    let current = run(ExecutionRunState::Closed, None, 0.0);

    assert!(visible_run_for_context(Some(current.clone()), Some("ticket-new"), None).is_none());
    assert!(visible_run_for_context(Some(current.clone()), Some("ticket-1"), None).is_some());
    assert!(
        visible_run_for_context(Some(current.clone()), Some("ticket-new"), Some("run-1")).is_some()
    );
    assert!(visible_run_for_context(Some(current), None, None).is_some());
}

fn run(
    state: ExecutionRunState,
    recovery_action: Option<RecoveryAction>,
    net_exposure_usd: f64,
) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
        net_exposure_usd,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action,
        status_reason: "test".into(),
        created_at_ms: 1,
        updated_at_ms: 2,
    }
}

fn leg(role: HedgeLegRole) -> ExecutionRunLeg {
    ExecutionRunLeg {
        role,
        exchange: "paper".into(),
        symbol: "BTC-USDT".into(),
        order_ids: Vec::new(),
        identity: None,
        finality_source: None,
        confirmed_filled_at_ms: None,
        state: LiveOrderState::Created,
        target_quantity: 0.0,
        filled_quantity: None,
        target_notional_usd: 0.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
