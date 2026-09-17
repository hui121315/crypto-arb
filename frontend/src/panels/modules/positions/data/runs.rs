//! Close-run 状态机派生与表格键/标签助手。
//!
//! 把后端 `CloseRun` 状态映射成 UI 的 `ActionState`、失败文案与补偿提示，并提供
//! positions 表格的稳定行键/配对键。失败/缺失补偿一律产出明确 `ApiProblem`
//! （fail-closed），绝不静默吞掉。订单终态与账户快照由 `AppWS` 分别投影；订单动作
//! 不主动重拉账户 REST，以免旧快照覆盖随后到达的私有 WS 账户事实。

use crate::state::action_state::ActionState;
use leptos::prelude::*;
use shared_types::{
    ActionEvidence, ApiProblem, CloseRun, CloseRunStatus, KillSwitchResponse, LiveOrderState,
    OrderRecord, PositionRow,
};

pub(in crate::panels::modules::positions) const PREVIOUS_CLOSE_RECORD_LABEL: &str = "上一笔平仓";

#[path = "runs/attempts.rs"]
mod attempts;
pub(in crate::panels::modules::positions) use attempts::{
    close_run_next_attempt_anchor, latest_close_all_attempt_anchor,
    latest_position_close_attempt_anchor,
};

#[path = "runs/failures.rs"]
mod failures;
pub(in crate::panels::modules::positions) use failures::close_run_problem;
#[cfg(test)]
pub(in crate::panels::modules::positions) use failures::close_run_retry_anchor;

#[path = "runs/recovery.rs"]
mod recovery;
#[cfg(test)]
pub(in crate::panels::modules::positions) use recovery::should_recover_close_run;
pub(in crate::panels::modules::positions) use recovery::{
    recover_close_all_state, recover_compensation_state, recover_position_close_state,
};

pub(in crate::panels::modules::positions) fn bump_refresh(refresh_nonce: RwSignal<u64>) {
    refresh_nonce.update(|value| *value = value.wrapping_add(1));
}

pub(in crate::panels::modules::positions) fn position_key(row: &PositionRow) -> String {
    format!("{}:{}:{:?}", row.venue, row.symbol, row.side)
}

pub(in crate::panels::modules::positions) fn pair_close_key(row: &PositionRow) -> Option<String> {
    row.pair_evidence
        .as_ref()
        .map(|pair| format!("pair:{}", pair.run_id))
}

pub(in crate::panels::modules::positions) fn pair_label(row: &PositionRow) -> Option<String> {
    row.pair_evidence
        .as_ref()
        .map(|pair| format!("{}@{}", pair.partner_venue, pair.partner_symbol))
}

pub(in crate::panels::modules::positions) fn has_pair_evidence(row: &PositionRow) -> bool {
    row.pair_evidence.is_some()
}

pub(in crate::panels::modules::positions) fn close_label(row: &PositionRow) -> String {
    format!("{} {}", row.venue, row.symbol)
}

pub(in crate::panels::modules::positions) fn compensation_key(
    run: &CloseRun,
    candidate_index: usize,
) -> String {
    format!("compensation:{}:{candidate_index}", run.id)
}

pub(in crate::panels::modules::positions) fn cancel_compensation_key(
    run: &CloseRun,
    order_id: &str,
) -> String {
    format!("compensation-cancel:{}:{order_id}", run.id)
}

pub(in crate::panels::modules::positions) fn manual_terminal_key(run: &CloseRun) -> String {
    format!("manual-terminal:{}", run.id)
}

pub(in crate::panels::modules::positions) fn apply_close_run_result(
    state: RwSignal<ActionState>,
    label: &str,
    run: &CloseRun,
    pending_evidence: ActionEvidence,
) {
    state.set(close_run_action_state(label, run).with_evidence(pending_evidence));
}

pub(in crate::panels::modules::positions) fn apply_cancel_order_result(
    state: RwSignal<ActionState>,
    order: &OrderRecord,
    pending_evidence: ActionEvidence,
) {
    state.set(cancel_order_action_state(order).with_evidence(pending_evidence));
}

pub(in crate::panels::modules::positions) fn close_run_action_state(
    label: &str,
    run: &CloseRun,
) -> ActionState {
    let state = match run.status {
        CloseRunStatus::Submitted | CloseRunStatus::CompensationSubmitted => {
            ActionState::accepted(close_run_status_message(label, run))
        }
        CloseRunStatus::Succeeded
        | CloseRunStatus::Compensated
        | CloseRunStatus::ManuallyResolved => {
            ActionState::succeeded(close_run_status_message(label, run))
        }
        CloseRunStatus::PartiallySubmitted
        | CloseRunStatus::UnwindRequired
        | CloseRunStatus::CompensationFailed
        | CloseRunStatus::Failed => {
            ActionState::failed(close_run_failure_label(label, run), close_run_problem(run))
        }
    };
    state.with_evidence(ActionEvidence::from_close_run(run))
}

fn cancel_order_action_state(order: &OrderRecord) -> ActionState {
    let message = cancel_order_status_message(order);
    let state = match order.state {
        LiveOrderState::Cancelled => ActionState::succeeded(message),
        LiveOrderState::Rejected | LiveOrderState::Failed => ActionState::failed(
            "补偿撤单失败",
            ApiProblem::new(
                shared_types::problem::codes::CLOSE_RUN_COMPENSATION_FAILED,
                message,
            )
            .with_source("positions.close_run_compensation_cancel"),
        ),
        _ => ActionState::accepted(message),
    };
    state.with_evidence(ActionEvidence::from_order_record(order))
}

pub(in crate::panels::modules::positions) fn kill_switch_response_evidence(
    response: &KillSwitchResponse,
    pending: ActionEvidence,
) -> ActionEvidence {
    pending
        .with_request_id(response.request_id.clone())
        .with_action_run_id(response.action_run_id.clone())
        .with_idempotency_key(response.idempotency_key.clone())
}

fn cancel_order_status_message(order: &OrderRecord) -> String {
    let state = match order.state {
        LiveOrderState::Cancelled => "已确认取消",
        LiveOrderState::CancelRequested => "撤单已提交，等待终态",
        LiveOrderState::Filled => "已成交，等待 CloseRun 更新",
        LiveOrderState::PartiallyFilled => "部分成交，等待撤单终态",
        LiveOrderState::Rejected => "交易所拒绝撤单",
        LiveOrderState::Failed => "撤单失败",
        _ => "撤单请求已受理",
    };
    format!("补偿撤单：{state} · {}", order.intent.id)
}

fn close_run_status_message(label: &str, run: &CloseRun) -> String {
    format!("{label}：{}", run.message)
}

pub(in crate::panels::modules::positions) fn close_run_failure_label(
    label: &str,
    run: &CloseRun,
) -> String {
    let label = if run.naked_exposure_usd > 0.0 {
        let prefix = if close_run_needs_compensation(run.status) {
            "需补偿处理，裸露"
        } else {
            "未完全完成，裸露"
        };
        format!("{label}{prefix} ${:.0}", run.naked_exposure_usd)
    } else {
        let suffix = if close_run_needs_compensation(run.status) {
            "需补偿处理"
        } else {
            "未完全完成"
        };
        format!("{label}{suffix}")
    };
    label
}

fn close_run_needs_compensation(status: CloseRunStatus) -> bool {
    matches!(
        status,
        CloseRunStatus::UnwindRequired
            | CloseRunStatus::CompensationSubmitted
            | CloseRunStatus::CompensationFailed
    )
}

pub(in crate::panels::modules::positions) fn missing_pair_problem(row: &PositionRow) -> ApiProblem {
    ApiProblem::new(
        "PAIR_NOT_FOUND",
        format!("{} {} 未找到配对腿", row.venue, row.symbol),
    )
    .with_source("positions")
}

pub(in crate::panels::modules::positions) fn kill_switch_success_message(
    prefix: &str,
    response: &KillSwitchResponse,
) -> String {
    let state = if response.summary.active {
        "已开启"
    } else {
        "已关闭"
    };
    let mut message = format!(
        "{prefix}{state} · 原因 {} · 挂单 {}",
        response.summary.reason, response.summary.open_order_count
    );
    if let Some(action_run_id) = response.action_run_id.as_deref() {
        message.push_str(" · Action ");
        message.push_str(action_run_id);
    }
    if let Some(request_id) = response.request_id.as_deref() {
        message.push_str(" · Request ");
        message.push_str(request_id);
    }
    if let Some(idempotency_key) = response.idempotency_key.as_deref() {
        message.push_str(" · Idempotency ");
        message.push_str(idempotency_key);
    }
    message
}
