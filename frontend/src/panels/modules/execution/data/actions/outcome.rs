//! Hedge-confirm context recovery, action evidence, labels, and failure classification.

use shared_types::{
    problem::codes, ActionEvidence, ApiProblem, ExecutionRunState, HedgeConfirmContext,
    HedgeConfirmResponse, HedgeConfirmStatus,
};

pub(super) fn resolved_confirm_context(
    response: &HedgeConfirmResponse,
    fallback: &HedgeConfirmContext,
) -> HedgeConfirmContext {
    let mut context = response.context.clone();
    if context.opportunity_id.is_empty() {
        context.opportunity_id.clone_from(&fallback.opportunity_id);
    }
    if context.idempotency_key.is_empty() {
        context
            .idempotency_key
            .clone_from(&response.idempotency_key);
    }
    if context.ticket_id.is_none() {
        context.ticket_id = response
            .execution_run
            .as_ref()
            .map(|run| run.ticket_id.clone())
            .or_else(|| fallback.ticket_id.clone());
    }
    if context.run_id.is_none() {
        context.run_id = response
            .execution_run
            .as_ref()
            .map(|run| run.run_id.clone());
    }
    if context.environment.is_none() {
        context.environment = fallback.environment;
    }
    if context.long_venue.is_none() {
        context.long_venue = response
            .execution_run
            .as_ref()
            .map(|run| run.long_leg.exchange.clone())
            .or_else(|| fallback.long_venue.clone());
    }
    if context.short_venue.is_none() {
        context.short_venue = response
            .execution_run
            .as_ref()
            .map(|run| run.short_leg.exchange.clone())
            .or_else(|| fallback.short_venue.clone());
    }
    context
}

pub(super) fn confirm_context_from_problem(
    problem: &ApiProblem,
) -> Result<Option<HedgeConfirmContext>, serde_json::Error> {
    let Some(value) = problem
        .details
        .as_ref()
        .and_then(|details| details.get("confirmContext"))
        .cloned()
    else {
        return Ok(None);
    };
    serde_json::from_value(value).map(Some)
}

pub(super) fn attach_confirm_context_decode_problem(
    problem: &mut ApiProblem,
    error: &serde_json::Error,
) {
    let details = problem.details.take();
    let mut object = match details {
        Some(serde_json::Value::Object(object)) => object,
        Some(value) => serde_json::Map::from_iter([("originalDetails".to_owned(), value)]),
        None => serde_json::Map::new(),
    };
    object.insert(
        "confirmContextDecodeProblem".to_owned(),
        serde_json::json!({
            "code": codes::HEDGE_CONFIRM_CONTEXT_DECODE_FAILED,
            "message": error.to_string(),
            "source": "hedge_confirm_action",
        }),
    );
    problem.details = Some(serde_json::Value::Object(object));
}

pub(super) fn confirm_response_evidence(
    response: &HedgeConfirmResponse,
    pending: ActionEvidence,
) -> ActionEvidence {
    let mut evidence = pending
        .with_idempotency_key(Some(response.idempotency_key.clone()))
        .with_run_id(response.context.run_id.clone())
        .with_ticket_id(response.context.ticket_id.clone());
    if let Some(run) = &response.execution_run {
        evidence.merge(ActionEvidence::from_execution_run(run));
    }
    for order in [
        response.long_record.as_ref(),
        response.short_record.as_ref(),
        response.unwind_record.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        evidence.merge(ActionEvidence::from_order_record(order));
    }
    evidence
}

pub(super) fn submitting_label(mode: &str) -> &'static str {
    match mode {
        "模拟" => "模拟提交中",
        "实盘" => "实盘提交中",
        _ => "提交中",
    }
}

pub(super) fn submit_failed_label(mode: &str) -> &'static str {
    match mode {
        "模拟" => "模拟提交失败",
        "实盘" => "实盘提交失败",
        _ => "提交失败",
    }
}

fn submitted_label(mode: &str) -> &'static str {
    match mode {
        "模拟" => "模拟已提交，等待成交确认",
        "实盘" => "实盘已提交，等待成交确认",
        _ => "已提交，等待成交确认",
    }
}

fn confirm_status_label(status: HedgeConfirmStatus, mode: &str) -> &'static str {
    match status {
        HedgeConfirmStatus::Submitted => submitted_label(mode),
        HedgeConfirmStatus::Replayed => "已提交过，返回既有执行结果",
        HedgeConfirmStatus::LongLegFailed => "第一执行腿提交失败",
        HedgeConfirmStatus::ValuationMissing => "执行估值缺少价格，需要人工复核",
        HedgeConfirmStatus::FirstLegPartialUnwindAttempted => "部分成交，已尝试反向平衡",
        HedgeConfirmStatus::FirstLegPartialUnwindFailed => "部分成交，反向平衡提交失败",
        HedgeConfirmStatus::FirstLegPartialWaitingFillQty => "部分成交，等待成交数量",
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindAttempted => "复检拒绝，已尝试反向平衡",
        HedgeConfirmStatus::HedgeRecheckBlockedUnwindFailed => "复检拒绝，反向平衡提交失败",
        HedgeConfirmStatus::HedgeBrokenUnwindAttempted => "对冲破裂，已尝试反向平衡",
        HedgeConfirmStatus::HedgeBrokenUnwindFailed => "对冲破裂，反向平衡提交失败",
        HedgeConfirmStatus::Unknown => "提交返回异常",
    }
}

pub(super) fn confirm_outcome_label(response: &HedgeConfirmResponse, mode: &str) -> String {
    let mut parts = vec![confirm_status_label(response.status, mode).to_owned()];
    if let Some(run) = response.execution_run.as_ref() {
        parts.push(format!("Run {}", run.run_id));
    }
    parts.push(format!("Idempotency {}", response.idempotency_key));
    parts.join(" · ")
}

pub(super) fn confirm_response_problem(response: &HedgeConfirmResponse) -> Option<ApiProblem> {
    if let Some(problem) = response.problem.clone() {
        return Some(problem);
    }
    if let Some(problem) = response
        .partial_outcome
        .as_ref()
        .and_then(confirm_partial_outcome_problem)
    {
        return Some(problem);
    }
    if let Some(error) = response.error.as_ref() {
        return Some(ApiProblem::new(
            codes::HEDGE_CONFIRM_NOT_HEDGED,
            error.clone(),
        ));
    }
    if confirm_response_succeeded(response) {
        None
    } else {
        Some(ApiProblem::new(
            codes::HEDGE_CONFIRM_NOT_HEDGED,
            format!("hedge confirm ended as {}", response.status),
        ))
    }
}

fn confirm_partial_outcome_problem(
    outcome: &shared_types::HedgeConfirmPartialOutcome,
) -> Option<ApiProblem> {
    outcome
        .unwind_problem
        .clone()
        .or_else(|| outcome.primary_problem.clone())
}

fn confirm_response_succeeded(response: &HedgeConfirmResponse) -> bool {
    matches!(
        response.execution_run.as_ref().map(|run| run.state),
        Some(ExecutionRunState::Hedged | ExecutionRunState::SecondLegSubmitted)
    )
}
