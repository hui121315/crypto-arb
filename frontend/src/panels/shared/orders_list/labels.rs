//! 订单行文案：状态/方向/模式/来源标签与 run 状态推导、problem 文案、活动行汇总。
//! 名义金额计算见 `notional.rs`，组件装配见 `orders_list.rs`。

use shared_types::{
    ApiProblem, ExecutionMode, ExecutionRun, ExecutionRunState, LiveOrderState, OrderRecord,
    OrderSide, OrderUpdateSource,
};
use wasm_bindgen::JsValue;

use crate::api::ws::{WsChannelState, WsStatus};
use crate::panels::modules::timestamp::local_date_hm;
use crate::panels::shared::{execution_mode_label, ws_channel_activity_label};

pub(super) fn active_state_label(
    run: Option<&ExecutionRun>,
    orders: &[OrderRecord],
    problem: Option<&ApiProblem>,
) -> String {
    if problem.is_some() {
        return "数据异常".into();
    }
    if let Some(run) = run {
        return execution_run_state_label(run).into();
    }
    latest_order_state(orders)
        .map(state_label)
        .unwrap_or("暂无运行")
        .to_owned()
}

pub(super) fn active_state_tone(
    run: Option<&ExecutionRun>,
    orders: &[OrderRecord],
    problem: Option<&ApiProblem>,
) -> &'static str {
    if problem.is_some() {
        return "error";
    }
    if let Some(run) = run {
        return execution_run_tone(run);
    }
    latest_order_state(orders)
        .map(order_state_tone)
        .unwrap_or("idle")
}

fn latest_order_state(orders: &[OrderRecord]) -> Option<LiveOrderState> {
    orders
        .iter()
        .max_by_key(|order| order.updated_at_ms)
        .map(|order| order.state)
}

pub(super) fn active_problem(
    seed_problem: Option<ApiProblem>,
    stream_problem: Option<ApiProblem>,
    channel_problem: Option<ApiProblem>,
) -> Option<ApiProblem> {
    stream_problem.or(channel_problem).or(seed_problem)
}

pub(super) fn order_count_label(count: usize, problem: Option<&ApiProblem>) -> String {
    if problem.is_some() {
        format!("{count} 单 · 数据异常")
    } else {
        format!("{count} 单")
    }
}

pub(super) fn order_feed_label(has_run: bool, run_is_current: bool) -> &'static str {
    match (has_run, run_is_current) {
        (true, true) => "当前执行订单",
        (true, false) => "上一笔订单",
        (false, _) => "最近订单",
    }
}

pub(super) fn empty_orders_label(has_run: bool, run_is_current: bool) -> &'static str {
    if has_run && run_is_current {
        "当前执行暂无订单"
    } else if has_run {
        "上一笔执行没有订单记录"
    } else {
        "暂无真实订单"
    }
}

pub(super) fn problem_text(problem: &ApiProblem) -> String {
    let mut parts = vec![problem.message.clone(), format!("code {}", problem.code)];
    if let Some(source) = &problem.source {
        parts.push(format!("source {source}"));
    }
    if let Some(status) = problem.status {
        parts.push(format!("HTTP {status}"));
    }
    if let Some(request_id) = &problem.request_id {
        parts.push(format!("request_id {request_id}"));
    }
    if let Some(retry_after_ms) = problem.retry_after_ms {
        parts.push(format!("retry {retry_after_ms}ms"));
    }
    parts.join(" · ")
}

pub(super) fn quantity_label(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "待确认".into())
}

pub(super) fn channel_problem(state: &WsChannelState) -> Option<ApiProblem> {
    state.last_error.clone()
}

pub(super) fn channel_meta_label(state: &WsChannelState) -> String {
    let status = if let Some(problem) = state.last_error.as_ref() {
        format!("WS异常 {}", problem_text(problem))
    } else if !state.subscribed {
        match state.status {
            WsStatus::Connecting => "WS连接中".to_owned(),
            WsStatus::Connected => "WS未订阅".to_owned(),
            WsStatus::Disconnected => "WS未连接".to_owned(),
        }
    } else {
        state
            .last_message_at_ms
            .map(|last_at| format!("WS最后帧 {}", time_label(u64_ms_to_i64(last_at))))
            .unwrap_or_else(|| "WS已订阅，等待首帧".to_owned())
    };
    format!("{status} · {}", ws_channel_activity_label(state))
}

pub(super) fn channel_state_tone(state: &WsChannelState) -> &'static str {
    if state.last_error.is_some() {
        return "error";
    }
    match (state.status, state.subscribed) {
        (WsStatus::Connected, true) => "ready",
        (WsStatus::Connecting, _) | (WsStatus::Connected, false) => "pending",
        (WsStatus::Disconnected, _) => "idle",
    }
}

fn u64_ms_to_i64(value: u64) -> i64 {
    if value > i64::MAX as u64 {
        i64::MAX
    } else {
        value as i64
    }
}

pub(super) fn order_label(order: &OrderRecord) -> String {
    format!(
        "{} · {} · {} · {} · {}",
        order.intent.exchange,
        order.intent.symbol,
        side_label(order.intent.side),
        mode_label(order.intent.mode),
        update_source_label(order.last_update_source)
    )
}

pub(super) fn order_primary_label(order: &OrderRecord) -> String {
    format!("{} · {}", order.intent.exchange, order.intent.symbol)
}

pub(super) fn order_detail_label(order: &OrderRecord) -> String {
    let base = format!(
        "{} · {}",
        side_label(order.intent.side),
        update_source_label(order.last_update_source)
    );
    order
        .message
        .as_deref()
        .map(str::trim)
        .filter(|message| !message.is_empty())
        .map_or(base.clone(), |message| format!("{base} · {message}"))
}

pub(super) fn run_scope_label(run: Option<&ExecutionRun>) -> String {
    run.map(|run| {
        let long_symbol = run.long_leg.symbol.to_ascii_uppercase();
        let short_symbol = run.short_leg.symbol.to_ascii_uppercase();
        let market = if long_symbol == short_symbol {
            long_symbol
        } else {
            format!("{long_symbol}/{short_symbol}")
        };
        format!(
            "{market} · {} 多 ↔ {} 空 · {}",
            run.long_leg.exchange.to_ascii_uppercase(),
            run.short_leg.exchange.to_ascii_uppercase(),
            history_time_label(run.updated_at_ms),
        )
    })
    .unwrap_or_else(|| "尚未创建 ExecutionRun".to_owned())
}

pub(super) fn run_identity_label(run: &ExecutionRun) -> String {
    format!("run {} · ticket {}", run.run_id, run.ticket_id)
}

pub(super) fn time_label(ms: i64) -> String {
    if ms <= 0 {
        return "--:--".into();
    }
    let date = js_sys::Date::new(&JsValue::from_f64(ms as f64));
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

pub(super) fn history_time_label(ms: i64) -> String {
    if ms <= 0 {
        return "日期时间未知".into();
    }
    local_date_hm(ms).unwrap_or_else(|| "日期时间未知".into())
}

pub(super) fn state_label(state: LiveOrderState) -> &'static str {
    match state {
        LiveOrderState::Created => "创建",
        LiveOrderState::RiskChecked => "风控",
        LiveOrderState::Submitted | LiveOrderState::Accepted => "等待成交确认",
        LiveOrderState::PartiallyFilled => "部分成交",
        LiveOrderState::Filled => "已成交",
        LiveOrderState::CancelRequested => "撤单中",
        LiveOrderState::Cancelled => "已取消",
        LiveOrderState::Rejected => "已拒绝",
        LiveOrderState::Failed => "失败",
        LiveOrderState::Unknown => "未知",
    }
}

pub(super) const fn order_state_tone(state: LiveOrderState) -> &'static str {
    match state {
        LiveOrderState::Created | LiveOrderState::RiskChecked => "draft",
        LiveOrderState::Submitted | LiveOrderState::Accepted | LiveOrderState::CancelRequested => {
            "pending"
        }
        LiveOrderState::PartiallyFilled => "warning",
        LiveOrderState::Filled => "ready",
        LiveOrderState::Cancelled => "idle",
        LiveOrderState::Rejected | LiveOrderState::Failed => "error",
        LiveOrderState::Unknown => "unknown",
    }
}

fn execution_run_state_label(run: &ExecutionRun) -> &'static str {
    if run.state == ExecutionRunState::Hedged && !run_legs_filled(run) {
        return "等待成交确认";
    }
    execution_state_label(run.state)
}

fn execution_run_tone(run: &ExecutionRun) -> &'static str {
    match run.state {
        ExecutionRunState::Closed => "ready",
        ExecutionRunState::Hedged if run_legs_filled(run) => "ready",
        ExecutionRunState::FirstLegPartial | ExecutionRunState::Unwinding => "warning",
        ExecutionRunState::UnwindRequired | ExecutionRunState::FailedSafe => "error",
        ExecutionRunState::Previewed
        | ExecutionRunState::RiskChecked
        | ExecutionRunState::SubmittingFirstLeg
        | ExecutionRunState::SubmittingSecondLeg
        | ExecutionRunState::SecondLegSubmitted
        | ExecutionRunState::Hedged => "pending",
    }
}

fn run_legs_filled(run: &ExecutionRun) -> bool {
    matches!(run.long_leg.state, LiveOrderState::Filled)
        && matches!(run.short_leg.state, LiveOrderState::Filled)
}

fn execution_state_label(state: ExecutionRunState) -> &'static str {
    match state {
        ExecutionRunState::Previewed => "已预览",
        ExecutionRunState::RiskChecked => "风控通过",
        ExecutionRunState::SubmittingFirstLeg => "提交第一腿",
        ExecutionRunState::FirstLegPartial => "第一腿部分成交",
        ExecutionRunState::SubmittingSecondLeg => "提交第二腿",
        ExecutionRunState::SecondLegSubmitted => "第二腿已提交，等待成交确认",
        ExecutionRunState::Hedged => "双腿完成",
        ExecutionRunState::UnwindRequired => "需要反向处理",
        ExecutionRunState::Unwinding => "反向处理中",
        ExecutionRunState::FailedSafe => "安全失败",
        ExecutionRunState::Closed => "执行已收口",
    }
}

fn side_label(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "买入",
        OrderSide::Sell => "卖出",
    }
}

pub(super) fn mode_label(mode: ExecutionMode) -> &'static str {
    execution_mode_label(mode)
}

pub(super) const fn order_environment_tone(mode: ExecutionMode) -> &'static str {
    match mode {
        ExecutionMode::DryRun | ExecutionMode::Testnet => "paper",
        ExecutionMode::Live => "live",
    }
}

pub(super) fn update_source_label(source: OrderUpdateSource) -> &'static str {
    match source {
        OrderUpdateSource::Unknown => "来源未知",
        OrderUpdateSource::Internal => "内部",
        OrderUpdateSource::AdapterAck => "ACK",
        OrderUpdateSource::OrderQuery => "查询",
        OrderUpdateSource::PrivateWs => "私有WS",
        OrderUpdateSource::FundingPoller => "资金费轮询",
        OrderUpdateSource::Reconcile => "回查",
        OrderUpdateSource::Manual => "手动",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_mode_labels_use_the_shared_paper_live_projection() {
        assert_eq!(mode_label(ExecutionMode::DryRun), "模拟");
        assert_eq!(mode_label(ExecutionMode::Testnet), "模拟");
        assert_eq!(mode_label(ExecutionMode::Live), "实盘");
        assert_eq!(order_environment_tone(ExecutionMode::DryRun), "paper");
        assert_eq!(order_environment_tone(ExecutionMode::Testnet), "paper");
        assert_eq!(order_environment_tone(ExecutionMode::Live), "live");
    }
}
