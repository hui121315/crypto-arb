//! 执行状态条的运行态派生：状态/原因/元信息文案、费用证据收敛、阶段与提示状态映射。
//! 纯文案格式化见 `format.rs`，视图组件见父模块 `execution_status_bar.rs`。

use shared_types::{
    ApiProblem, ExecutionRun, ExecutionRunLeg, ExecutionRunState, LiveOrderState, RecoveryAction,
};

use crate::api::ws::{WsChannelState, WsStatus};
use crate::panels::shared::ws_channel_activity_label;

use super::format::{money, time_label};

pub(super) fn state_text(run: Option<&ExecutionRun>) -> String {
    run.map(run_state_label).unwrap_or("等待提交".to_owned())
}

#[cfg(test)]
pub(super) fn reason_text(
    run: Option<&ExecutionRun>,
    seed_problem: Option<&ApiProblem>,
    stream_problem: Option<&ApiProblem>,
) -> String {
    reason_text_with_channel(run, seed_problem, stream_problem, None)
}

pub(super) fn reason_text_with_channel(
    run: Option<&ExecutionRun>,
    seed_problem: Option<&ApiProblem>,
    stream_problem: Option<&ApiProblem>,
    channel_state: Option<&WsChannelState>,
) -> String {
    if let Some(problem) = stream_problem {
        return format!("ExecutionRun 执行流异常：{}", problem_message(problem));
    }
    if let Some(problem) = seed_problem {
        return format!("ExecutionRun 恢复失败：{}", problem_message(problem));
    }
    if let Some(problem) = run.and_then(|run| run.finality_problem.as_ref()) {
        return format!("ExecutionRun 最终结果回查异常：{}", problem_message(problem));
    }
    if let Some(problem) = run.and_then(run_problem) {
        return format!("ExecutionRun 补救异常：{}", problem_message(problem));
    }
    if let Some(problem) = channel_state.and_then(|state| state.last_error.as_ref()) {
        return format!("ExecutionRun 通道异常：{}", problem_message(problem));
    }
    run.map(|run| run.status_reason.clone())
        .unwrap_or("预览通过后提交双腿执行。".to_owned())
}

#[cfg(test)]
pub(super) fn status_meta_text(
    run: Option<&ExecutionRun>,
    seed_problem: Option<&ApiProblem>,
    stream_problem: Option<&ApiProblem>,
) -> String {
    status_meta_text_with_channel(run, seed_problem, stream_problem, None)
}

pub(super) fn status_meta_text_with_channel(
    run: Option<&ExecutionRun>,
    seed_problem: Option<&ApiProblem>,
    stream_problem: Option<&ApiProblem>,
    channel_state: Option<&WsChannelState>,
) -> String {
    if stream_problem.is_some() {
        "执行流异常".into()
    } else if seed_problem.is_some() {
        "恢复失败".into()
    } else if run.and_then(|run| run.finality_problem.as_ref()).is_some() {
        "最终结果回查异常".into()
    } else if let Some(checked_at_ms) = run.and_then(|run| run.finality_checked_at_ms) {
        format!("最终结果回查 {}", time_label(checked_at_ms))
    } else if let Some(channel_text) = channel_state.and_then(channel_meta_text) {
        let base = exposure_text(run);
        if base == "未开始" {
            channel_text
        } else {
            format!("{base} · {channel_text}")
        }
    } else {
        exposure_text(run)
    }
}

fn channel_meta_text(state: &WsChannelState) -> Option<String> {
    let status = if let Some(problem) = state.last_error.as_ref() {
        format!("WS异常 {}", problem_message(problem))
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
    Some(format!(
        "{} 通道 · {status} · {}",
        state.channel,
        ws_channel_activity_label(state)
    ))
}

fn u64_ms_to_i64(value: u64) -> i64 {
    if value > i64::MAX as u64 {
        i64::MAX
    } else {
        value as i64
    }
}

fn exposure_text(run: Option<&ExecutionRun>) -> String {
    run.map(|run| {
        let exposure = format!("裸露 {}", money(run.net_exposure_usd));
        match filled_fee(run) {
            FeeEvidence::Complete(fee) if fee.abs() > f64::EPSILON => {
                format!("{exposure} · 成交费 {}", money(fee))
            }
            FeeEvidence::Complete(_) => exposure,
            FeeEvidence::Missing => format!("{exposure} · 成交费数据待确认"),
            FeeEvidence::Waiting => format!("{exposure} · 成交费待成交回报"),
        }
    })
    .unwrap_or_else(|| "未开始".into())
}

enum FeeEvidence {
    Complete(f64),
    Missing,
    Waiting,
}

fn filled_fee(run: &ExecutionRun) -> FeeEvidence {
    match (run.long_leg.filled_fee, run.short_leg.filled_fee) {
        (Some(long_fee), Some(short_fee)) => FeeEvidence::Complete(long_fee + short_fee),
        _ if has_fee_missing_fill(&run.long_leg) || has_fee_missing_fill(&run.short_leg) => {
            FeeEvidence::Missing
        }
        _ => FeeEvidence::Waiting,
    }
}

fn has_fee_missing_fill(leg: &ExecutionRunLeg) -> bool {
    leg.filled_fee.is_none() && leg_has_fill_evidence(leg)
}

fn leg_has_fill_evidence(leg: &ExecutionRunLeg) -> bool {
    matches!(
        leg.state,
        LiveOrderState::PartiallyFilled | LiveOrderState::Filled
    ) || leg.filled_quantity.is_some()
        || leg.filled_notional_usd.is_some()
}

fn problem_message(problem: &ApiProblem) -> String {
    let mut parts = vec![problem.message.clone()];
    if !problem.code.trim().is_empty() {
        parts.push(format!("code {}", problem.code));
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

fn run_problem(run: &ExecutionRun) -> Option<&ApiProblem> {
    run.unwind_problem
        .as_ref()
        .or(run.valuation_problem.as_ref())
}

pub(super) fn stage_class(run: Option<&ExecutionRun>, stage: u8) -> &'static str {
    let Some(run) = run else {
        return "execution-step";
    };
    if run_stage(run.state) == stage {
        "execution-step active"
    } else if run_stage(run.state) > stage {
        "execution-step done"
    } else {
        "execution-step"
    }
}

pub(super) fn run_stage(state: ExecutionRunState) -> u8 {
    match state {
        ExecutionRunState::Previewed | ExecutionRunState::RiskChecked => 1,
        ExecutionRunState::SubmittingFirstLeg | ExecutionRunState::FirstLegPartial => 2,
        ExecutionRunState::SubmittingSecondLeg | ExecutionRunState::SecondLegSubmitted => 3,
        ExecutionRunState::UnwindRequired | ExecutionRunState::Unwinding => 4,
        ExecutionRunState::Hedged | ExecutionRunState::Closed | ExecutionRunState::FailedSafe => 5,
    }
}

pub(super) fn state_notice_text(run: &ExecutionRun) -> Option<String> {
    match run.state {
        ExecutionRunState::FirstLegPartial => Some(format!(
            "第一腿部分成交，裸露 {}，等待第二腿补齐或反向处理。",
            money(run.net_exposure_usd)
        )),
        ExecutionRunState::SecondLegSubmitted => {
            Some("第二腿已提交，等待私有 WS 或订单回查确认双腿成交。".into())
        }
        ExecutionRunState::Hedged if !run_legs_filled(run) => {
            Some("后端已进入最终结果，但双腿成交回报未完整确认，等待订单最终结果回查。".into())
        }
        ExecutionRunState::UnwindRequired => {
            let problem = run
                .unwind_problem
                .as_ref()
                .map(|problem| format!(" · {}", problem_message(problem)))
                .unwrap_or_default();
            Some(format!(
                "需要补救：{}，裸露 {}{}。",
                recovery_action_label(run.recovery_action),
                money(run.net_exposure_usd),
                problem
            ))
        }
        ExecutionRunState::Unwinding => Some(format!(
            "反向处理中：{}，当前裸露 {}。",
            recovery_action_label(run.recovery_action),
            money(run.net_exposure_usd)
        )),
        ExecutionRunState::FailedSafe => Some(format!(
            "执行失败进入安全状态，裸露 {}，需要人工复核。",
            money(run.net_exposure_usd)
        )),
        ExecutionRunState::Closed if run_legs_filled(run) => Some(
            "执行已收口；原始双腿成交回报已确认，平仓或补偿结果以时间线和订单最终结果为准。".into(),
        ),
        ExecutionRunState::Closed => {
            Some("执行已收口；原始双腿成交回报不完整，继续核对补偿记录与订单最终结果。".into())
        }
        _ => None,
    }
}

pub(super) fn state_notice_class(run: &ExecutionRun) -> &'static str {
    match run.state {
        ExecutionRunState::FailedSafe => "execution-state-notice danger",
        ExecutionRunState::Closed => "execution-state-notice closed",
        _ => "execution-state-notice warn",
    }
}

fn recovery_action_label(action: Option<RecoveryAction>) -> &'static str {
    match action {
        Some(RecoveryAction::CancelOpenOrders) => "撤销未成交订单",
        Some(RecoveryAction::UnwindLongLeg) => "反向处理多腿",
        Some(RecoveryAction::UnwindShortLeg) => "反向处理空腿",
        Some(RecoveryAction::ManualReview) => "人工复核",
        None => "等待后端补救指令",
    }
}

pub(in crate::panels::modules::execution) fn run_state_label(run: &ExecutionRun) -> String {
    if run.state == ExecutionRunState::Hedged && !run_legs_filled(run) {
        return "等待成交确认".to_owned();
    }
    state_label(run.state)
}

pub(in crate::panels::modules::execution) fn run_requires_attention(run: &ExecutionRun) -> bool {
    run.state != ExecutionRunState::Closed
        || !run_legs_filled(run)
        || run.finality_problem.is_some()
        || run.unwind_problem.is_some()
        || run.valuation_problem.is_some()
}

fn run_legs_filled(run: &ExecutionRun) -> bool {
    matches!(run.long_leg.state, LiveOrderState::Filled)
        && matches!(run.short_leg.state, LiveOrderState::Filled)
}

fn state_label(state: ExecutionRunState) -> String {
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
    .to_owned()
}
