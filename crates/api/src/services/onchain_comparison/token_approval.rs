use std::collections::VecDeque;

use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    OnchainComparisonQuality, OnchainTokenApprovalBuildRequest, OnchainTokenApprovalBuildResponse,
    OnchainTokenApprovalRunStatus, OnchainTokenApprovalRunsResponse,
    OnchainTokenApprovalSubmitRequest, OnchainTokenApprovalSubmitResponse,
};

use crate::services::onchain_token_approval_store::{ApprovalClaimError, ClaimedTokenApproval};
use crate::state::AppState;

use super::{allowance, execution_build, execution_submit};

mod receipts;
mod reconciliation;

pub(crate) use receipts::refresh as refresh_receipts;

const APPROVAL_VALIDITY_MS: i64 = 120_000;
const MAX_STORED_RUNS: usize = 64;

struct ApprovalRunContext<'a> {
    state: &'a AppState,
    run_id: &'a str,
    started_at_ms: i64,
    claimed: &'a ClaimedTokenApproval,
}

pub(crate) async fn build(
    state: &AppState,
    request: &OnchainTokenApprovalBuildRequest,
) -> Result<OnchainTokenApprovalBuildResponse, AppError> {
    let snapshot = state.onchain_monitor().snapshot();
    validate_build_request(&snapshot, request)?;
    let config = snapshot.config.clone();
    let required_amount_raw =
        execution_build::monitored_input_amount(&snapshot, request.direction)?;
    let plan = allowance::build_plan(state, &config, request.direction, &required_amount_raw)
        .await
        .map_err(|problem| conflict("ONCHAIN_TOKEN_APPROVAL_BUILD_REJECTED", problem))?;
    let built_at_ms = common::time::now_ms();
    let approval_required = !plan.transactions.is_empty();
    let readiness_problem = state
        .onchain_token_approval_runs()
        .readiness()
        .and_then(|_| execution_submit::readiness(state, &config))
        .err();
    let submit_ready = approval_required && readiness_problem.is_none();
    let blockers = approval_blockers(approval_required, readiness_problem);
    let response = OnchainTokenApprovalBuildResponse {
        approval_id: format!("onchain-approval-{}", uuid::Uuid::new_v4()),
        direction: request.direction,
        provider: config.provider.clone(),
        chain: config.chain.clone(),
        wallet_address: config.wallet_address.clone(),
        token_address: plan.token_address,
        token_symbol: plan.token_symbol,
        token_decimals: plan.token_decimals,
        spender: plan.spender,
        required_amount_raw: plan.required_amount_raw,
        current_allowance_raw: plan.current_allowance_raw,
        transactions: plan.transactions,
        built_at_ms,
        valid_until_ms: built_at_ms.saturating_add(APPROVAL_VALIDITY_MS),
        official_docs_url: plan.official_docs_url,
        approval_required,
        submit_ready,
        blockers,
    };
    if approval_required {
        state
            .onchain_token_approval_builds()
            .insert(response.clone(), config, built_at_ms);
    }
    Ok(response)
}

pub(crate) fn recent_runs(state: &AppState, limit: usize) -> OnchainTokenApprovalRunsResponse {
    let rows = state
        .onchain_token_approval_runs()
        .recent(limit.min(MAX_STORED_RUNS));
    OnchainTokenApprovalRunsResponse {
        cost_owners: rows.iter().filter_map(|row| state.onchain_execution_run_store()
            .approval_cost_owner(&row.run_id).map(|id| (row.run_id.clone(), id))).collect(),
        rows,
        observed_at_ms: common::time::now_ms(),
    }
}

pub(crate) async fn submit(
    state: &AppState,
    request: &OnchainTokenApprovalSubmitRequest,
) -> Result<OnchainTokenApprovalSubmitResponse, AppError> {
    if let Some(existing) = replayed_run(state, &request.approval_id) {
        if existing.fee_checks_exhausted {
            return state
                .onchain_token_approval_runs()
                .resume_checks(&existing.run_id)
                .map_err(|problem| conflict("ONCHAIN_TOKEN_APPROVAL_RECHECK_FAILED", problem));
        }
        return Ok(existing);
    }
    let started_at_ms = common::time::now_ms();
    let run_id = format!("onchain-approval-run-{}", uuid::Uuid::new_v4());
    let claimed = match state.onchain_token_approval_builds().claim(
        &request.approval_id,
        &run_id,
        started_at_ms,
    ) {
        Ok(claimed) => claimed,
        Err(ApprovalClaimError::AlreadyClaimed(existing_run_id)) => {
            if let Some(existing) = state.onchain_token_approval_runs().record(&existing_run_id) {
                return Ok(existing.response);
            }
            return Err(claim_error(ApprovalClaimError::AlreadyClaimed(
                existing_run_id,
            )));
        }
        Err(error) => return Err(claim_error(error)),
    };
    if state.onchain_monitor().snapshot().config != claimed.config {
        state
            .onchain_token_approval_builds()
            .release(&request.approval_id, &run_id);
        return Err(conflict(
            "ONCHAIN_TOKEN_APPROVAL_CONFIG_CHANGED",
            "链上配置已变化，请重新构建授权计划",
        ));
    }
    execution_submit::readiness(state, &claimed.config).map_err(|problem| {
        state
            .onchain_token_approval_builds()
            .release(&request.approval_id, &run_id);
        conflict("ONCHAIN_TOKEN_APPROVAL_NOT_READY", problem)
    })?;
    let context = ApprovalRunContext {
        state,
        run_id: &run_id,
        started_at_ms,
        claimed: &claimed,
    };
    let response = submit_claimed(context).await;
    state
        .onchain_token_approval_builds()
        .finish(&request.approval_id, &run_id);
    record_run(state, response)
        .map_err(|problem| conflict("ONCHAIN_TOKEN_APPROVAL_NOT_DURABLE", problem))
}

fn validate_build_request(
    snapshot: &shared_types::OnchainComparisonSnapshot,
    request: &OnchainTokenApprovalBuildRequest,
) -> Result<(), AppError> {
    if !snapshot.config.enabled
        || !matches!(
            snapshot.quality,
            OnchainComparisonQuality::Fresh | OnchainComparisonQuality::LowLiquidity
        )
    {
        return Err(conflict(
            "ONCHAIN_TOKEN_APPROVAL_OPPORTUNITY_STALE",
            "当前链上/CEX 机会已失效，不能沿用旧报价构建授权",
        ));
    }
    if snapshot.quote_observed_at_ms != Some(request.expected_quote_observed_at_ms) {
        return Err(conflict(
            "ONCHAIN_TOKEN_APPROVAL_SNAPSHOT_CHANGED",
            "链上报价已经变化，请按最新快照重新构建授权",
        ));
    }
    let minimum_bps = snapshot.config.spread_alert.min_net_spread_bps.max(0.0);
    let profitable = snapshot.comparisons.iter().any(|row| {
        row.direction == request.direction
            && row.net_spread_bps > 0.0
            && row.net_spread_bps >= minimum_bps
    });
    if !profitable {
        return Err(conflict(
            "ONCHAIN_TOKEN_APPROVAL_DIRECTION_NOT_PROFITABLE",
            "当前方向已不满足费后净收益门槛，不再建议新增代币授权",
        ));
    }
    Ok(())
}

fn approval_blockers(approval_required: bool, readiness_problem: Option<String>) -> Vec<String> {
    if !approval_required {
        return vec!["当前 allowance 已足够，请重新构建双腿交易计划".to_owned()];
    }
    readiness_problem.into_iter().collect()
}

async fn submit_claimed(context: ApprovalRunContext<'_>) -> OnchainTokenApprovalSubmitResponse {
    let mut response = initial_response(&context);
    match context
        .state
        .onchain_token_approval_runs()
        .create(context.claimed.response.clone(), response.clone())
    {
        Ok(existing) if existing.run_id != response.run_id => return existing,
        Ok(_) => (),
        Err(problem) => return failed(response, problem),
    }
    let mut transactions = VecDeque::from(context.claimed.response.transactions.clone());
    while let Some(transaction) = transactions.pop_front() {
        let prepared = match execution_submit::prepare_independent_chain_transaction(
            context.state,
            &context.claimed.config,
            &transaction,
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(problem) => return failed(response, format!("授权交易签名失败：{problem}")),
        };
        if let Err(problem) = context.state.onchain_token_approval_runs().intent(
            &response.run_id,
            response.transaction_ids.len(),
            prepared.transaction_id(),
            common::time::now_ms(),
        ) {
            return failed(response, problem);
        }
        match execution_submit::broadcast_independent_chain_transaction(prepared.clone()).await {
            execution_submit::IndependentChainOutcome::Confirmed { transaction_id } => {
                push_transaction_id(&mut response, transaction_id.clone());
                if let Err(problem) =
                    receipts::confirmed(context.state, &response.run_id, &transaction_id).await
                {
                    response.status = OnchainTokenApprovalRunStatus::FinalityUnresolved;
                    response.problem = Some(problem);
                    response.message = "授权回执待核验，后续步骤未提交".into();
                    return response;
                }
            }
            execution_submit::IndependentChainOutcome::Rejected {
                transaction_id,
                problem,
            } => {
                push_transaction_id(&mut response, transaction_id);
                return failed(response, format!("授权交易被链上拒绝：{problem}"));
            }
            execution_submit::IndependentChainOutcome::Pending {
                transaction_id,
                problem,
            } => {
                push_transaction_id(&mut response, transaction_id.clone());
                response.status = OnchainTokenApprovalRunStatus::AwaitingFinality;
                response.message = "授权交易已广播，正在等待链上确认".to_owned();
                response.problem = Some(problem);
                response.updated_at_ms = common::time::now_ms();
                if let Err(problem) = record_run(context.state, response.clone()) {
                    return failed(response, problem);
                }
                reconciliation::spawn(reconciliation::PendingTokenApproval {
                    state: context.state.clone(),
                    config: context.claimed.config.clone(),
                    remaining: transactions,
                    prepared,
                    transaction_id,
                    response: response.clone(),
                });
                return response;
            }
        }
    }
    completed(response)
}

fn initial_response(context: &ApprovalRunContext<'_>) -> OnchainTokenApprovalSubmitResponse {
    OnchainTokenApprovalSubmitResponse {
        run_id: context.run_id.to_owned(),
        approval_id: context.claimed.response.approval_id.clone(),
        status: OnchainTokenApprovalRunStatus::AwaitingFinality,
        transaction_ids: Vec::new(),
        message: "正在提交独立 ERC-20 授权交易".to_owned(),
        problem: None,
        started_at_ms: context.started_at_ms,
        updated_at_ms: context.started_at_ms,
        fee_receipts: Vec::new(),
        fee_checks_exhausted: false,
    }
}

pub(super) fn completed(
    mut response: OnchainTokenApprovalSubmitResponse,
) -> OnchainTokenApprovalSubmitResponse {
    response.status = OnchainTokenApprovalRunStatus::Completed;
    response.message = "ERC-20 授权已在链上确认；请重新构建双腿交易计划".to_owned();
    response.problem = None;
    response.updated_at_ms = common::time::now_ms();
    response
}

pub(super) fn failed(
    mut response: OnchainTokenApprovalSubmitResponse,
    problem: String,
) -> OnchainTokenApprovalSubmitResponse {
    response.status = OnchainTokenApprovalRunStatus::Failed;
    response.message = "ERC-20 授权未完成；没有提交 CEX 订单".to_owned();
    response.problem = Some(problem);
    response.updated_at_ms = common::time::now_ms();
    response
}

pub(super) fn push_transaction_id(
    response: &mut OnchainTokenApprovalSubmitResponse,
    transaction_id: String,
) {
    if !response
        .transaction_ids
        .iter()
        .any(|row| row == &transaction_id)
    {
        response.transaction_ids.push(transaction_id);
    }
    response.updated_at_ms = common::time::now_ms();
}

fn replayed_run(state: &AppState, approval_id: &str) -> Option<OnchainTokenApprovalSubmitResponse> {
    state.onchain_token_approval_runs().by_approval(approval_id)
}

pub(super) fn record_run(
    state: &AppState,
    response: OnchainTokenApprovalSubmitResponse,
) -> Result<OnchainTokenApprovalSubmitResponse, String> {
    state.onchain_token_approval_runs().response(response)
}

fn claim_error(error: ApprovalClaimError) -> AppError {
    match error {
        ApprovalClaimError::Missing => conflict(
            "ONCHAIN_TOKEN_APPROVAL_NOT_FOUND",
            "授权计划不存在或已完成，请重新构建",
        ),
        ApprovalClaimError::Expired => conflict(
            "ONCHAIN_TOKEN_APPROVAL_EXPIRED",
            "授权计划已过期，请重新构建",
        ),
        ApprovalClaimError::AlreadyClaimed(run_id) => conflict(
            "ONCHAIN_TOKEN_APPROVAL_ALREADY_SUBMITTED",
            format!("授权计划已由任务 {run_id} 领取"),
        ),
    }
}

fn conflict(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::CONFLICT, code, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_transaction_ids_are_deduplicated() {
        let mut response = OnchainTokenApprovalSubmitResponse {
            run_id: "run".to_owned(),
            approval_id: "approval".to_owned(),
            status: OnchainTokenApprovalRunStatus::AwaitingFinality,
            transaction_ids: Vec::new(),
            message: String::new(),
            problem: None,
            started_at_ms: 1,
            updated_at_ms: 1,
            fee_receipts: Vec::new(),
            fee_checks_exhausted: false,
        };
        push_transaction_id(&mut response, "0xabc".to_owned());
        push_transaction_id(&mut response, "0xabc".to_owned());
        assert_eq!(response.transaction_ids, vec!["0xabc"]);
    }

    #[test]
    fn allowance_sufficient_plan_points_back_to_trade_build() {
        assert_eq!(
            approval_blockers(false, None),
            vec!["当前 allowance 已足够，请重新构建双腿交易计划"]
        );
    }
}
