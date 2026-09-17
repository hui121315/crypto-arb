use std::collections::VecDeque;

use shared_types::{
    OnchainComparisonConfig, OnchainTokenApprovalRunStatus, OnchainTokenApprovalSubmitResponse,
    OnchainUnsignedTransaction,
};

use crate::state::AppState;

use super::super::execution_submit::{self, IndependentChainOutcome, IndependentChainSubmission};

const FIRST_RECHECK_DELAY_MS: u64 = 2_000;
const RECHECK_INTERVAL_MS: u64 = 5_000;
const MAX_RECHECKS: usize = 24;

pub(super) struct PendingTokenApproval {
    pub(super) state: AppState,
    pub(super) config: OnchainComparisonConfig,
    pub(super) remaining: VecDeque<OnchainUnsignedTransaction>,
    pub(super) prepared: IndependentChainSubmission,
    pub(super) transaction_id: String,
    pub(super) response: OnchainTokenApprovalSubmitResponse,
}

pub(super) fn spawn(context: PendingTokenApproval) {
    tokio::spawn(reconcile(context));
}

async fn reconcile(mut context: PendingTokenApproval) {
    tokio::time::sleep(std::time::Duration::from_millis(FIRST_RECHECK_DELAY_MS)).await;
    let mut attempts = 0;
    loop {
        match execution_submit::recheck_independent_chain_transaction(
            &context.prepared,
            &context.transaction_id,
        )
        .await
        {
            IndependentChainOutcome::Confirmed { transaction_id } => {
                super::push_transaction_id(&mut context.response, transaction_id.clone());
                if let Err(problem) = super::receipts::confirmed(
                    &context.state,
                    &context.response.run_id,
                    &transaction_id,
                )
                .await
                {
                    context.response.status = OnchainTokenApprovalRunStatus::FinalityUnresolved;
                    context.response.problem = Some(problem);
                    context.response.message = "授权回执待核验，后续步骤未提交".into();
                    publish(&context.state, context.response);
                    return;
                }
                match submit_remaining(&mut context).await {
                    RemainingOutcome::Completed => {
                        publish(&context.state, super::completed(context.response));
                        return;
                    }
                    RemainingOutcome::Failed(problem) => {
                        publish(&context.state, super::failed(context.response, problem));
                        return;
                    }
                    RemainingOutcome::Pending => attempts = 0,
                }
            }
            IndependentChainOutcome::Rejected {
                transaction_id,
                problem,
            } => {
                super::push_transaction_id(&mut context.response, transaction_id);
                publish(
                    &context.state,
                    super::failed(context.response, format!("授权交易被链上拒绝：{problem}")),
                );
                return;
            }
            IndependentChainOutcome::Pending {
                transaction_id,
                problem,
            } => {
                context.transaction_id = transaction_id.clone();
                super::push_transaction_id(&mut context.response, transaction_id);
                context.response.problem = Some(problem);
                context.response.updated_at_ms = common::time::now_ms();
                if super::record_run(&context.state, context.response.clone()).is_err() {
                    return;
                }
                attempts += 1;
            }
        }
        if attempts >= MAX_RECHECKS {
            context.response.status = OnchainTokenApprovalRunStatus::FinalityUnresolved;
            context.response.message =
                "授权交易终态超过两分钟仍未确认；没有提交 CEX 订单".to_owned();
            context.response.problem = Some(format!(
                "{}；后台终态追踪已达到本轮上限",
                context
                    .response
                    .problem
                    .as_deref()
                    .unwrap_or("链上授权终态未知")
            ));
            context.response.updated_at_ms = common::time::now_ms();
            publish(&context.state, context.response);
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(RECHECK_INTERVAL_MS)).await;
    }
}

enum RemainingOutcome {
    Completed,
    Failed(String),
    Pending,
}

async fn submit_remaining(context: &mut PendingTokenApproval) -> RemainingOutcome {
    if context.state.onchain_monitor().snapshot().config != context.config {
        return RemainingOutcome::Failed(
            "第一笔授权确认后链上配置已变化，后续授权未提交".to_owned(),
        );
    }
    while let Some(transaction) = context.remaining.pop_front() {
        let prepared = match execution_submit::prepare_independent_chain_transaction(
            &context.state,
            &context.config,
            &transaction,
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(problem) => {
                return RemainingOutcome::Failed(format!("后续授权签名失败：{problem}"))
            }
        };
        if let Err(problem) = context.state.onchain_token_approval_runs().intent(
            &context.response.run_id,
            context.response.transaction_ids.len(),
            prepared.transaction_id(),
            common::time::now_ms(),
        ) {
            return RemainingOutcome::Failed(problem);
        }
        match execution_submit::broadcast_independent_chain_transaction(prepared.clone()).await {
            IndependentChainOutcome::Confirmed { transaction_id } => {
                super::push_transaction_id(&mut context.response, transaction_id.clone());
                if let Err(problem) = super::receipts::confirmed(
                    &context.state,
                    &context.response.run_id,
                    &transaction_id,
                )
                .await
                {
                    return RemainingOutcome::Failed(problem);
                }
            }
            IndependentChainOutcome::Rejected {
                transaction_id,
                problem,
            } => {
                super::push_transaction_id(&mut context.response, transaction_id);
                return RemainingOutcome::Failed(format!("后续授权被链上拒绝：{problem}"));
            }
            IndependentChainOutcome::Pending {
                transaction_id,
                problem,
            } => {
                context.prepared = prepared;
                context.transaction_id = transaction_id.clone();
                super::push_transaction_id(&mut context.response, transaction_id);
                context.response.status = OnchainTokenApprovalRunStatus::AwaitingFinality;
                context.response.message = "后续授权交易已广播，正在等待链上确认".to_owned();
                context.response.problem = Some(problem);
                if let Err(problem) = super::record_run(&context.state, context.response.clone()) {
                    return RemainingOutcome::Failed(problem);
                }
                return RemainingOutcome::Pending;
            }
        }
    }
    RemainingOutcome::Completed
}

fn publish(state: &AppState, response: OnchainTokenApprovalSubmitResponse) {
    if let Err(problem) = super::record_run(state, response) {
        tracing::error!(%problem, "approval outcome was not durable");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reconciliation_window_is_bounded_to_about_two_minutes() {
        let elapsed = FIRST_RECHECK_DELAY_MS + RECHECK_INTERVAL_MS * MAX_RECHECKS as u64;
        assert!((120_000..=125_000).contains(&elapsed));
    }
}
