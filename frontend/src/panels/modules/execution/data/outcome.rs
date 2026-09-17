use shared_types::{
    HedgeConfirmContext, HedgeConfirmPartialCause, HedgeConfirmResponse, HedgeConfirmStatus,
    HedgeConfirmUnwindStatus, LiveOrderState, OrderRecord, RecoveryAction,
};

use crate::panels::shared::execution_environment_label;

#[cfg(test)]
#[path = "outcome/tests.rs"]
mod tests;

pub(in crate::panels::modules::execution) fn confirm_context_detail(
    context: &HedgeConfirmContext,
) -> Option<String> {
    if context.opportunity_id.is_empty() && context.idempotency_key.is_empty() {
        return None;
    }
    let mut parts = Vec::new();
    if let Some(environment) = context.environment {
        parts.push(execution_environment_label(environment).to_owned());
    }
    if let Some(ticket_id) = context.ticket_id.as_deref() {
        parts.push(format!("Ticket {ticket_id}"));
    }
    if let Some(run_id) = context.run_id.as_deref() {
        parts.push(format!("Run {run_id}"));
    }
    if !context.idempotency_key.is_empty() {
        parts.push(format!("Idempotency {}", context.idempotency_key));
    }
    push_leg_context(
        &mut parts,
        "long",
        context.long_venue.as_deref(),
        context.long_problem.as_ref(),
    );
    push_leg_context(
        &mut parts,
        "short",
        context.short_venue.as_deref(),
        context.short_problem.as_ref(),
    );
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn push_leg_context(
    parts: &mut Vec<String>,
    role: &str,
    venue: Option<&str>,
    problem: Option<&shared_types::ApiProblem>,
) {
    let Some(venue) = venue else {
        return;
    };
    let mut detail = format!("{role} {venue}");
    if let Some(problem) = problem {
        detail.push_str(&format!(" [{}]", problem.code));
    }
    parts.push(detail);
}

pub(in crate::panels::modules::execution) fn confirm_outcome_detail(
    response: &HedgeConfirmResponse,
) -> Option<String> {
    let Some(outcome) = response.partial_outcome.as_ref() else {
        return Some(basic_confirm_outcome_detail(response));
    };
    let mut parts = vec![
        format!("事故 {}", cause_label(outcome.cause)),
        format!("unwind {}", unwind_status_label(outcome.unwind_status)),
    ];
    if let Some(quantity) = outcome.unwind_quantity {
        parts.push(format!("qty {}", compact_number(quantity)));
    }
    if let Some(action) = outcome.recovery_action {
        parts.push(format!("恢复 {}", recovery_action_label(action)));
    }
    if outcome.manual_review_required {
        parts.push("需要人工复核".into());
    }
    if let Some(message) = outcome.primary_message.as_deref() {
        parts.push(message.to_owned());
    }
    if let Some(problem) = outcome
        .unwind_problem
        .as_ref()
        .or(outcome.primary_problem.as_ref())
        .or(response.problem.as_ref())
    {
        parts.push(format!("code {}", problem.code));
    }
    if let Some(record) = response.long_record.as_ref() {
        parts.push(format!("long {}", order_record_summary(record)));
    }
    if let Some(record) = response.short_record.as_ref() {
        parts.push(format!("short {}", order_record_summary(record)));
    }
    if let Some(record) = response.unwind_record.as_ref() {
        parts.push(format!("unwind {}", order_record_summary(record)));
    }
    Some(parts.join(" · "))
}

pub(in crate::panels::modules::execution) fn confirm_outcome_summary(
    response: &HedgeConfirmResponse,
) -> Option<String> {
    if let Some(outcome) = response.partial_outcome.as_ref() {
        return Some(format!(
            "异常处置 · {} · {}",
            cause_label(outcome.cause),
            unwind_status_label(outcome.unwind_status)
        ));
    }
    let mut parts = vec![format!(
        "提交结果 · {}",
        confirm_status_label(response.status)
    )];
    if let Some(run) = response.execution_run.as_ref() {
        let symbol = display_symbol(&run.long_leg.symbol, &run.short_leg.symbol);
        if !symbol.is_empty() {
            parts.push(symbol);
        }
        parts.push(format!(
            "{} / {}",
            run.long_leg.exchange, run.short_leg.exchange
        ));
    }
    Some(parts.join(" · "))
}

fn display_symbol(long_symbol: &str, short_symbol: &str) -> String {
    if !long_symbol.trim().is_empty() {
        long_symbol.to_owned()
    } else {
        short_symbol.to_owned()
    }
}

fn basic_confirm_outcome_detail(response: &HedgeConfirmResponse) -> String {
    let mut parts = vec![format!("结果 {}", confirm_status_label(response.status))];
    if !response.idempotency_key.trim().is_empty() {
        parts.push(format!("idempotency {}", response.idempotency_key));
    }
    if let Some(run) = response.execution_run.as_ref() {
        parts.push(format!("run {}", run.run_id));
        parts.push(format!("ticket {}", run.ticket_id));
    }
    if let Some(problem) = response.problem.as_ref() {
        parts.push(format!("code {}", problem.code));
    }
    if let Some(error) = response
        .error
        .as_deref()
        .filter(|error| !error.trim().is_empty())
    {
        parts.push(error.to_owned());
    }
    parts.join(" · ")
}

const fn confirm_status_label(status: HedgeConfirmStatus) -> &'static str {
    match status {
        HedgeConfirmStatus::Submitted => "已提交",
        HedgeConfirmStatus::Replayed => "已重放",
        HedgeConfirmStatus::LongLegFailed => "第一执行腿失败",
        HedgeConfirmStatus::ValuationMissing => "估值缺失",
        HedgeConfirmStatus::FirstLegPartialUnwindAttempted => "第一腿部分成交，已尝试反向",
        HedgeConfirmStatus::FirstLegPartialUnwindFailed => "第一腿部分成交，反向失败",
        HedgeConfirmStatus::FirstLegPartialWaitingFillQty => "等待第一腿成交数量",
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindAttempted => "二次复检阻断，已尝试反向",
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindFailed => "二次复检阻断，反向失败",
        HedgeConfirmStatus::HedgeBrokenUnwindAttempted => "第二腿失败，已尝试反向",
        HedgeConfirmStatus::HedgeBrokenUnwindFailed => "第二腿失败，反向失败",
        HedgeConfirmStatus::Unknown => "未知",
    }
}

fn cause_label(cause: HedgeConfirmPartialCause) -> &'static str {
    match cause {
        HedgeConfirmPartialCause::FirstLegPartial => "第一腿部分成交",
        HedgeConfirmPartialCause::HedgeRecheckBlocked => "二次复检拒绝",
        HedgeConfirmPartialCause::HedgeBroken => "第二腿失败",
        HedgeConfirmPartialCause::Unknown => "未知",
    }
}

fn unwind_status_label(status: HedgeConfirmUnwindStatus) -> &'static str {
    match status {
        HedgeConfirmUnwindStatus::Submitted => "已提交",
        HedgeConfirmUnwindStatus::SubmitFailed => "提交失败",
        HedgeConfirmUnwindStatus::AwaitingFillQuantity => "等待成交数量",
        HedgeConfirmUnwindStatus::Unknown => "未知",
    }
}

fn recovery_action_label(action: RecoveryAction) -> &'static str {
    match action {
        RecoveryAction::CancelOpenOrders => "撤销挂单",
        RecoveryAction::UnwindLongLeg => "反向长腿",
        RecoveryAction::UnwindShortLeg => "反向短腿",
        RecoveryAction::ManualReview => "人工复核",
    }
}

fn order_record_summary(record: &OrderRecord) -> String {
    let mut parts = vec![
        record.intent.exchange.clone(),
        record.intent.symbol.clone(),
        live_state_label(record.state).into(),
    ];
    if let Some(quantity) = record.filled_quantity {
        parts.push(format!("filled {}", compact_number(quantity)));
    }
    if let Some(order_id) = record.exchange_order_id.as_deref() {
        parts.push(format!("exchange {order_id}"));
    }
    parts.join("/")
}

fn live_state_label(state: LiveOrderState) -> &'static str {
    match state {
        LiveOrderState::Created => "created",
        LiveOrderState::RiskChecked => "risk_checked",
        LiveOrderState::Submitted => "submitted",
        LiveOrderState::Accepted => "accepted",
        LiveOrderState::PartiallyFilled => "partial",
        LiveOrderState::Filled => "filled",
        LiveOrderState::CancelRequested => "cancel_requested",
        LiveOrderState::Cancelled => "cancelled",
        LiveOrderState::Rejected => "rejected",
        LiveOrderState::Failed => "failed",
        LiveOrderState::Unknown => "unknown",
    }
}

fn compact_number(value: f64) -> String {
    if value.abs() >= 100.0 {
        format!("{value:.2}")
    } else if value.abs() >= 1.0 {
        format!("{value:.4}")
    } else {
        format!("{value:.6}")
    }
}
