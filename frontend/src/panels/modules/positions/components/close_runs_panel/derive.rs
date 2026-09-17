//! 平仓事故（CloseRun）面板的纯派生：可操作筛选/排序、状态与成本/候选文案、补偿可提交判定。
//! 表格与行组件见父模块 `close_runs_panel.rs`，测试夹具见 `testing.rs`。

#[path = "derive/costs.rs"]
mod costs;

use shared_types::{
    CloseLegStatus, CloseRun, CloseRunCompensationAttempt, CloseRunNextAction,
    CloseRunNextActionKind, CloseRunStatus, CloseRunUnwindLegEvidence, CloseRunUnwindPlanStatus,
    LiveOrderState, OrderSide, OrderSource, CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE,
    CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE,
};

use super::super::section_state::SectionData;

pub(super) use costs::{
    close_candidate_evidence, close_candidate_label, close_candidate_title, close_run_cost_detail,
    close_run_cost_label, close_run_cost_title, compensation_button_label,
};
use costs::{close_candidate_notional_evidence, time_label};

pub(super) const RECENT_CLOSE_RUN_TABLE_LIMIT: usize = 8;

pub(super) fn close_run_rows(source: SectionData<Vec<CloseRun>>) -> SectionData<Vec<CloseRun>> {
    let mut rows = source
        .value
        .into_iter()
        .filter(close_run_needs_user_action)
        .collect::<Vec<_>>();
    rows.sort_by_key(|run| std::cmp::Reverse(run.updated_at_ms));
    let rows = rows
        .into_iter()
        .take(RECENT_CLOSE_RUN_TABLE_LIMIT)
        .collect();
    SectionData {
        value: rows,
        status: source.status,
    }
}

fn close_run_needs_user_action(run: &CloseRun) -> bool {
    if run.finality_problem.is_some() {
        return true;
    }
    matches!(
        run.status,
        CloseRunStatus::UnwindRequired
            | CloseRunStatus::CompensationSubmitted
            | CloseRunStatus::CompensationFailed
    )
}

pub(super) fn close_run_status_detail(run: &CloseRun) -> String {
    if let Some(problem) = run.finality_problem.as_ref() {
        return format!("终态回查异常：{}", problem.message);
    }
    if let Some(checked_at_ms) = run.finality_checked_at_ms {
        return format!("终态回查 {}", time_label(checked_at_ms));
    }
    run.message.clone()
}

pub(super) fn close_run_status_title(run: &CloseRun) -> String {
    let mut parts = vec![run.message.clone()];
    if let Some(problem) = run.finality_problem.as_ref() {
        parts.push(format!("finality {}", problem.code));
        if let Some(request_id) = problem.request_id.as_deref() {
            parts.push(format!("request {request_id}"));
        }
        if let Some(retry_after_ms) = problem.retry_after_ms {
            parts.push(format!("retryAfterMs {retry_after_ms}"));
        }
    }
    parts.join(" · ")
}

pub(super) fn close_run_next_action_detail(run: &CloseRun) -> String {
    let Some(plan) = run.unwind_plan.as_ref() else {
        return "下一步待终态回查".to_owned();
    };
    if plan.next_actions.is_empty() {
        return if run.status == CloseRunStatus::CompensationSubmitted {
            "等待后端终态对账".to_owned()
        } else {
            "无待处理动作".to_owned()
        };
    }
    plan.next_actions
        .iter()
        .map(next_action_label)
        .collect::<Vec<_>>()
        .join(" · ")
}

fn next_action_label(action: &CloseRunNextAction) -> String {
    let mut parts = vec![next_action_primary_label(action)];
    if let Some(index) = action.candidate_index {
        parts.push(format!("#{}", index + 1));
    }
    if action.requires_confirmation {
        parts.push("需确认".to_owned());
    }
    if !action.required_evidence.is_empty() {
        parts.push(format!("证据 {}", action.required_evidence.join(",")));
    }
    if let Some(reason) = action.reason.as_deref().filter(|reason| !reason.is_empty()) {
        parts.push(format!("原因 {reason}"));
    }
    parts.join(" ")
}

fn next_action_primary_label(action: &CloseRunNextAction) -> String {
    if action.label.trim().is_empty() {
        next_action_kind_label(action.kind).to_owned()
    } else {
        action.label.clone()
    }
}

fn next_action_kind_label(kind: CloseRunNextActionKind) -> &'static str {
    match kind {
        CloseRunNextActionKind::SubmitCompensationOrder => "提交补偿",
        CloseRunNextActionKind::CancelCompensationOrder => "撤销补偿",
        CloseRunNextActionKind::WaitForCompensationFinality => "等待补偿终态",
        CloseRunNextActionKind::ManualIncidentReview => "人工复核",
    }
}

pub(super) fn close_run_remaining_positions_detail(run: &CloseRun) -> String {
    let Some(plan) = run.unwind_plan.as_ref() else {
        return "剩余仓位待回查".to_owned();
    };
    if plan.remaining_positions.is_empty() {
        return if run.status == CloseRunStatus::CompensationSubmitted {
            "当前快照无裸露仓位，仍待补偿终态".to_owned()
        } else {
            "无剩余裸露仓位".to_owned()
        };
    }
    plan.remaining_positions
        .iter()
        .map(remaining_position_label)
        .collect::<Vec<_>>()
        .join(" · ")
}

fn remaining_position_label(position: &CloseRunUnwindLegEvidence) -> String {
    format!(
        "{} {} {} {}",
        position.venue,
        position.symbol,
        position_side_label(position.side),
        close_candidate_notional_evidence(position)
    )
}

fn position_side_label(side: shared_types::PositionSide) -> &'static str {
    match side {
        shared_types::PositionSide::Long => "多",
        shared_types::PositionSide::Short => "空",
    }
}

pub(super) fn compensation_candidates(run: &CloseRun) -> Vec<(usize, CloseRunUnwindLegEvidence)> {
    run.unwind_plan
        .as_ref()
        .map(|plan| {
            plan.compensation_candidates
                .iter()
                .cloned()
                .enumerate()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(super) fn can_submit_compensation(run: &CloseRun, phrase: &str) -> bool {
    run.status == CloseRunStatus::UnwindRequired
        && phrase.trim() == CLOSE_RUN_COMPENSATION_CONFIRMATION_PHRASE
        && !compensation_candidates(run).is_empty()
}

pub(super) fn can_submit_manual_terminal(run: &CloseRun, phrase: &str, reason: &str) -> bool {
    run.status == CloseRunStatus::CompensationFailed
        && phrase.trim() == CLOSE_RUN_MANUAL_TERMINAL_CONFIRMATION_PHRASE
        && !reason.trim().is_empty()
        && has_manual_terminal_action(run)
}

pub(super) fn has_manual_terminal_action(run: &CloseRun) -> bool {
    run.unwind_plan.as_ref().is_some_and(|plan| {
        plan.next_actions
            .iter()
            .any(|action| action.kind == CloseRunNextActionKind::ManualIncidentReview)
    })
}

pub(super) fn cancellable_compensation_attempts(
    run: &CloseRun,
) -> Vec<CloseRunCompensationAttempt> {
    if run.status != CloseRunStatus::CompensationSubmitted {
        return Vec::new();
    }
    run.unwind_plan
        .as_ref()
        .filter(|plan| plan.status == CloseRunUnwindPlanStatus::CompensationSubmitted)
        .map(|plan| {
            plan.compensation_attempts
                .iter()
                .filter(|attempt| compensation_attempt_can_cancel(attempt))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn compensation_attempt_can_cancel(attempt: &CloseRunCompensationAttempt) -> bool {
    matches!(
        attempt.status,
        CloseLegStatus::Submitted | CloseLegStatus::Accepted
    ) && attempt.order.as_ref().is_some_and(|order| {
        order.intent.source == OrderSource::CloseRunCompensation
            && matches!(
                order.state,
                LiveOrderState::Submitted | LiveOrderState::Accepted | LiveOrderState::Unknown
            )
            && !order.intent.id.trim().is_empty()
    })
}

pub(super) fn compensation_cancel_order_id(
    attempt: &CloseRunCompensationAttempt,
) -> Option<String> {
    attempt
        .order
        .as_ref()
        .map(|order| order.intent.id.trim().to_owned())
        .filter(|order_id| !order_id.is_empty())
}

pub(super) fn compensation_cancel_button_label(
    attempt: &CloseRunCompensationAttempt,
) -> &'static str {
    match attempt.compensation_order_side {
        OrderSide::Buy => "撤补买",
        OrderSide::Sell => "撤补卖",
    }
}

pub(super) fn compensation_cancel_title(attempt: &CloseRunCompensationAttempt) -> String {
    let Some(order) = attempt.order.as_ref() else {
        return "补偿订单缺少 order evidence".to_owned();
    };
    let mut parts = vec![format!("order {}", order.intent.id)];
    parts.push(format!("client {}", order.intent.client_order_id));
    if let Some(exchange_order_id) = order.exchange_order_id.as_deref() {
        parts.push(format!("exchange {exchange_order_id}"));
    }
    if let Some(action_run_id) = attempt.action_run_id.as_deref() {
        parts.push(format!("Action {action_run_id}"));
    }
    parts.join(" · ")
}

pub(super) fn close_run_status_text(rows: &SectionData<Vec<CloseRun>>) -> String {
    if rows.value.is_empty() {
        return rows.status.empty_text("0 条待处理", "读取中", "读取失败");
    }
    format!("{} 条待处理", rows.value.len())
}

pub(super) fn close_run_status_label(status: CloseRunStatus) -> &'static str {
    match status {
        CloseRunStatus::UnwindRequired => "需补偿",
        CloseRunStatus::CompensationSubmitted => "补偿中",
        CloseRunStatus::Compensated => "已补偿",
        CloseRunStatus::CompensationFailed => "补偿失败",
        CloseRunStatus::ManuallyResolved => "已人工终结",
        CloseRunStatus::Submitted => "已提交",
        CloseRunStatus::Succeeded => "已完成",
        CloseRunStatus::PartiallySubmitted => "部分提交",
        CloseRunStatus::Failed => "失败",
    }
}

pub(super) fn close_run_row_class(status: CloseRunStatus) -> &'static str {
    match status {
        CloseRunStatus::UnwindRequired | CloseRunStatus::CompensationFailed => "danger-row",
        CloseRunStatus::CompensationSubmitted => "warning-row",
        _ => "",
    }
}
