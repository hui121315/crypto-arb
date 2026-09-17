use shared_types::{OnchainCexOrderPlan, OnchainExecutionRunStatus, OrderRecord};

use crate::services::onchain_execution_build_store::ClaimedOnchainBuild;
use crate::services::onchain_execution_run_store::{
    OnchainExecutionStage, OnchainQuoteConversionAttempt, PendingOnchainExecution,
};
use crate::state::AppState;

use super::providers::{self, ChainSubmissionOutcome};

const FIRST_RECHECK_DELAY_MS: u64 = 2_000;
const RECHECK_INTERVAL_MS: u64 = 5_000;
const MAX_RECHECKS: usize = 24;

pub(super) struct PendingChainExecution {
    pub(super) state: AppState,
    pub(super) run_id: String,
    pub(super) started_at_ms: i64,
    pub(super) claimed: ClaimedOnchainBuild,
    pub(super) cex_plan: OnchainCexOrderPlan,
    pub(super) cex_record: OrderRecord,
    pub(super) quote_conversion_plan: Option<shared_types::OnchainQuoteConversionOrderPlan>,
    pub(super) quote_conversion_attempts: Vec<OnchainQuoteConversionAttempt>,
    pub(super) pending: shared_types::OnchainExecutionSubmitResponse,
    pub(super) transaction_id: String,
}

pub(super) fn spawn(context: PendingChainExecution) {
    tokio::spawn(reconcile(context));
}

pub(super) fn restore(state: &AppState, checkpoints: Vec<PendingOnchainExecution>) {
    if state.onchain_execution_run_store().readiness().is_err() {
        return;
    }
    for checkpoint in checkpoints {
        let current = state
            .onchain_execution_runs()
            .get(&checkpoint.response.run_id)
            .map(|entry| entry.value().clone())
            .unwrap_or_else(|| checkpoint.response.clone());
        match checkpoint.stage {
            OnchainExecutionStage::Prepared => {
                let mut response = current;
                response.status = OnchainExecutionRunStatus::Failed;
                response.message = "重启前仅保存了计划，尚未提交任何资金动作；请重新构建".into();
                super::record_run(state, response);
            }
            OnchainExecutionStage::CexActionSubmitting => {
                tokio::spawn(recover_cex_intent(state.clone(), current, checkpoint));
            }
            OnchainExecutionStage::ChainBroadcasting
            | OnchainExecutionStage::AwaitingChainFinality => {
                if let Some(context) = restored_chain_context(state, current, checkpoint) {
                    spawn(context);
                }
            }
            OnchainExecutionStage::QuoteConversionFilled
            | OnchainExecutionStage::PrimaryCexFilled => {
                tokio::spawn(recover_prebroadcast(state.clone(), current, checkpoint));
            }
        }
    }
}

async fn recover_cex_intent(
    state: AppState,
    current: shared_types::OnchainExecutionSubmitResponse,
    checkpoint: PendingOnchainExecution,
) {
    let mut response = super::interrupted_response(
        current,
        "后端在 CEX 资金动作确认前中断，禁止重发或盲目补偿".into(),
    );
    if let Some(plan) = checkpoint.active_cex_order {
        let record = state
            .trading_service()
            .get_order_by_client_order_id(&plan.client_order_id)
            .or(checkpoint.active_cex_record);
        let record = match record {
            Some(record) => state
                .trading_service()
                .refresh_order_state(&record.intent.id)
                .await
                .ok()
                .flatten()
                .or(Some(record)),
            None => None,
        };
        response.message = format!(
            "已停止自动续单；核对 {} {} 客户端订单 {}，不得按新订单重试",
            plan.venue, plan.native_symbol, plan.client_order_id
        );
        if let Some(record) = record {
            if let Err(problem) = state
                .onchain_execution_run_store()
                .append_cex_observation(&response.run_id, &record)
            {
                response.problem = Some(problem);
                publish_terminal((state, response)).await;
                return;
            }
            response.problem = Some(format!(
                "已读取订单 {}，状态 {:?}；需核对本单与此前各腿的实际成交后恢复",
                record.intent.id, record.state
            ));
            if plan.client_order_id == checkpoint.primary_plan.client_order_id {
                response.cex_order_id = Some(record.intent.id.clone());
                response.cex_order_state = Some(record.state);
                response.cex_filled_quantity = record.filled_quantity;
            }
        }
    }
    publish_terminal((state, response)).await;
}

fn restored_chain_context(
    state: &AppState,
    current: shared_types::OnchainExecutionSubmitResponse,
    checkpoint: PendingOnchainExecution,
) -> Option<PendingChainExecution> {
    let quote_conversion_attempts = restored_conversion_attempts(&checkpoint);
    let stored_primary = checkpoint.primary_record?;
    let transaction_id = checkpoint.transaction_id?;
    let cex_record = state
        .trading_service()
        .get_order(&stored_primary.intent.id)
        .unwrap_or(stored_primary);
    Some(PendingChainExecution {
        state: state.clone(),
        run_id: current.run_id.clone(),
        started_at_ms: current.started_at_ms,
        claimed: ClaimedOnchainBuild {
            response: checkpoint.build,
            config: checkpoint.config,
        },
        cex_plan: checkpoint.primary_plan,
        cex_record,
        quote_conversion_plan: checkpoint.quote_conversion_plan,
        quote_conversion_attempts,
        pending: current,
        transaction_id,
    })
}

async fn recover_prebroadcast(
    state: AppState,
    current: shared_types::OnchainExecutionSubmitResponse,
    checkpoint: PendingOnchainExecution,
) {
    let attempts = restored_conversion_attempts(&checkpoint);
    let Some(conversion) = checkpoint.quote_conversion_plan.clone() else {
        publish_terminal((state, invalid_prebroadcast_checkpoint(current))).await;
        return;
    };
    let claimed = ClaimedOnchainBuild {
        response: checkpoint.build.clone(),
        config: checkpoint.config.clone(),
    };
    let primary_record = checkpoint.primary_record.or_else(|| {
        state
            .trading_service()
            .get_order_by_client_order_id(&checkpoint.primary_plan.client_order_id)
    });
    let primary_record = match primary_record {
        Some(record) => Some(super::settle_cex_order(&state, record).await),
        None => None,
    };
    let response = super::three_leg_submit::recover_prebroadcast(
        super::three_leg_submit::RecoveredPrebroadcast {
            state: &state,
            response: super::ResponseContext {
                run_id: &current.run_id,
                build: &claimed.response,
                started_at_ms: current.started_at_ms,
            },
            claimed: &claimed,
            primary: primary_record
                .as_ref()
                .map(|record| (&checkpoint.primary_plan, record)),
            conversion: &conversion,
            attempts,
        },
    )
    .await;
    publish_terminal((state, response)).await;
}

fn invalid_prebroadcast_checkpoint(
    mut response: shared_types::OnchainExecutionSubmitResponse,
) -> shared_types::OnchainExecutionSubmitResponse {
    response.status = OnchainExecutionRunStatus::Exposed;
    response.message = "链上广播前恢复检查点不完整，已停止自动执行".to_owned();
    response.problem = Some("恢复日志缺少 Quote 换汇计划，请人工核对账户暴露".to_owned());
    response.updated_at_ms = common::time::now_ms();
    response
}

fn restored_conversion_attempts(
    checkpoint: &PendingOnchainExecution,
) -> Vec<OnchainQuoteConversionAttempt> {
    if !checkpoint.quote_conversion_attempts.is_empty() {
        return checkpoint.quote_conversion_attempts.clone();
    }
    let Some(conversion) = checkpoint.quote_conversion_plan.as_ref() else {
        return Vec::new();
    };
    checkpoint
        .quote_conversion_records
        .iter()
        .cloned()
        .map(|record| OnchainQuoteConversionAttempt {
            plan: conversion.order.clone(),
            record,
        })
        .collect()
}

async fn reconcile(mut context: PendingChainExecution) {
    tokio::time::sleep(std::time::Duration::from_millis(FIRST_RECHECK_DELAY_MS)).await;
    for attempt in 0..MAX_RECHECKS {
        match providers::recheck_transaction(
            &context.state,
            &context.claimed.config,
            &context.transaction_id,
        )
        .await
        {
            ChainSubmissionOutcome::Confirmed { transaction_id } => {
                publish_terminal(completed(&context, transaction_id).await).await;
                return;
            }
            ChainSubmissionOutcome::Rejected {
                transaction_id,
                problem,
            } => {
                publish_terminal(rejected(&context, transaction_id, problem).await).await;
                return;
            }
            ChainSubmissionOutcome::Pending {
                transaction_id,
                problem,
            } => update_pending(&mut context, transaction_id, problem),
        }
        if attempt + 1 < MAX_RECHECKS {
            tokio::time::sleep(std::time::Duration::from_millis(RECHECK_INTERVAL_MS)).await;
        }
    }
    context.pending.message =
        "链上终态超过两分钟仍未确认；系统保留 CEX 对冲并等待人工核验".to_owned();
    context.pending.status = OnchainExecutionRunStatus::FinalityUnresolved;
    context.pending.problem = Some(format!(
        "{}；后台终态追踪已达到本轮上限",
        context.pending.problem.as_deref().unwrap_or("链上终态未知")
    ));
    context.pending.updated_at_ms = common::time::now_ms();
    publish_terminal((context.state.clone(), context.pending)).await;
}

fn update_pending(context: &mut PendingChainExecution, transaction_id: String, problem: String) {
    context.transaction_id = transaction_id.clone();
    context.pending.chain_transaction_id = Some(transaction_id);
    context.pending.problem = Some(problem);
    context.pending.updated_at_ms = common::time::now_ms();
    if let Err(problem) = super::record_pending(&context.state, context) {
        context.pending = super::interrupted_response(context.pending.clone(), problem);
    }
}

async fn completed(
    context: &PendingChainExecution,
    transaction_id: String,
) -> (AppState, shared_types::OnchainExecutionSubmitResponse) {
    let response_context = super::ResponseContext {
        run_id: &context.run_id,
        build: &context.claimed.response,
        started_at_ms: context.started_at_ms,
    };
    let response = match context.quote_conversion_plan.as_ref() {
        Some(conversion) => {
            super::three_leg_submit::complete_recovered(
                super::three_leg_submit::RecoveredThreeLeg {
                    state: &context.state,
                    response: response_context,
                    claimed: &context.claimed,
                    primary: &context.cex_plan,
                    primary_record: &context.cex_record,
                    conversion,
                    attempts: context.quote_conversion_attempts.clone(),
                },
                transaction_id,
            )
            .await
        }
        None => super::response(
            response_context,
            Some(&context.cex_record),
            super::RunOutcome {
                status: OnchainExecutionRunStatus::Completed,
                chain_transaction_id: Some(transaction_id),
                remaining_exposure_usd: 0.0,
                message: "双腿均已确认完成".to_owned(),
                problem: None,
            },
        ),
    };
    (context.state.clone(), response)
}

async fn rejected(
    context: &PendingChainExecution,
    transaction_id: String,
    chain_problem: String,
) -> (AppState, shared_types::OnchainExecutionSubmitResponse) {
    if let Some(conversion) = context.quote_conversion_plan.as_ref() {
        let response = super::three_leg_submit::reject_recovered(
            super::three_leg_submit::RecoveredThreeLeg {
                state: &context.state,
                response: super::ResponseContext {
                    run_id: &context.run_id,
                    build: &context.claimed.response,
                    started_at_ms: context.started_at_ms,
                },
                claimed: &context.claimed,
                primary: &context.cex_plan,
                primary_record: &context.cex_record,
                conversion,
                attempts: context.quote_conversion_attempts.clone(),
            },
            transaction_id,
            chain_problem,
        )
        .await;
        return (context.state.clone(), response);
    }
    let mut response = super::compensate_or_expose(
        super::CompensationContext {
            state: &context.state,
            response: super::ResponseContext {
                run_id: &context.run_id,
                build: &context.claimed.response,
                started_at_ms: context.started_at_ms,
            },
            claimed: &context.claimed,
            plan: &context.cex_plan,
            reason: "链上交易明确失败，已启动 CEX 反向补偿",
        },
        context.cex_record.clone(),
    )
    .await;
    super::attach_chain_transaction(
        &mut response,
        &context.claimed.response,
        transaction_id,
        shared_types::OnchainExecutionLegStatus::Rejected,
    );
    response.problem = Some(match response.problem.take() {
        Some(compensation_problem) => {
            format!("{chain_problem}；{compensation_problem}")
        }
        None => chain_problem,
    });
    (context.state.clone(), response)
}

async fn publish_terminal(
    (state, response): (AppState, shared_types::OnchainExecutionSubmitResponse),
) {
    let response = super::record_run(&state, response);
    super::emit_webhook(&state, &response).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciliation_window_is_bounded_to_about_two_minutes() {
        let elapsed = FIRST_RECHECK_DELAY_MS + RECHECK_INTERVAL_MS * (MAX_RECHECKS as u64 - 1);
        assert!((110_000..=120_000).contains(&elapsed));
    }
}
