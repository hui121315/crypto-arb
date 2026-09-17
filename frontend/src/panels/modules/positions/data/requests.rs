//! 平仓 / 补偿 / kill-switch 的请求构造与网络任务。
//!
//! 这里把 [`super::actions`] hooks 收集到的输入转成后端请求 DTO，并封装实际的
//! `ApiClient` 调用任务。所有快照版本/补偿候选缺失都 fail-closed 成 `ApiProblem`，
//! 绝不在缺证据时下单。

#[path = "requests/close.rs"]
mod close;

use crate::api::rest::{ApiClient, ApiError, MutationRequestContext};

use super::actions::{
    CloseRunCompensationCancelInput, CloseRunCompensationInput, CloseRunManualTerminalInput,
};
use shared_types::{
    ApiProblem, CloseRun, CloseRunCompensationRequest, CloseRunManualTerminalRequest,
    CloseRunNextActionKind, CloseRunStatus, CloseRunUnwindLegEvidence, CloseRunUnwindPlanStatus,
    KillSwitchRequest, KillSwitchResponse, LiveOrderState, OrderSource,
};

pub(super) type CloseRequestResult<T> = Result<T, Box<ApiProblem>>;

use close::close_run_snapshot_version;
pub(in crate::panels::modules::positions) use close::{
    cancel_close_run_compensation_task, close_all_positions_task, close_all_request,
    close_execution_scope, close_position_pair_task, close_position_request, close_position_task,
    portfolio_close_requires_live, submit_close_run_compensation_task,
    submit_close_run_manual_terminal_task,
};

pub(in crate::panels::modules::positions) fn close_run_compensation_request(
    input: &CloseRunCompensationInput,
) -> CloseRequestResult<CloseRunCompensationRequest> {
    let candidate = compensation_candidate(&input.run, input.candidate_index)?;
    let target_quantity = compensation_target_quantity(candidate)?;
    let limit_price = compensation_limit_price(candidate)?;
    Ok(CloseRunCompensationRequest {
        confirmation_phrase: input.confirmation_phrase.trim().to_owned(),
        snapshot_version: Some(close_run_snapshot_version(&input.run)?),
        candidate_index: Some(input.candidate_index),
        target_quantity: Some(target_quantity),
        limit_price: Some(limit_price),
        reason: Some("positions.close_run_compensation".to_owned()),
    })
}

pub(in crate::panels::modules::positions) fn close_run_compensation_cancel_order_id(
    input: &CloseRunCompensationCancelInput,
) -> CloseRequestResult<String> {
    let order_id = input.order_id.trim();
    if order_id.is_empty() {
        return Err(close_compensation_cancel_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "补偿撤单缺少订单 ID",
        ));
    }
    if input.run.status != CloseRunStatus::CompensationSubmitted {
        return Err(close_compensation_cancel_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "只有补偿中 CloseRun 可以撤销补偿单",
        ));
    }
    let belongs_to_run = input
        .run
        .unwind_plan
        .as_ref()
        .filter(|plan| plan.status == CloseRunUnwindPlanStatus::CompensationSubmitted)
        .is_some_and(|plan| {
            plan.compensation_attempts
                .iter()
                .any(|attempt| compensation_attempt_cancel_order_matches(attempt, order_id))
        });
    if !belongs_to_run {
        return Err(close_compensation_cancel_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "补偿撤单订单不属于当前 CloseRun",
        ));
    }
    Ok(order_id.to_owned())
}

pub(in crate::panels::modules::positions) fn close_run_manual_terminal_request(
    input: &CloseRunManualTerminalInput,
) -> CloseRequestResult<CloseRunManualTerminalRequest> {
    validate_manual_terminal_run(&input.run)?;
    let reason = required_manual_terminal_reason(&input.reason)?;
    let manual_handling_cost_usd = parse_manual_handling_cost(&input.manual_handling_cost_usd)?;
    Ok(CloseRunManualTerminalRequest {
        confirmation_phrase: input.confirmation_phrase.trim().to_owned(),
        snapshot_version: Some(close_run_snapshot_version(&input.run)?),
        reason,
        manual_handling_cost_usd,
        evidence: manual_terminal_evidence_items(&input.evidence),
    })
}

fn validate_manual_terminal_run(run: &CloseRun) -> CloseRequestResult<()> {
    let has_manual_action = run.unwind_plan.as_ref().is_some_and(|plan| {
        plan.next_actions
            .iter()
            .any(|action| action.kind == CloseRunNextActionKind::ManualIncidentReview)
    });
    if run.status == CloseRunStatus::CompensationFailed && has_manual_action {
        return Ok(());
    }
    Err(close_manual_terminal_problem(
        shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
        "只有补偿失败且进入人工复核的 CloseRun 可以人工终结",
    ))
}

fn required_manual_terminal_reason(reason: &str) -> CloseRequestResult<String> {
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(close_manual_terminal_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "人工终结必须填写原因",
        ));
    }
    Ok(reason.to_owned())
}

fn parse_manual_handling_cost(value: &str) -> CloseRequestResult<Option<f64>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let parsed = value.parse::<f64>().map_err(|_| {
        close_manual_terminal_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "人工处理成本必须是 USD 数字",
        )
    })?;
    if parsed.is_finite() && parsed >= 0.0 {
        Ok(Some(parsed))
    } else {
        Err(close_manual_terminal_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "人工处理成本必须是非负有限 USD 数字",
        ))
    }
}

fn manual_terminal_evidence_items(evidence: &str) -> Vec<String> {
    evidence
        .split(['\n', ',', ';'])
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

fn compensation_attempt_cancel_order_matches(
    attempt: &shared_types::CloseRunCompensationAttempt,
    order_id: &str,
) -> bool {
    matches!(
        attempt.status,
        shared_types::CloseLegStatus::Submitted | shared_types::CloseLegStatus::Accepted
    ) && attempt.order.as_ref().is_some_and(|order| {
        order.intent.id == order_id
            && order.intent.source == OrderSource::CloseRunCompensation
            && matches!(
                order.state,
                LiveOrderState::Submitted | LiveOrderState::Accepted | LiveOrderState::Unknown
            )
    })
}

fn compensation_candidate(
    run: &CloseRun,
    candidate_index: usize,
) -> CloseRequestResult<&CloseRunUnwindLegEvidence> {
    run.unwind_plan
        .as_ref()
        .and_then(|plan| plan.compensation_candidates.get(candidate_index))
        .ok_or_else(|| {
            close_compensation_problem(
                shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
                "平仓事故缺少可补偿候选腿",
            )
        })
}

fn compensation_target_quantity(candidate: &CloseRunUnwindLegEvidence) -> CloseRequestResult<f64> {
    let target = candidate
        .confirmed_quantity
        .unwrap_or(candidate.target_quantity)
        .abs();
    if target.is_finite() && target > f64::EPSILON {
        Ok(target)
    } else {
        Err(close_compensation_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "补偿候选腿缺少有效成交数量",
        ))
    }
}

fn compensation_limit_price(candidate: &CloseRunUnwindLegEvidence) -> CloseRequestResult<f64> {
    if candidate.mark_price.is_finite() && candidate.mark_price > 0.0 {
        Ok(candidate.mark_price)
    } else {
        Err(close_compensation_problem(
            shared_types::problem::codes::CLOSE_RUN_REQUEST_INVALID,
            "补偿候选腿缺少有效限价",
        ))
    }
}

fn close_compensation_problem(code: &'static str, message: &'static str) -> Box<ApiProblem> {
    Box::new(ApiProblem::new(code, message).with_source("positions.close_run_compensation"))
}

fn close_compensation_cancel_problem(code: &'static str, message: &'static str) -> Box<ApiProblem> {
    Box::new(ApiProblem::new(code, message).with_source("positions.close_run_compensation_cancel"))
}

fn close_manual_terminal_problem(code: &'static str, message: &'static str) -> Box<ApiProblem> {
    Box::new(ApiProblem::new(code, message).with_source("positions.close_run_manual_terminal"))
}

pub(in crate::panels::modules::positions) async fn set_kill_switch_task(
    client: ApiClient,
    request: KillSwitchRequest,
    context: MutationRequestContext,
) -> Result<KillSwitchResponse, ApiError> {
    client
        .set_kill_switch_with_context(&request, &context)
        .await
}
