use shared_types::{
    ApiProblem, ExecutionRun, ExecutionRunLeg, ExecutionRunState, HedgeLegRole, LiveOrderState,
    OrderUpdateSource,
};

use crate::api::ws::{WsChannelState, WsStatus};

use super::labels::{
    active_problem, active_state_label, active_state_tone, channel_meta_label, channel_problem,
    channel_state_tone, empty_orders_label, order_state_tone, state_label, update_source_label,
};

#[test]
fn active_problem_prefers_stream_over_seed() {
    let problem = active_problem(
        Some(ApiProblem::new("REST_SEED", "snapshot failed")),
        Some(ApiProblem::new("WS_READ_ERROR", "stream failed")),
        None,
    );

    assert_eq!(problem.map(|p| p.code).as_deref(), Some("WS_READ_ERROR"));
}

#[test]
fn active_problem_includes_channel_error_after_stream_before_seed() {
    let mut state = WsChannelState::new("orders");
    let mut problem = ApiProblem::new("WS_READ_ERROR", "closed");
    problem.request_id = Some("req-1".to_owned());
    problem.retry_after_ms = Some(5_000);
    state.last_error = Some(problem.clone());
    state.retry_after_ms = problem.retry_after_ms;
    state.problem_count = 1;
    state.last_problem_at_ms = Some(42);

    let active = active_problem(
        Some(ApiProblem::new("REST_SEED", "snapshot failed")),
        None,
        channel_problem(&state),
    );

    assert_eq!(
        active.as_ref().map(|problem| problem.code.as_str()),
        Some("WS_READ_ERROR")
    );
    assert!(channel_meta_label(&state).contains("request_id req-1"));
    assert!(channel_meta_label(&state).contains("retry 5000ms"));
    assert!(channel_meta_label(&state).contains("帧 0 · 错误 1"));
}

#[test]
fn channel_meta_distinguishes_disconnected_and_subscribed_empty() {
    let mut state = WsChannelState::new("orders");

    assert_eq!(channel_meta_label(&state), "WS未连接 · 帧 0 · 错误 0");

    state.status = WsStatus::Connected;
    state.subscribed = true;

    assert_eq!(
        channel_meta_label(&state),
        "WS已订阅，等待首帧 · 帧 0 · 错误 0"
    );
}

#[test]
fn active_state_prefers_problem_then_run() {
    let problem = ApiProblem::new("WS_READ_ERROR", "stream failed");
    let run = run(ExecutionRunState::UnwindRequired);

    assert_eq!(
        active_state_label(Some(&run), &[], Some(&problem)),
        "数据异常"
    );
    assert_eq!(active_state_label(Some(&run), &[], None), "需要反向处理");
    assert_eq!(active_state_label(None, &[], None), "暂无运行");
    assert_eq!(active_state_tone(Some(&run), &[], None), "error");
    assert_eq!(active_state_tone(None, &[], Some(&problem)), "error");
    assert_eq!(active_state_tone(None, &[], None), "idle");
}

#[test]
fn queue_tones_distinguish_transport_and_order_lifecycle() {
    let mut channel = WsChannelState::new("orders");
    assert_eq!(channel_state_tone(&channel), "idle");

    channel.status = WsStatus::Connected;
    assert_eq!(channel_state_tone(&channel), "pending");
    channel.subscribed = true;
    assert_eq!(channel_state_tone(&channel), "ready");
    channel.last_error = Some(ApiProblem::new("WS_READ_ERROR", "closed"));
    assert_eq!(channel_state_tone(&channel), "error");

    assert_eq!(order_state_tone(LiveOrderState::Accepted), "pending");
    assert_eq!(order_state_tone(LiveOrderState::PartiallyFilled), "warning");
    assert_eq!(order_state_tone(LiveOrderState::Filled), "ready");
    assert_eq!(order_state_tone(LiveOrderState::Rejected), "error");
}

#[test]
fn submitted_and_accepted_wait_for_fill_confirmation() {
    assert_eq!(state_label(LiveOrderState::Submitted), "等待成交确认");
    assert_eq!(state_label(LiveOrderState::Accepted), "等待成交确认");
    assert_eq!(
        active_state_label(Some(&run(ExecutionRunState::SecondLegSubmitted)), &[], None),
        "第二腿已提交，等待成交确认"
    );
}

#[test]
fn hedged_run_requires_both_legs_filled_before_complete_label() {
    let mut run = run(ExecutionRunState::Hedged);

    assert_eq!(active_state_label(Some(&run), &[], None), "等待成交确认");

    run.long_leg.state = LiveOrderState::Filled;
    run.short_leg.state = LiveOrderState::Filled;

    assert_eq!(active_state_label(Some(&run), &[], None), "双腿完成");
}

#[test]
fn closed_run_names_risk_closure_without_claiming_fill_success() {
    assert_eq!(
        active_state_label(Some(&run(ExecutionRunState::Closed)), &[], None),
        "执行已收口"
    );
}

#[test]
fn empty_orders_label_distinguishes_current_run_scope() {
    assert_eq!(empty_orders_label(true, true), "当前执行暂无订单");
    assert_eq!(empty_orders_label(true, false), "上一笔执行没有订单记录");
    assert_eq!(empty_orders_label(false, false), "暂无真实订单");
}

#[test]
fn update_source_label_names_finality_evidence() {
    assert_eq!(update_source_label(OrderUpdateSource::PrivateWs), "私有WS");
    assert_eq!(update_source_label(OrderUpdateSource::OrderQuery), "查询");
    assert_eq!(update_source_label(OrderUpdateSource::Reconcile), "回查");
}

fn run(state: ExecutionRunState) -> ExecutionRun {
    ExecutionRun {
        run_id: "run-1".into(),
        ticket_id: "ticket-1".into(),
        opportunity_id: "opp-1".into(),
        state,
        long_leg: leg(HedgeLegRole::Long),
        short_leg: leg(HedgeLegRole::Short),
        net_exposure_usd: 0.0,
        cost_reconciliation: None,
        valuation_problem: None,
        unwind_problem: None,
        finality_problem: None,
        finality_checked_at_ms: None,
        evidence: Default::default(),
        recovery_action: None,
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
        state: LiveOrderState::Accepted,
        target_quantity: 0.0,
        filled_quantity: None,
        target_notional_usd: 0.0,
        filled_notional_usd: None,
        filled_fee: None,
    }
}
