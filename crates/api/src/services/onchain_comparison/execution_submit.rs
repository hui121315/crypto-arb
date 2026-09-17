use axum::http::StatusCode;
use common::AppError;
use shared_types::{
    HedgeLegRole, LiveOrderState, OnchainCexOrderPlan, OnchainExecutionLegKind,
    OnchainExecutionLegResult, OnchainExecutionLegStatus, OnchainExecutionRecoveryAction,
    OnchainExecutionRecoveryKind, OnchainExecutionRunStatus, OnchainExecutionRunsResponse,
    OnchainExecutionSubmitRequest, OnchainExecutionSubmitResponse, OrderRecord, OrderSide,
    OrderSource, WebhookEvent, WebhookEventKind, WEBHOOK_EVENT_VERSION,
};
use trading::ExecutionLedgerOrderContext;

use crate::services::onchain_execution_build_store::{BuildClaimError, ClaimedOnchainBuild};
use crate::services::onchain_execution_run_store::{
    OnchainCexActionKind, OnchainExecutionStage, PendingOnchainExecution,
};
use crate::state::AppState;

use super::execution_build;

pub(super) mod accounting;
mod chain_settlement;

pub(crate) use accounting::refresh_pending as refresh_accounting;
mod compensation;
mod primary_alignment;
mod providers;
mod reconciliation;
mod settlement;
mod three_leg_submit;

const PRIVATE_FINALITY_WAIT_MS: u64 = 700;
const PRIVATE_FINALITY_POLL_MS: u64 = 20;
const MAX_STORED_RUNS: usize = 128;

#[derive(Clone, Copy)]
pub(super) struct ResponseContext<'a> {
    run_id: &'a str,
    build: &'a shared_types::OnchainExecutionBuildResponse,
    started_at_ms: i64,
}

struct RunOutcome {
    status: OnchainExecutionRunStatus,
    chain_transaction_id: Option<String>,
    remaining_exposure_usd: f64,
    message: String,
    problem: Option<String>,
}

#[derive(Clone, Copy)]
pub(super) struct CompensationContext<'a> {
    state: &'a AppState,
    response: ResponseContext<'a>,
    claimed: &'a ClaimedOnchainBuild,
    plan: &'a OnchainCexOrderPlan,
    reason: &'a str,
}

struct ChainOutcomeContext<'a> {
    state: &'a AppState,
    response: ResponseContext<'a>,
    claimed: &'a ClaimedOnchainBuild,
    plan: OnchainCexOrderPlan,
}

pub(super) struct ReversedCexOrder {
    pub(super) plan: OnchainCexOrderPlan,
    pub(super) record: OrderRecord,
    pub(super) base_asset: String,
    pub(super) original_order_id: String,
    pub(super) remaining_base: Option<rust_decimal::Decimal>,
    pub(super) recovery_problem: Option<String>,
}

impl ReversedCexOrder {
    fn recovered(&self) -> bool {
        terminal(self.record.state) && self.remaining_base == Some(rust_decimal::Decimal::ZERO)
    }

    fn residual_evidence(&self) -> Option<shared_types::OnchainCexRecoveryResidual> {
        Some(shared_types::OnchainCexRecoveryResidual {
            original_order_id: self.original_order_id.clone(),
            asset: self.base_asset.clone(),
            amount: self
                .remaining_base
                .map(|amount| amount.normalize().to_string()),
        })
    }
}

#[derive(Debug, Clone)]
pub(super) struct IndependentChainSubmission(providers::PreparedChainSubmission);

impl IndependentChainSubmission {
    pub(super) fn transaction_id(&self) -> &str {
        self.0.transaction_id()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum IndependentChainOutcome {
    Confirmed {
        transaction_id: String,
    },
    Rejected {
        transaction_id: String,
        problem: String,
    },
    Pending {
        transaction_id: String,
        problem: String,
    },
}

pub(super) fn readiness(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
) -> Result<(), String> {
    providers::readiness(state, config)
}

pub(super) fn submission_rpc(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
) -> Result<String, String> {
    providers::require_custom_rpc(state, config)
}

pub(super) async fn verified_submission_rpc(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
) -> Result<String, String> {
    providers::require_verified_custom_rpc(state, config).await
}

pub(super) async fn prepare_independent_chain_transaction(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    transaction: &shared_types::OnchainUnsignedTransaction,
) -> Result<IndependentChainSubmission, String> {
    providers::prepare(state, config, transaction)
        .await
        .map(IndependentChainSubmission)
}

pub(super) async fn prepare_independent_rpc_transaction(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    transaction: &shared_types::OnchainUnsignedTransaction,
) -> Result<IndependentChainSubmission, String> {
    providers::prepare_rpc(state, config, transaction)
        .await
        .map(IndependentChainSubmission)
}

pub(super) async fn broadcast_independent_chain_transaction(
    prepared: IndependentChainSubmission,
) -> IndependentChainOutcome {
    independent_outcome(providers::broadcast(prepared.0).await)
}

pub(super) async fn recheck_independent_chain_transaction(
    prepared: &IndependentChainSubmission,
    transaction_id: &str,
) -> IndependentChainOutcome {
    independent_outcome(providers::recheck(&prepared.0, transaction_id).await)
}

fn independent_outcome(outcome: providers::ChainSubmissionOutcome) -> IndependentChainOutcome {
    match outcome {
        providers::ChainSubmissionOutcome::Confirmed { transaction_id } => {
            IndependentChainOutcome::Confirmed { transaction_id }
        }
        providers::ChainSubmissionOutcome::Rejected {
            transaction_id,
            problem,
        } => IndependentChainOutcome::Rejected {
            transaction_id,
            problem,
        },
        providers::ChainSubmissionOutcome::Pending {
            transaction_id,
            problem,
        } => IndependentChainOutcome::Pending {
            transaction_id,
            problem,
        },
    }
}

pub(crate) fn recent_runs(state: &AppState, limit: usize) -> OnchainExecutionRunsResponse {
    let mut rows = state
        .onchain_execution_runs()
        .iter()
        .map(|entry| entry.value().clone())
        .collect::<Vec<_>>();
    rows.sort_by_key(|row| std::cmp::Reverse(row.updated_at_ms));
    rows.truncate(limit.clamp(1, MAX_STORED_RUNS));
    for row in &mut rows {
        let saved_value = row.accounting.as_ref().and_then(|a| a.usd_value.clone());
        settlement::enrich(state, row);
        if let Some(accounting) = &mut row.accounting {
            if accounting.usd_value.is_some() && accounting.usd_value != saved_value {
                accounting.usd_value = None;
                accounting.status =
                    shared_types::OnchainExecutionAccountingStatus::PendingValuation;
                accounting
                    .problems
                    .push("美元折算等待后台保存，暂不确认净变动".into());
            }
        }
    }
    OnchainExecutionRunsResponse {
        rows,
        observed_at_ms: common::time::now_ms(),
        recovery_problem: state.onchain_execution_run_store().readiness().err(),
    }
}

pub(crate) fn restore_reconciliations(state: &AppState, pending: Vec<PendingOnchainExecution>) {
    reconciliation::restore(state, pending);
    let rows = state
        .onchain_execution_runs()
        .iter()
        .map(|row| row.value().clone())
        .collect::<Vec<_>>();
    for row in rows {
        chain_settlement::spawn(state, &row);
    }
}

pub(crate) async fn submit(
    state: &AppState,
    request: &OnchainExecutionSubmitRequest,
) -> Result<OnchainExecutionSubmitResponse, AppError> {
    state
        .onchain_execution_run_store()
        .readiness()
        .map_err(|problem| conflict("ONCHAIN_EXECUTION_RECOVERY_UNAVAILABLE", problem))?;
    if let Some(existing) = replayed_run(state, &request.build_id) {
        return Ok(existing);
    }
    let started_at_ms = common::time::now_ms();
    let run_id = format!("onchain-run-{}", uuid::Uuid::new_v4());
    let claimed =
        match state
            .onchain_execution_builds()
            .claim(&request.build_id, &run_id, started_at_ms)
        {
            Ok(claimed) => claimed,
            Err(BuildClaimError::AlreadyClaimed(existing_run_id)) => {
                if let Some(existing) = state.onchain_execution_runs().get(&existing_run_id) {
                    return Ok(existing.value().clone());
                }
                return Err(claim_error(BuildClaimError::AlreadyClaimed(
                    existing_run_id,
                )));
            }
            Err(error) => return Err(claim_error(error)),
        };
    let result = submit_claimed(state, &run_id, started_at_ms, &claimed).await;
    match result {
        Ok(response) => {
            state
                .onchain_execution_builds()
                .finish(&request.build_id, &run_id);
            let response = record_run(state, response);
            emit_webhook(state, &response).await;
            Ok(response)
        }
        Err(error) => {
            if let Some(existing) = state
                .onchain_execution_runs()
                .get(&run_id)
                .map(|entry| entry.value().clone())
            {
                let response = record_run(state, interrupted_response(existing, error.to_string()));
                state
                    .onchain_execution_builds()
                    .finish(&request.build_id, &run_id);
                emit_webhook(state, &response).await;
                return Ok(response);
            }
            state
                .onchain_execution_builds()
                .release(&request.build_id, &run_id);
            Err(error)
        }
    }
}

fn replayed_run(state: &AppState, build_id: &str) -> Option<OnchainExecutionSubmitResponse> {
    state
        .onchain_execution_runs()
        .iter()
        .filter(|entry| entry.build_id == build_id)
        .max_by_key(|entry| entry.updated_at_ms)
        .map(|entry| entry.value().clone())
}

async fn submit_claimed(
    state: &AppState,
    run_id: &str,
    started_at_ms: i64,
    claimed: &ClaimedOnchainBuild,
) -> Result<OnchainExecutionSubmitResponse, AppError> {
    let build = &claimed.response;
    let response_context = ResponseContext {
        run_id,
        build,
        started_at_ms,
    };
    if state.onchain_monitor().snapshot().config != claimed.config {
        return Err(conflict(
            "ONCHAIN_EXECUTION_CONFIG_CHANGED",
            "链上配置已变化，请重新构建交易计划",
        ));
    }
    providers::readiness(state, &claimed.config)
        .map_err(|problem| conflict("ONCHAIN_SUBMISSION_NOT_READY", problem))?;
    let plans = execution_build::revalidate_for_submit(state, &claimed.config, build).await?;
    if let Some(conversion) = plans.quote_conversion {
        return three_leg_submit::submit(
            state,
            response_context,
            claimed,
            plans.primary,
            conversion,
        )
        .await;
    }
    let cex_plan = plans.primary;
    let prepared = providers::prepare(state, &claimed.config, &build.chain_transaction)
        .await
        .map_err(|problem| conflict("ONCHAIN_SIGNING_FAILED", problem))?;
    record_prepared(state, response_context, claimed, &cex_plan, None)?;
    let cex_record = match submit_cex_order(
        state,
        run_id,
        &claimed.config,
        &cex_plan,
        OnchainCexActionKind::Primary,
    )
    .await
    {
        Ok(record) => record,
        Err(problem) => {
            return Ok(response(
                response_context,
                None,
                RunOutcome {
                    status: OnchainExecutionRunStatus::Failed,
                    chain_transaction_id: None,
                    remaining_exposure_usd: 0.0,
                    message: "CEX Spot 腿未提交，链上交易未广播".to_owned(),
                    problem: Some(problem),
                },
            ))
        }
    };
    let target_quantity = cex_plan.sizing_plan.rounded_contracts;
    if !fully_filled(&cex_record, target_quantity) {
        return Ok(compensate_or_expose(
            CompensationContext {
                state,
                response: response_context,
                claimed,
                plan: &cex_plan,
                reason: "CEX Spot 腿未全额成交，链上交易未广播",
            },
            cex_record,
        )
        .await);
    }
    let aligned =
        match primary_alignment::align(state, claimed, &cex_plan, &cex_record, &[], prepared).await
        {
            Ok(aligned) => aligned,
            Err(problem) => {
                return Ok(compensate_or_expose(
                    CompensationContext {
                        state,
                        response: response_context,
                        claimed,
                        plan: &cex_plan,
                        reason: &problem,
                    },
                    cex_record,
                )
                .await)
            }
        };
    let prepared = aligned.prepared;
    let claimed = &aligned.claimed;
    let response_context = ResponseContext {
        build: &claimed.response,
        ..response_context
    };
    let broadcast_checkpoint = reconciliation::PendingChainExecution {
        state: state.clone(),
        run_id: response_context.run_id.to_owned(),
        started_at_ms: response_context.started_at_ms,
        claimed: claimed.clone(),
        cex_plan: cex_plan.clone(),
        cex_record: cex_record.clone(),
        quote_conversion_plan: None,
        quote_conversion_attempts: Vec::new(),
        pending: response(
            response_context,
            Some(&cex_record),
            RunOutcome {
                status: OnchainExecutionRunStatus::AwaitingChainFinality,
                chain_transaction_id: Some(prepared.transaction_id().to_owned()),
                remaining_exposure_usd: order_exposure_usd(&cex_record, &cex_plan),
                message: "链上交易正在广播；若后端重启将按交易哈希继续核验".to_owned(),
                problem: None,
            },
        ),
        transaction_id: prepared.transaction_id().to_owned(),
    };
    record_pending_with_stage(
        state,
        &broadcast_checkpoint,
        OnchainExecutionStage::ChainBroadcasting,
    )
    .map_err(durability_error)?;
    let chain = providers::broadcast(prepared).await;
    Ok(resolve_chain_outcome(
        ChainOutcomeContext {
            state,
            response: response_context,
            claimed,
            plan: cex_plan,
        },
        cex_record,
        chain,
    )
    .await)
}

async fn resolve_chain_outcome(
    context: ChainOutcomeContext<'_>,
    cex_record: OrderRecord,
    chain: providers::ChainSubmissionOutcome,
) -> OnchainExecutionSubmitResponse {
    let ChainOutcomeContext {
        state,
        response: response_context,
        claimed,
        plan,
    } = context;
    match chain {
        providers::ChainSubmissionOutcome::Confirmed { transaction_id } => response(
            response_context,
            Some(&cex_record),
            RunOutcome {
                status: OnchainExecutionRunStatus::Completed,
                chain_transaction_id: Some(transaction_id),
                remaining_exposure_usd: 0.0,
                message: "双腿均已确认完成".to_owned(),
                problem: None,
            },
        ),
        providers::ChainSubmissionOutcome::Pending {
            transaction_id,
            problem,
        } => {
            let pending = response(
                response_context,
                Some(&cex_record),
                RunOutcome {
                    status: OnchainExecutionRunStatus::AwaitingChainFinality,
                    chain_transaction_id: Some(transaction_id.clone()),
                    remaining_exposure_usd: order_exposure_usd(&cex_record, &plan),
                    message: "链上交易已广播但终态未确认；为避免反向过度对冲，暂不自动回滚 CEX"
                        .to_owned(),
                    problem: Some(problem),
                },
            );
            let reconciliation = reconciliation::PendingChainExecution {
                state: state.clone(),
                run_id: response_context.run_id.to_owned(),
                started_at_ms: response_context.started_at_ms,
                claimed: claimed.clone(),
                cex_plan: plan,
                cex_record,
                quote_conversion_plan: None,
                quote_conversion_attempts: Vec::new(),
                pending: pending.clone(),
                transaction_id,
            };
            if let Err(problem) = record_pending(state, &reconciliation) {
                tracing::warn!(run_id = response_context.run_id, %problem, "chain is already broadcast; retaining read-only finality tracking");
            }
            reconciliation::spawn(reconciliation);
            pending
        }
        providers::ChainSubmissionOutcome::Rejected {
            transaction_id,
            problem,
        } => {
            let mut response = compensate_or_expose(
                CompensationContext {
                    state,
                    response: response_context,
                    claimed,
                    plan: &plan,
                    reason: "链上交易明确失败，已启动 CEX 反向补偿",
                },
                cex_record,
            )
            .await;
            attach_chain_transaction(
                &mut response,
                response_context.build,
                transaction_id,
                OnchainExecutionLegStatus::Rejected,
            );
            response.problem = Some(match response.problem.take() {
                Some(compensation_problem) => format!("{problem}；{compensation_problem}"),
                None => problem,
            });
            response
        }
    }
}

async fn submit_cex_order(
    state: &AppState,
    run_id: &str,
    config: &shared_types::OnchainComparisonConfig,
    plan: &OnchainCexOrderPlan,
    kind: OnchainCexActionKind,
) -> Result<OrderRecord, String> {
    state
        .onchain_execution_run_store()
        .append_cex_intent(run_id, plan, kind)?;
    let intent = execution_build::cex_order_intent(config, plan, common::time::now_ms());
    let role = leg_role(intent.side);
    let ledger =
        ExecutionLedgerOrderContext::new(run_id.to_owned(), plan.client_order_id.clone(), role);
    let record = state
        .trading_service()
        .submit_with_ledger_context_and_context(
            intent,
            ledger,
            execution_build::cex_submission_context(plan),
        )
        .await
        .map_err(|error| format!("CEX Spot 下单结果未确认：{error}"))?;
    let record = settle_cex_order(state, record).await;
    state
        .onchain_execution_run_store()
        .append_cex_observation(run_id, &record)?;
    Ok(record)
}

async fn settle_cex_order(state: &AppState, record: OrderRecord) -> OrderRecord {
    let mut latest = record;
    let deadline =
        tokio::time::Instant::now() + std::time::Duration::from_millis(PRIVATE_FINALITY_WAIT_MS);
    while tokio::time::Instant::now() < deadline && !terminal(latest.state) {
        tokio::time::sleep(std::time::Duration::from_millis(PRIVATE_FINALITY_POLL_MS)).await;
        if let Some(updated) = state.trading_service().get_order(&latest.intent.id) {
            latest = updated;
        }
    }
    if !terminal(latest.state) {
        if let Ok(Some(updated)) = state
            .trading_service()
            .refresh_order_state(&latest.intent.id)
            .await
        {
            latest = updated;
        }
    }
    if !terminal(latest.state) {
        if let Ok(cancelled) = state.trading_service().cancel(&latest.intent.id).await {
            latest = cancelled;
            if let Ok(Some(updated)) = state
                .trading_service()
                .refresh_order_state(&latest.intent.id)
                .await
            {
                latest = updated;
            }
        }
    }
    latest
}

async fn compensate_or_expose(
    context: CompensationContext<'_>,
    cex_record: OrderRecord,
) -> OnchainExecutionSubmitResponse {
    let response_context = context.response;
    let plan = context.plan;
    let reason = context.reason;
    if !terminal(cex_record.state) {
        return response(
            response_context,
            Some(&cex_record),
            RunOutcome {
                status: OnchainExecutionRunStatus::FinalityUnresolved,
                chain_transaction_id: None,
                remaining_exposure_usd: order_exposure_usd(&cex_record, plan),
                message: "原 CEX 订单仍可能成交，不能重复下单或反向补偿".into(),
                problem: Some(reason.into()),
            },
        );
    }
    let filled_contracts = cex_record.filled_quantity.unwrap_or(0.0).max(0.0);
    if filled_contracts <= 0.0 {
        return response(
            response_context,
            Some(&cex_record),
            RunOutcome {
                status: OnchainExecutionRunStatus::Failed,
                chain_transaction_id: None,
                remaining_exposure_usd: 0.0,
                message: reason.to_owned(),
                problem: Some("CEX Spot 腿没有确认成交数量".to_owned()),
            },
        );
    }
    let reversed = match reverse_filled_order(context, plan, &cex_record, "primary").await {
        Ok(reversed) => reversed,
        Err(problem) => {
            return response(
                response_context,
                Some(&cex_record),
                RunOutcome {
                    status: OnchainExecutionRunStatus::Exposed,
                    chain_transaction_id: None,
                    remaining_exposure_usd: order_exposure_usd(&cex_record, plan),
                    message: reason.to_owned(),
                    problem: Some(problem),
                },
            )
        }
    };
    let mut response = if reversed.recovered() {
        response_with_compensation(
            response_context,
            &cex_record,
            Some(&reversed.record),
            RunOutcome {
                status: OnchainExecutionRunStatus::Compensated,
                chain_transaction_id: None,
                remaining_exposure_usd: 0.0,
                message: reason.to_owned(),
                problem: None,
            },
        )
    } else {
        response_with_compensation(
            response_context,
            &cex_record,
            Some(&reversed.record),
            RunOutcome {
                status: OnchainExecutionRunStatus::Exposed,
                chain_transaction_id: None,
                remaining_exposure_usd: order_exposure_usd(&cex_record, plan),
                message: reason.to_owned(),
                problem: reversed
                    .recovery_problem
                    .clone()
                    .or_else(|| Some("CEX 回滚后的净资产尚未归零".into())),
            },
        )
    };
    if let Some(leg) = response.legs.last_mut() {
        leg.recovery_residual = reversed.residual_evidence();
    }
    response
}

pub(super) async fn reverse_filled_order(
    context: CompensationContext<'_>,
    plan: &OnchainCexOrderPlan,
    record: &OrderRecord,
    leg_key: &str,
) -> Result<ReversedCexOrder, String> {
    if !terminal(record.state) {
        return Err("原订单终态未确认，不能反向补偿".into());
    }
    let filled_contracts = record.filled_quantity.unwrap_or(0.0).max(0.0);
    if filled_contracts <= 0.0 {
        return Err("CEX 订单没有可反向处理的确认成交数量".to_owned());
    }
    let original_receipt =
        settlement::confirmed_order(context.state, record, &plan.instrument_spec).await?;
    let compensation = compensation::plan(plan, record, &original_receipt)?;
    let compensation_record = submit_compensation(context, &compensation, leg_key)
        .await
        .map_err(|error| format!("CEX 自动补偿下单失败：{error}"))?;
    let remaining = if terminal(compensation_record.state)
        && compensation_record.filled_quantity == Some(0.0)
    {
        compensation::base_delta(&original_receipt)
    } else {
        match settlement::confirmed_order(
            context.state,
            &compensation_record,
            &compensation.instrument_spec,
        )
        .await
        {
            Ok(reverse_receipt) => compensation::residual(&original_receipt, &reverse_receipt),
            Err(problem) => Err(problem),
        }
    };
    let (remaining_base, recovery_problem) = match remaining {
        Ok(amount) => (
            Some(amount),
            (amount != rust_decimal::Decimal::ZERO).then(|| {
                format!(
                    "回滚后仍有 {} {} 净变动；手续费或下单步长留下的余额不能视为已清零",
                    amount.normalize(),
                    original_receipt.basis.base_asset
                )
            }),
        ),
        Err(problem) => (
            None,
            Some(format!("补偿订单已留存，但净到账未核清：{problem}")),
        ),
    };
    Ok(ReversedCexOrder {
        plan: compensation,
        record: compensation_record,
        base_asset: original_receipt.basis.base_asset,
        original_order_id: record.intent.id.clone(),
        remaining_base,
        recovery_problem,
    })
}

async fn submit_compensation(
    context: CompensationContext<'_>,
    compensation: &OnchainCexOrderPlan,
    leg_key: &str,
) -> Result<OrderRecord, String> {
    context
        .state
        .onchain_execution_run_store()
        .append_cex_intent(
            context.response.run_id,
            compensation,
            OnchainCexActionKind::Compensation,
        )?;
    let mut intent = execution_build::cex_order_intent(
        &context.claimed.config,
        compensation,
        common::time::now_ms(),
    );
    intent.source = OrderSource::CloseRunCompensation;
    let ledger = ExecutionLedgerOrderContext::new(
        context.response.run_id.to_owned(),
        format!("{}-{leg_key}-compensation", context.response.build.build_id),
        leg_role(intent.side),
    );
    let record = context
        .state
        .trading_service()
        .submit_with_ledger_context_and_context(
            intent,
            ledger,
            execution_build::cex_submission_context(compensation),
        )
        .await
        .map_err(|error| error.to_string())?;
    let record = settle_cex_order(context.state, record).await;
    context
        .state
        .onchain_execution_run_store()
        .append_cex_observation(context.response.run_id, &record)?;
    Ok(record)
}

fn response(
    context: ResponseContext<'_>,
    cex: Option<&OrderRecord>,
    outcome: RunOutcome,
) -> OnchainExecutionSubmitResponse {
    let ResponseContext {
        run_id,
        build,
        started_at_ms,
    } = context;
    let mut legs = cex
        .into_iter()
        .map(|record| {
            order_leg_result(
                1,
                OnchainExecutionLegKind::PrimaryCex,
                record,
                &build.cex_order.instrument_spec,
            )
        })
        .collect::<Vec<_>>();
    if let Some(transaction_id) = outcome.chain_transaction_id.as_deref() {
        legs.push(chain_leg_result(
            build,
            transaction_id,
            chain_status(outcome.status),
            outcome.problem.as_deref().unwrap_or(&outcome.message),
        ));
    }
    let mut result = OnchainExecutionSubmitResponse {
        run_id: run_id.to_owned(),
        build_id: build.build_id.clone(),
        status: outcome.status,
        cex_order_id: cex.map(|record| record.intent.id.clone()),
        cex_order_state: cex.map(|record| record.state),
        cex_filled_quantity: cex.and_then(|record| record.filled_quantity),
        chain_transaction_id: outcome.chain_transaction_id,
        compensation_order_id: None,
        legs,
        recovery_actions: recovery_actions(outcome.status),
        replenishment_costs: build.replenishment_costs.clone(),
        approval_costs: build.approval_costs.clone(),
        estimated_net_profit_usd: build.estimated_net_profit_usd,
        remaining_exposure_usd: outcome.remaining_exposure_usd,
        quantity_reconciled: false,
        accounting: None,
        message: outcome.message,
        problem: outcome.problem,
        started_at_ms,
        updated_at_ms: common::time::now_ms(),
    };
    primary_alignment::retain_residual(build, &mut result);
    result
}

fn recovery_actions(status: OnchainExecutionRunStatus) -> Vec<OnchainExecutionRecoveryAction> {
    let action = |kind, automated, message: &str| OnchainExecutionRecoveryAction {
        kind,
        automated,
        message: message.to_owned(),
    };
    match status {
        OnchainExecutionRunStatus::Executing => vec![action(
            OnchainExecutionRecoveryKind::DoNotResubmit,
            false,
            "执行任务正在按顺序推进，不要重复提交",
        )],
        OnchainExecutionRunStatus::AwaitingChainFinality => vec![
            action(
                OnchainExecutionRecoveryKind::WaitForFinality,
                true,
                "系统正在追踪链上终态，暂时保留 CEX 对冲",
            ),
            action(
                OnchainExecutionRecoveryKind::DoNotResubmit,
                false,
                "不要重复提交同一计划，避免形成双倍仓位",
            ),
        ],
        OnchainExecutionRunStatus::FinalityUnresolved => vec![
            action(
                OnchainExecutionRecoveryKind::DoNotResubmit,
                false,
                "先核对链上交易哈希，再决定是否处理 CEX 对冲",
            ),
            action(
                OnchainExecutionRecoveryKind::VerifyExposure,
                false,
                "按逐腿记录核对链上余额与 CEX 成交数量",
            ),
        ],
        OnchainExecutionRunStatus::Exposed => vec![action(
            OnchainExecutionRecoveryKind::VerifyExposure,
            false,
            "存在未对冲资产，请按逐腿成交记录处理剩余暴露",
        )],
        OnchainExecutionRunStatus::Compensated | OnchainExecutionRunStatus::Failed => {
            vec![action(
                OnchainExecutionRecoveryKind::RebuildPlan,
                false,
                "旧计划不可再次使用；确认余额后重新读取报价并构建",
            )]
        }
        OnchainExecutionRunStatus::Completed => Vec::new(),
    }
}

fn response_with_compensation(
    context: ResponseContext<'_>,
    cex: &OrderRecord,
    compensation: Option<&OrderRecord>,
    outcome: RunOutcome,
) -> OnchainExecutionSubmitResponse {
    let mut response = response(context, Some(cex), outcome);
    response.compensation_order_id = compensation.map(|record| record.intent.id.clone());
    if let Some(record) = compensation {
        response.legs.push(order_leg_result(
            u8::try_from(response.legs.len() + 1).unwrap_or(u8::MAX),
            OnchainExecutionLegKind::Compensation,
            record,
            &context.build.cex_order.instrument_spec,
        ));
    }
    response
}

fn order_leg_result(
    position: u8,
    kind: OnchainExecutionLegKind,
    record: &OrderRecord,
    spec: &shared_types::InstrumentSpec,
) -> OnchainExecutionLegResult {
    OnchainExecutionLegResult {
        position,
        kind,
        status: order_leg_status(record.state),
        venue: record.intent.exchange.clone(),
        symbol: Some(record.intent.symbol.clone()),
        order_id: Some(record.intent.id.clone()),
        transaction_id: None,
        filled_quantity: record.filled_quantity,
        settlement: settlement::seed(record, spec),
        recovery_residual: None,
        chain_input_adjustment: None,
        chain_settlement: None,
        message: record
            .message
            .clone()
            .unwrap_or_else(|| format!("CEX 订单状态 {:?}", record.state)),
    }
}

fn chain_leg_result(
    build: &shared_types::OnchainExecutionBuildResponse,
    transaction_id: &str,
    status: OnchainExecutionLegStatus,
    message: &str,
) -> OnchainExecutionLegResult {
    OnchainExecutionLegResult {
        position: 2,
        kind: OnchainExecutionLegKind::Chain,
        status,
        venue: build.chain.clone(),
        symbol: None,
        order_id: None,
        transaction_id: Some(transaction_id.to_owned()),
        filled_quantity: None,
        settlement: None,
        recovery_residual: None,
        chain_input_adjustment: build.chain_input_adjustment.clone(),
        chain_settlement: chain_settlement::seed(build, transaction_id),
        message: message.to_owned(),
    }
}

fn attach_chain_transaction(
    response: &mut OnchainExecutionSubmitResponse,
    build: &shared_types::OnchainExecutionBuildResponse,
    transaction_id: String,
    status: OnchainExecutionLegStatus,
) {
    response.chain_transaction_id = Some(transaction_id.clone());
    if let Some(leg) = response
        .legs
        .iter_mut()
        .find(|leg| leg.kind == OnchainExecutionLegKind::Chain)
    {
        if leg.transaction_id.as_deref() != Some(&transaction_id) {
            leg.chain_settlement = chain_settlement::seed(build, &transaction_id);
        }
        leg.transaction_id = Some(transaction_id);
        leg.status = status;
        return;
    }
    response.legs.push(OnchainExecutionLegResult {
        position: 2,
        kind: OnchainExecutionLegKind::Chain,
        status,
        venue: build.chain.clone(),
        symbol: None,
        order_id: None,
        transaction_id: Some(transaction_id.clone()),
        filled_quantity: None,
        settlement: None,
        recovery_residual: None,
        chain_input_adjustment: build.chain_input_adjustment.clone(),
        chain_settlement: chain_settlement::seed(build, &transaction_id),
        message: "链上交易终态已更新".to_owned(),
    });
}

const fn chain_status(status: OnchainExecutionRunStatus) -> OnchainExecutionLegStatus {
    match status {
        OnchainExecutionRunStatus::Executing => OnchainExecutionLegStatus::Submitted,
        OnchainExecutionRunStatus::Completed => OnchainExecutionLegStatus::Confirmed,
        OnchainExecutionRunStatus::AwaitingChainFinality
        | OnchainExecutionRunStatus::FinalityUnresolved => OnchainExecutionLegStatus::Pending,
        OnchainExecutionRunStatus::Compensated | OnchainExecutionRunStatus::Failed => {
            OnchainExecutionLegStatus::Rejected
        }
        OnchainExecutionRunStatus::Exposed => OnchainExecutionLegStatus::Exposed,
    }
}

const fn order_leg_status(state: LiveOrderState) -> OnchainExecutionLegStatus {
    match state {
        LiveOrderState::Filled => OnchainExecutionLegStatus::Filled,
        LiveOrderState::PartiallyFilled => OnchainExecutionLegStatus::PartiallyFilled,
        LiveOrderState::Cancelled | LiveOrderState::CancelRequested => {
            OnchainExecutionLegStatus::Cancelled
        }
        LiveOrderState::Rejected => OnchainExecutionLegStatus::Rejected,
        LiveOrderState::Failed | LiveOrderState::Unknown => OnchainExecutionLegStatus::Failed,
        LiveOrderState::Created
        | LiveOrderState::RiskChecked
        | LiveOrderState::Submitted
        | LiveOrderState::Accepted => OnchainExecutionLegStatus::Submitted,
    }
}

fn fully_filled(record: &OrderRecord, target_quantity: f64) -> bool {
    record.state == LiveOrderState::Filled
        && record.filled_quantity.is_some_and(|filled| {
            filled.is_finite()
                && target_quantity.is_finite()
                && target_quantity > 0.0
                && filled >= target_quantity * (1.0 - 1e-6)
        })
}

fn terminal(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
}

fn order_exposure_usd(record: &OrderRecord, plan: &OnchainCexOrderPlan) -> f64 {
    let quantity = record.filled_quantity.unwrap_or(0.0).max(0.0);
    let price = record.filled_price.unwrap_or(plan.reference_price).max(0.0);
    quantity * plan.sizing_plan.contract_size * price
}

const fn opposite(side: OrderSide) -> OrderSide {
    match side {
        OrderSide::Buy => OrderSide::Sell,
        OrderSide::Sell => OrderSide::Buy,
    }
}

const fn leg_role(side: OrderSide) -> HedgeLegRole {
    match side {
        OrderSide::Buy => HedgeLegRole::Long,
        OrderSide::Sell => HedgeLegRole::Short,
    }
}

fn claim_error(error: BuildClaimError) -> AppError {
    match error {
        BuildClaimError::Missing => conflict(
            "ONCHAIN_BUILD_NOT_FOUND",
            "交易计划不存在或已完成，请重新构建",
        ),
        BuildClaimError::Expired => conflict("ONCHAIN_BUILD_EXPIRED", "交易计划已过期，请重新构建"),
        BuildClaimError::AlreadyClaimed(run_id) => conflict(
            "ONCHAIN_BUILD_ALREADY_SUBMITTED",
            format!("交易计划已由执行任务 {run_id} 领取"),
        ),
    }
}

fn record_run(
    state: &AppState,
    mut response: OnchainExecutionSubmitResponse,
) -> OnchainExecutionSubmitResponse {
    let mut current = state
        .onchain_execution_runs()
        .entry(response.run_id.clone())
        .or_insert_with(|| response.clone());
    chain_settlement::preserve_known(&current, &mut response);
    response.accounting = current.accounting.clone();
    settlement::enrich(state, &mut response);
    if state
        .onchain_execution_run_store()
        .unresolved_cex_action(&response.run_id)
    {
        response.status = OnchainExecutionRunStatus::FinalityUnresolved;
        response = interrupted_response(
            response,
            "上一笔 CEX 订单终态未确认；已保留提交记录，不能认定未下单或重复提交".into(),
        );
    }
    if let Err(problem) = state.onchain_execution_run_store().append_run(&response) {
        response = interrupted_response(response, problem);
    }
    *current = response.clone();
    drop(current);
    prune_runs(state);
    chain_settlement::spawn(state, &response);
    response
}

fn interrupted_response(
    mut response: OnchainExecutionSubmitResponse,
    problem: String,
) -> OnchainExecutionSubmitResponse {
    response.quantity_reconciled = false;
    if let Some(accounting) = &mut response.accounting {
        accounting.status = shared_types::OnchainExecutionAccountingStatus::PendingReceipts;
        accounting.usd_value = None;
        let warning = "执行或日志尚未确认，暂不确认美元净变动".to_owned();
        if !accounting.problems.contains(&warning) {
            accounting.problems.push(warning);
        }
    }
    if matches!(
        response.status,
        OnchainExecutionRunStatus::Executing | OnchainExecutionRunStatus::AwaitingChainFinality
    ) {
        response.status = OnchainExecutionRunStatus::FinalityUnresolved;
    }
    response.message =
        "执行恢复需要核对；已停止新增资金动作，保留订单号和交易哈希，不要重复提交".into();
    response.problem = Some(match response.problem.take() {
        Some(old) if !old.contains(&problem) => format!("{old}；{problem}"),
        Some(old) => old,
        None => problem,
    });
    response.recovery_actions = recovery_actions(OnchainExecutionRunStatus::FinalityUnresolved);
    response.updated_at_ms = common::time::now_ms();
    response
}

fn publish_run(state: &AppState, response: OnchainExecutionSubmitResponse) {
    state
        .onchain_execution_runs()
        .insert(response.run_id.clone(), response);
    prune_runs(state);
}

fn prune_runs(state: &AppState) {
    if state.onchain_execution_runs().len() <= MAX_STORED_RUNS {
        return;
    }
    let oldest = state
        .onchain_execution_runs()
        .iter()
        .filter(|entry| {
            matches!(
                entry.status,
                OnchainExecutionRunStatus::Completed
                    | OnchainExecutionRunStatus::Compensated
                    | OnchainExecutionRunStatus::Failed
            )
        })
        .min_by_key(|entry| entry.updated_at_ms)
        .map(|entry| entry.key().clone());
    if let Some(run_id) = oldest {
        state.onchain_execution_runs().remove(&run_id);
    }
}

fn durability_error(problem: String) -> AppError {
    conflict("ONCHAIN_EXECUTION_RECOVERY_UNAVAILABLE", problem)
}

fn record_prepared(
    state: &AppState,
    context: ResponseContext<'_>,
    claimed: &ClaimedOnchainBuild,
    primary: &OnchainCexOrderPlan,
    conversion: Option<&shared_types::OnchainQuoteConversionOrderPlan>,
) -> Result<(), AppError> {
    record_checkpoint(
        state,
        &PendingOnchainExecution {
            response: response(
                context,
                None,
                RunOutcome {
                    status: OnchainExecutionRunStatus::Executing,
                    chain_transaction_id: None,
                    remaining_exposure_usd: 0.0,
                    message: "执行计划已保存，尚未提交资金动作".into(),
                    problem: None,
                },
            ),
            config: claimed.config.clone(),
            build: claimed.response.clone(),
            stage: OnchainExecutionStage::Prepared,
            primary_plan: primary.clone(),
            primary_record: None,
            quote_conversion_plan: conversion.cloned(),
            quote_conversion_records: Vec::new(),
            quote_conversion_attempts: Vec::new(),
            transaction_id: None,
            active_cex_order: None,
            active_cex_kind: None,
            active_cex_record: None,
        },
    )
    .map_err(durability_error)
}

fn record_checkpoint(state: &AppState, checkpoint: &PendingOnchainExecution) -> Result<(), String> {
    let result = state
        .onchain_execution_run_store()
        .append_pending(checkpoint);
    // One durable row holds both the visible state and the recovery checkpoint.
    let response = match &result {
        Ok(()) => checkpoint.response.clone(),
        Err(problem) => interrupted_response(checkpoint.response.clone(), problem.clone()),
    };
    publish_run(state, response);
    result
}

fn record_pending(
    state: &AppState,
    context: &reconciliation::PendingChainExecution,
) -> Result<(), String> {
    record_pending_with_stage(state, context, OnchainExecutionStage::AwaitingChainFinality)
}

fn record_pending_with_stage(
    state: &AppState,
    context: &reconciliation::PendingChainExecution,
    stage: OnchainExecutionStage,
) -> Result<(), String> {
    record_checkpoint(
        state,
        &PendingOnchainExecution {
            response: context.pending.clone(),
            config: context.claimed.config.clone(),
            build: context.claimed.response.clone(),
            stage,
            primary_plan: context.cex_plan.clone(),
            primary_record: Some(context.cex_record.clone()),
            quote_conversion_plan: context.quote_conversion_plan.clone(),
            quote_conversion_records: context
                .quote_conversion_attempts
                .iter()
                .map(|attempt| attempt.record.clone())
                .collect(),
            quote_conversion_attempts: context.quote_conversion_attempts.clone(),
            transaction_id: Some(context.transaction_id.clone()),
            active_cex_order: None,
            active_cex_kind: None,
            active_cex_record: None,
        },
    )
}

async fn emit_webhook(state: &AppState, response: &OnchainExecutionSubmitResponse) {
    let kind = match response.status {
        OnchainExecutionRunStatus::Executing => WebhookEventKind::ExecutionResult,
        OnchainExecutionRunStatus::Compensated => WebhookEventKind::Compensation,
        OnchainExecutionRunStatus::AwaitingChainFinality
        | OnchainExecutionRunStatus::FinalityUnresolved
        | OnchainExecutionRunStatus::Exposed => WebhookEventKind::RiskAlert,
        OnchainExecutionRunStatus::Completed | OnchainExecutionRunStatus::Failed => {
            WebhookEventKind::ExecutionResult
        }
    };
    let event = WebhookEvent {
        id: format!("{}:{:?}", response.run_id, response.status).to_ascii_lowercase(),
        version: WEBHOOK_EVENT_VERSION.to_owned(),
        kind,
        occurred_at_ms: response.updated_at_ms,
        payload: serde_json::json!({
            "title": "CROSSLINE 链上/CEX 套利执行",
            "runId": response.run_id,
            "buildId": response.build_id,
            "status": response.status,
            "message": response.message,
            "problem": response.problem,
            "cexOrderId": response.cex_order_id,
            "chainTransactionId": response.chain_transaction_id,
            "compensationOrderId": response.compensation_order_id,
            "remainingExposureUsd": response.remaining_exposure_usd,
            "legs": response.legs,
        }),
    };
    if let Err(error) = state.webhook().enqueue(event, false).await {
        tracing::warn!(%error, run_id = %response.run_id, "onchain execution webhook enqueue failed");
    }
}

fn conflict(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::CONFLICT, code, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execution_recovery_write_barrier_stops_primary_and_compensation_before_trading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runs.jsonl");
        let mut config = common::config::AppConfig::default();
        config.history.enabled = false;
        config.storage.onchain_execution_run_ledger_path = Some(path.to_string_lossy().into());
        let state = AppState::new(config).await.unwrap();
        let fixture = crate::services::onchain_execution_run_store::test_checkpoint();
        let claimed = ClaimedOnchainBuild {
            response: fixture.build,
            config: fixture.config,
        };
        let response = ResponseContext {
            run_id: "barrier-run",
            build: &claimed.response,
            started_at_ms: 1,
        };
        record_prepared(&state, response, &claimed, &fixture.primary_plan, None).unwrap();
        let original = std::fs::read(&path).unwrap();
        let saved = dir.path().join("saved.jsonl");
        std::fs::rename(&path, &saved).unwrap();
        let error = submit_cex_order(
            &state,
            response.run_id,
            &claimed.config,
            &fixture.primary_plan,
            OnchainCexActionKind::Primary,
        )
        .await
        .unwrap_err();
        assert!(error.contains("写入失败"));
        let context = CompensationContext {
            state: &state,
            response,
            claimed: &claimed,
            plan: &fixture.primary_plan,
            reason: "test",
        };
        assert!(
            submit_compensation(context, &fixture.primary_plan, "primary")
                .await
                .unwrap_err()
                .contains("写入失败")
        );
        assert!(state
            .trading_service()
            .get_order_by_client_order_id(&fixture.primary_plan.client_order_id)
            .is_none());
        assert_eq!(std::fs::read(saved).unwrap(), original);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn execution_recovery_unknown_order_cannot_be_compensated_or_presented_as_zero_exposure()
    {
        let state = AppState::new(common::config::AppConfig::default())
            .await
            .unwrap();
        let fixture = crate::services::onchain_execution_run_store::test_checkpoint();
        let claimed = ClaimedOnchainBuild {
            response: fixture.build,
            config: fixture.config,
        };
        let response = ResponseContext {
            run_id: "unknown-run",
            build: &claimed.response,
            started_at_ms: 1,
        };
        let context = CompensationContext {
            state: &state,
            response,
            claimed: &claimed,
            plan: &fixture.primary_plan,
            reason: "unknown order",
        };
        let record = record(LiveOrderState::PartiallyFilled, Some(0.1));
        let run = compensate_or_expose(context, record.clone()).await;
        assert_eq!(run.status, OnchainExecutionRunStatus::FinalityUnresolved);
        assert!(run.compensation_order_id.is_none());
        assert!(
            reverse_filled_order(context, &fixture.primary_plan, &record, "primary")
                .await
                .is_err()
        );
    }

    #[test]
    fn full_fill_requires_terminal_state_and_the_complete_quantity() {
        let mut record = record(LiveOrderState::Filled, Some(1.0));
        assert!(fully_filled(&record, 1.0));
        record.filled_quantity = Some(0.5);
        assert!(!fully_filled(&record, 1.0));
        record.state = LiveOrderState::PartiallyFilled;
        record.filled_quantity = Some(1.0);
        assert!(!fully_filled(&record, 1.0));
    }

    #[test]
    fn unresolved_finality_tells_the_operator_not_to_resubmit() {
        let actions = recovery_actions(OnchainExecutionRunStatus::FinalityUnresolved);
        assert!(actions.iter().any(|action| {
            action.kind == OnchainExecutionRecoveryKind::DoNotResubmit && !action.automated
        }));
        assert!(actions
            .iter()
            .any(|action| action.kind == OnchainExecutionRecoveryKind::VerifyExposure));
    }

    #[test]
    fn completed_execution_has_no_recovery_work() {
        assert!(recovery_actions(OnchainExecutionRunStatus::Completed).is_empty());
    }

    fn record(state: LiveOrderState, filled_quantity: Option<f64>) -> OrderRecord {
        let intent = shared_types::OrderIntent {
            id: "order".to_owned(),
            source: OrderSource::Strategy,
            strategy: Some(shared_types::StrategyKind::OnchainDepeg),
            mode: shared_types::ExecutionMode::Live,
            exchange: "kraken".to_owned(),
            symbol: "SOL/USD".to_owned(),
            side: OrderSide::Sell,
            order_type: shared_types::OrderType::Market,
            quantity: 1.0,
            price: None,
            slippage_tolerance_bps: Some(10.0),
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 1,
        };
        OrderRecord {
            identity: shared_types::VenueOrderIdentity::from_intent(&intent),
            intent,
            state,
            risk: None,
            last_update_source: shared_types::OrderUpdateSource::Internal,
            exchange_order_id: Some("exchange".to_owned()),
            message: None,
            filled_quantity,
            filled_price: Some(100.0),
            filled_fee: None,
            updated_at_ms: 2,
        }
    }
}
