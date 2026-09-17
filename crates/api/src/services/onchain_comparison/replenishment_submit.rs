use axum::http::StatusCode;
use common::AppError;
use exchange::{
    DepositStatus, DepositStatusRequest, WithdrawalSourceBalanceRequest, WithdrawalStatusRequest,
    WithdrawalSubmitRequest, WithdrawalWalletType,
};
use rust_decimal::Decimal;
use shared_types::{
    OnchainComparisonDirection, OnchainReplenishmentDestinationStatus, OnchainReplenishmentLeg,
    OnchainReplenishmentPlanResponse, OnchainReplenishmentPlanStatus, OnchainReplenishmentRun,
    OnchainReplenishmentRunStatus, OnchainReplenishmentSubmitRequest,
    OnchainReplenishmentTransferStatus, OnchainTransferDirection, WebhookEventKind,
};

use crate::services::onchain_replenishment_plan_store::ReplenishmentSubmitClaimError;
use crate::state::AppState;

use super::replenishment_credit::DestinationCreditCheck;

const SOURCE_STATUS_RECHECK_MS: i64 = 60_000;
const SOURCE_HISTORY_GRACE_MS: i64 = 10 * 60_000;
use shared_types::{ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS, ONCHAIN_REPLENISHMENT_DESTINATION_WAIT_MS};

mod wait_policy;

struct PreparedWithdrawal {
    venue: String,
    currency: String,
    network: String,
    address: String,
    tag: Option<String>,
    amount: Decimal,
    max_fee: Decimal,
}

enum PreparedTransfer {
    ExchangeWithdrawal(PreparedWithdrawal),
    ChainDeposit(super::replenishment_chain_transfer::PreparedChainDeposit),
}

pub(crate) async fn submit(
    state: &AppState,
    request: &OnchainReplenishmentSubmitRequest,
    actor: &str,
) -> Result<OnchainReplenishmentRun, AppError> {
    state
        .onchain_replenishment_plans()
        .readiness()
        .map_err(|problem| conflict("ONCHAIN_REPLENISHMENT_RECOVERY_UNAVAILABLE", problem))?;
    let run_id = request.run_id.trim();
    if run_id.is_empty() {
        return Err(bad_request(
            "ONCHAIN_REPLENISHMENT_RUN_INVALID",
            "runId 不能为空",
        ));
    }
    let now_ms = common::time::now_ms();
    let authorized = state
        .onchain_replenishment_plans()
        .run(run_id, now_ms)
        .ok_or_else(|| not_found("ONCHAIN_REPLENISHMENT_RUN_MISSING", "补仓运行记录不存在"))?;
    if authorized.read_only_recovery {
        return Err(conflict("ONCHAIN_REPLENISHMENT_READ_ONLY_RECOVERY", "本次仅核验原转账；继续资金动作需要重新规划并授权"));
    }
    match authorized.status {
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit
        | OnchainReplenishmentRunStatus::ReadyForNextTransfer => {}
        OnchainReplenishmentRunStatus::AuthorizationExpired => {
            return Err(conflict(
                "ONCHAIN_REPLENISHMENT_AUTHORIZATION_EXPIRED",
                "实盘补仓授权已过期，请重新生成计划并明确授权",
            ));
        }
        _ => return Ok(authorized),
    }

    let leg_index = authorized.transfers.len();
    let source_snapshot = state.onchain_replenishment_plans().submission_snapshot();
    let leg = authorized.plan.legs.get(leg_index)
        .ok_or_else(|| map_claim_error(ReplenishmentSubmitClaimError::InvalidLeg))?;
    source_snapshot.ensure_available(leg).map_err(map_claim_error)?;
    let mut current_plan = revalidate_plan(state, &authorized).await?;
    let prepared = prepare_transfer(state, &current_plan, leg_index).await?;
    current_plan.submit_ready = true;
    let claim = state
        .onchain_replenishment_plans()
        .claim_submission(
            run_id,
            actor,
            current_plan,
            leg_index,
            &source_snapshot,
            common::time::now_ms(),
        )
        .map_err(map_claim_error)?;
    if claim.replayed {
        return Ok(claim.run);
    }
    let transfer = claim
        .run
        .transfers
        .last()
        .ok_or_else(|| conflict("ONCHAIN_REPLENISHMENT_CLAIM_MISSING", "提交占位未写入"))?;
    let updated = match prepared {
        PreparedTransfer::ExchangeWithdrawal(prepared) => {
            submit_exchange_withdrawal(state, run_id, transfer.client_transfer_id.clone(), prepared)
                .await?
        }
        PreparedTransfer::ChainDeposit(prepared) => {
            submit_chain_deposit(state, run_id, prepared).await?
        }
    };
    let updated =
        super::replenishment_costs::fill_from_ws(state, updated).map_err(durability_error)?;
    emit_replenishment_webhook(state, &updated).await;
    Ok(updated)
}

async fn submit_exchange_withdrawal(
    state: &AppState,
    run_id: &str,
    client_transfer_id: String,
    prepared: PreparedWithdrawal,
) -> Result<OnchainReplenishmentRun, AppError> {
    let withdrawal = WithdrawalSubmitRequest {
        venue: prepared.venue,
        currency: prepared.currency,
        network: prepared.network,
        address: prepared.address,
        tag: prepared.tag,
        amount: prepared.amount,
        client_withdrawal_id: client_transfer_id,
        wallet_type: WithdrawalWalletType::Spot,
        max_fee: Some(prepared.max_fee),
    };
    match state.trading_service().submit_withdrawal(&withdrawal).await {
        Ok(submission) => record_withdrawal_submission(state.onchain_replenishment_plans(), run_id, submission, common::time::now_ms()).map_err(durability_error),
        Err(exchange::ExchangeError::Api { exchange, code, message })
            if exchange == "kraken" && code == "LOCAL_WITHDRAWAL_PREFLIGHT_REJECTED" => state.onchain_replenishment_plans()
                .reject_before_send(run_id, format!("提币前检查未通过，未发送提币请求：{message}；请重新核对计划后授权"), common::time::now_ms())
                .map_err(durability_error),
        Err(error) => state
            .onchain_replenishment_plans()
            .pause_submission(
                run_id,
                format!(
                    "交易所提币提交结果不确定，已禁止自动重试：{error}；请按客户端提币号 {} 核对历史",
                    withdrawal.client_withdrawal_id
                ),
                common::time::now_ms(),
            )
            .map_err(durability_error),
    }
}

fn record_withdrawal_submission(
    store: &crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore,
    run_id: &str,
    submission: exchange::WithdrawalSubmission,
    now_ms: i64,
) -> Result<OnchainReplenishmentRun, String> {
    let updated = store.record_submission_ack(run_id, submission.provider_withdrawal_id, None,
        submission.submitted_at_ms, submission.source_url, now_ms)?;
    match submission.problem {
        Some(problem) => store.pause_submission(run_id, problem, now_ms),
        None => Ok(updated),
    }
}

async fn submit_chain_deposit(
    state: &AppState,
    run_id: &str,
    prepared: super::replenishment_chain_transfer::PreparedChainDeposit,
) -> Result<OnchainReplenishmentRun, AppError> {
    let transaction_id = prepared.transaction_id().to_owned();
    let source = prepared.evidence_source().to_owned();
    let intent = state
        .onchain_replenishment_plans()
        .record_chain_submission_intent(
            run_id,
            transaction_id.clone(),
            source.clone(),
            common::time::now_ms(),
        )
        .map_err(durability_error)?;
    let outcome = prepared.broadcast().await;
    apply_chain_submission_outcome(state, &intent, outcome, &source, common::time::now_ms())
        .await
        .map_err(durability_error)
}

async fn apply_chain_submission_outcome(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    outcome: super::execution_submit::IndependentChainOutcome,
    source: &str,
    now_ms: i64,
) -> Result<OnchainReplenishmentRun, String> {
    use super::execution_submit::IndependentChainOutcome;

    match outcome {
        IndependentChainOutcome::Confirmed { transaction_id } => {
            let leg = active_leg(run)?;
            let (credit, cost) =
                super::replenishment_credit::check_with_cost(state, leg, &transaction_id).await;
            match credit {
                DestinationCreditCheck::Credited {
                    confirmations,
                    source,
                    ..
                } => state
                    .onchain_replenishment_plans()
                    .record_chain_source_status(
                        &run.run_id,
                        transaction_id.clone(),
                        OnchainReplenishmentTransferStatus::SourceCompleted,
                        confirmations,
                        source,
                        None,
                        cost,
                        now_ms,
                    ),
                DestinationCreditCheck::Pending {
                    confirmations,
                    source,
                    problem,
                } => state
                    .onchain_replenishment_plans()
                    .record_chain_source_status(
                        &run.run_id,
                        transaction_id.clone(),
                        OnchainReplenishmentTransferStatus::Submitted,
                        confirmations,
                        source,
                        Some(problem),
                        cost,
                        now_ms,
                    ),
                DestinationCreditCheck::Rejected { source, problem } => state
                    .onchain_replenishment_plans()
                    .record_chain_source_status(
                        &run.run_id,
                        transaction_id.clone(),
                        OnchainReplenishmentTransferStatus::Paused,
                        None,
                        source,
                        Some(problem),
                        cost,
                        now_ms,
                    ),
            }
        }
        IndependentChainOutcome::Pending {
            transaction_id,
            problem,
        } => state.onchain_replenishment_plans().record_source_status(
            &run.run_id,
            OnchainReplenishmentTransferStatus::Submitted,
            transaction_id.clone(),
            Some(transaction_id),
            None,
            source.to_owned(),
            Some(problem),
            now_ms,
        ),
        IndependentChainOutcome::Rejected {
            transaction_id,
            problem,
        } => {
            let (_, cost) = super::replenishment_credit::check_with_cost(
                state,
                active_leg(run)?,
                &transaction_id,
            )
            .await;
            state
                .onchain_replenishment_plans()
                .record_chain_source_status(
                    &run.run_id,
                    transaction_id.clone(),
                    OnchainReplenishmentTransferStatus::Failed,
                    None,
                    source.to_owned(),
                    Some(problem),
                    cost,
                    now_ms,
                )
        }
    }
}

pub(crate) async fn reconcile_pending(state: &AppState, now_ms: i64) {
    if state.onchain_replenishment_plans().readiness().is_err() {
        return;
    }
    let rows = state.onchain_replenishment_plans().runs(128, now_ms).rows;
    let assets = super::replenishment_costs::missing_assets(&rows);
    if !assets.is_empty() {
        super::usd_valuation::refresh_assets(
            state,
            &state.onchain_monitor().snapshot().config,
            &assets,
        )
        .await;
    }
    for run in rows {
        let run = match super::replenishment_costs::fill_from_ws(state, run) {
            Ok(run) => run,
            Err(error) => {
                tracing::warn!(%error, "replenishment cost valuation persistence failed");
                continue;
            }
        };
        let run = if run.read_only_recovery {
            match state.onchain_replenishment_plans().claim_recovery_check(&run.run_id, now_ms) {
                Ok(Some(checked)) if checked.status == OnchainReplenishmentRunStatus::Paused => {
                    emit_replenishment_webhook(state, &checked).await;
                    continue;
                }
                Ok(Some(checked)) => checked,
                Ok(None) => continue,
                Err(error) => { tracing::warn!(%error, "replenishment recovery check could not be persisted"); continue; }
            }
        } else { run };
        let result = match run.status {
            OnchainReplenishmentRunStatus::ReadyForNextTransfer
                if super::replenishment_costs::waiting_for_valuation(&run) =>
            {
                continue
            }
            OnchainReplenishmentRunStatus::ReadyForNextTransfer => {
                continue_next_transfer(state, &run, now_ms).await
            }
            OnchainReplenishmentRunStatus::Submitting
            | OnchainReplenishmentRunStatus::AwaitingSourceFinality
                if run.read_only_recovery || check_due(&run, now_ms) =>
            {
                reconcile_source(state, &run, now_ms).await
            }
            OnchainReplenishmentRunStatus::AwaitingDestinationCredit if run.read_only_recovery || check_due(&run, now_ms) => {
                reconcile_destination(state, &run, now_ms).await
            }
            _ => continue,
        };
        if let Err(error) = result {
            tracing::warn!(run_id = %run.run_id, %error, "on-chain replenishment reconciliation failed");
        }
    }
}

pub(crate) fn recheck(
    state: &AppState,
    request: &shared_types::OnchainReplenishmentRecheckRequest,
    actor: &str,
) -> Result<OnchainReplenishmentRun, AppError> {
    state.onchain_replenishment_plans().request_recheck(request, actor, common::time::now_ms())
        .map_err(|p| conflict("ONCHAIN_REPLENISHMENT_RECHECK_REJECTED", p))
}

async fn continue_next_transfer(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    now_ms: i64,
) -> Result<(), String> {
    if run.read_only_recovery { return Err("只读核验不能提交下一条资金腿".into()); }
    let request = OnchainReplenishmentSubmitRequest {
        run_id: run.run_id.clone(),
    };
    if let Err(error) = submit(state, &request, &run.authorization.actor).await {
        let paused = state
            .onchain_replenishment_plans()
            .pause_before_next_transfer(
                &run.run_id,
                format!("下一条已授权资金腿重新核验失败：{error}"),
                now_ms,
            )?;
        emit_replenishment_webhook(state, &paused).await;
    }
    Ok(())
}

async fn revalidate_plan(
    state: &AppState,
    run: &OnchainReplenishmentRun,
) -> Result<OnchainReplenishmentPlanResponse, AppError> {
    let run =
        super::replenishment_costs::fill_from_ws(state, run.clone()).map_err(durability_error)?;
    let snapshot = super::snapshot(state);
    let request = shared_types::OnchainReplenishmentBuildRequest {
        direction: run.plan.direction,
        expected_quote_observed_at_ms: snapshot.quote_observed_at_ms.ok_or_else(|| {
            conflict(
                "ONCHAIN_REPLENISHMENT_QUOTE_MISSING",
                "链上报价缺失，不能重新核验补仓计划",
            )
        })?,
        expected_cex_observed_at_ms: snapshot.cex_observed_at_ms.ok_or_else(|| {
            conflict(
                "ONCHAIN_REPLENISHMENT_CEX_QUOTE_MISSING",
                "CEX 报价缺失，不能重新核验补仓计划",
            )
        })?,
    };
    let current = super::replenishment::build(state, &request).await?;
    if current.status != OnchainReplenishmentPlanStatus::ReadyForAuthorization
        || !current.blockers.is_empty()
    {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_REVALIDATION_BLOCKED",
            current
                .blockers
                .first()
                .cloned()
                .unwrap_or_else(|| "最新补仓计划未达到实盘条件".to_owned()),
        ));
    }
    revalidated_remaining_plan(&run, current)
}

fn revalidated_remaining_plan(
    run: &OnchainReplenishmentRun,
    mut current: OnchainReplenishmentPlanResponse,
) -> Result<OnchainReplenishmentPlanResponse, AppError> {
    let authorized = &run.plan;
    let completed = run.transfers.len();
    if completed >= authorized.legs.len()
        || run.transfers.iter().enumerate().any(|(index, transfer)| {
            transfer.leg_index as usize != index
                || transfer.status != OnchainReplenishmentTransferStatus::DestinationCredited
                || transfer.withdrawal_unlocked == Some(false)
                || exact_decimal(transfer.credited_amount_exact.as_deref())
                    .zip(exact_decimal(
                        authorized.legs[index].transfer_amount_exact.as_deref(),
                    ))
                    .is_none_or(|(actual, expected)| actual < expected)
        })
    {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_PREVIOUS_LEG_INCOMPLETE",
            "前序补仓尚未逐条确认到账，不能继续下一步",
        ));
    }
    validate_completed_withdrawal_fees(run)?;
    let remaining = &authorized.legs[completed..];
    if authorized.direction != current.direction
        || remaining.len() != current.legs.len()
        || !remaining
            .iter()
            .zip(&current.legs)
            .all(|(left, right)| same_leg_scope(left, right))
    {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_SCOPE_CHANGED",
            "补仓资产、网络、数量或目标地址已变化，需要重新明确授权",
        ));
    }
    let authorized_profit = authorized
        .post_transfer_net_profit_usd
        .filter(|profit| profit.is_finite() && *profit > 0.0)
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_REPLENISHMENT_AUTHORIZED_ECONOMICS_MISSING",
                "原授权计划缺少搬运后净收益证据",
            )
        })?;
    let costs = completed_leg_costs(run)
        .map_err(|problem| conflict("ONCHAIN_REPLENISHMENT_COMPLETED_COST_MISSING", problem))?;
    let spent_cost = costs.iter().sum::<f64>();
    let remaining_cost = current
        .transfer_cost_usd
        .filter(|cost| cost.is_finite() && *cost >= 0.0)
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_REPLENISHMENT_CURRENT_ECONOMICS_MISSING",
                "剩余补仓费用缺失或无效，不能继续",
            )
        })?;
    if estimated_leg_cost(&current.legs).is_none_or(|cost| {
        (cost - remaining_cost).abs() > f64::EPSILON * remaining_cost.abs().max(1.0) * 8.0
    }) {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_CURRENT_ECONOMICS_MISSING",
            "剩余补仓逐条费用与合计不一致，不能继续",
        ));
    }
    let current_profit = current
        .post_transfer_net_profit_usd
        .filter(|profit| profit.is_finite())
        .map(|profit| profit - spent_cost)
        .filter(|profit| profit.is_finite())
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_REPLENISHMENT_CURRENT_ECONOMICS_MISSING",
                "最新计划缺少搬运后净收益证据",
            )
        })?;
    if current_profit <= 0.0 || current_profit + f64::EPSILON < authorized_profit {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_ECONOMICS_WORSENED",
            "计入已完成补仓费用后，最新净收益低于授权时水平，需要重新确认",
        ));
    }
    let total_cost = spent_cost + remaining_cost;
    if !total_cost.is_finite() {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_CURRENT_ECONOMICS_MISSING",
            "累计补仓费用无效，不能继续",
        ));
    }
    // Retain the completed prefix and stable leg indices in the durable run.
    // Fresh previews contain only the remaining balance shortfalls.
    let mut legs = authorized.legs[..completed].to_vec();
    for (leg, cost) in legs.iter_mut().zip(costs) {
        leg.economics.reconciled_cost_usd = Some(cost);
    }
    legs.extend(current.legs);
    current.legs = legs;
    current.plan_id = authorized.plan_id.clone();
    current.transfer_cost_usd = Some(total_cost);
    current.post_transfer_net_profit_usd = Some(current_profit);
    Ok(current)
}

fn completed_leg_costs(run: &OnchainReplenishmentRun) -> Result<Vec<f64>, String> {
    run.plan
        .legs
        .iter()
        .zip(&run.transfers)
        .map(|(leg, transfer)| {
            let estimate = leg
                .economics
                .estimated_cost_usd
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
                .ok_or("前序费用缺少原预算记录，不能按零费用继续")?;
            if leg.direction != OnchainTransferDirection::DepositToCex {
                let cost = transfer.withdrawal_cost.as_ref()
                    .filter(|cost| cost.asset.eq_ignore_ascii_case(&leg.asset))
                    .ok_or("前序提币缺少同一资产的已确认实扣费用")?;
                return super::replenishment_costs::withdrawal_fee_usd(cost);
            }
            let budget = leg
                .economics
                .estimated_network_cost_usd
                .filter(|cost| cost.is_finite() && *cost >= 0.0 && *cost <= estimate)
                .ok_or("前序链上转账缺少独立 Gas 预算，不能重复扣费或覆盖其他费用；需核对原记录")?;
            let actual = transfer
                .network_cost
                .as_ref()
                .ok_or("前序链上转账的实扣网络费待核验")?;
            let same = |left: &str, right: &str| {
                if leg.chain == "solana" {
                    left == right
                } else {
                    left.eq_ignore_ascii_case(right)
                }
            };
            if actual.chain != leg.chain
                || shared_types::onchain_chain_preset(&leg.chain)
                    .is_none_or(|preset| actual.asset != preset.base_token)
                || leg
                    .source_address
                    .as_deref()
                    .is_none_or(|payer| !same(payer, &actual.payer))
                || transfer
                    .transaction_id
                    .as_deref()
                    .is_none_or(|hash| !same(hash, &actual.transaction_id))
            {
                return Err("前序网络费与该笔交易或付款钱包不一致，不能借用其他交易的费用".into());
            }
            let amount = super::replenishment_costs::network_fee_usd(actual)?;
            let cost = estimate - budget + amount;
            if !cost.is_finite() || cost < 0.0 {
                return Err("前序费用合计无效".into());
            }
            Ok(cost)
        })
        .collect()
}

fn estimated_leg_cost(legs: &[OnchainReplenishmentLeg]) -> Option<f64> {
    legs.iter().try_fold(0.0_f64, |cost, leg| {
        leg.economics
            .estimated_cost_usd
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| cost + value)
            .filter(|value| value.is_finite())
    })
}

fn validate_completed_withdrawal_fees(run: &OnchainReplenishmentRun) -> Result<(), AppError> {
    for (leg, transfer) in run.plan.legs.iter().zip(&run.transfers) {
        if leg.direction != OnchainTransferDirection::WithdrawToChain {
            continue;
        }
        let fee = transfer
            .withdrawal_cost
            .as_ref()
            .filter(|cost| cost.confirmed && cost.asset.eq_ignore_ascii_case(&leg.asset))
            .and_then(|cost| cost.fee_exact.parse::<Decimal>().ok())
            .filter(|fee| *fee >= Decimal::ZERO);
        let planned = leg
            .economics
            .fee_amount_exact
            .as_deref()
            .and_then(|value| value.parse::<Decimal>().ok())
            .filter(|fee| *fee >= Decimal::ZERO);
        let (Some(fee), Some(planned)) = (fee, planned) else {
            return Err(conflict(
                "ONCHAIN_REPLENISHMENT_WITHDRAWAL_FEE_UNPROVEN",
                "前序提币缺少已确认实扣费用或原计划费用，不能按零手续费继续；请核对原提币回执",
            ));
        };
        if fee > planned {
            return Err(conflict(
                "ONCHAIN_REPLENISHMENT_WITHDRAWAL_FEE_EXCEEDED",
                format!(
                    "前序实扣提币费 {} {} 高于原计划 {}；不能沿用旧估算继续，需要重新确认",
                    fee.normalize(),
                    leg.asset,
                    planned.normalize()
                ),
            ));
        }
    }
    Ok(())
}

fn same_leg_scope(left: &OnchainReplenishmentLeg, right: &OnchainReplenishmentLeg) -> bool {
    left.direction == right.direction
        && shared_types::venue_names_equal(&left.venue, &right.venue)
        && left.asset.eq_ignore_ascii_case(&right.asset)
        && left.chain.eq_ignore_ascii_case(&right.chain)
        && left.source_address == right.source_address
        && left.asset_address == right.asset_address
        && left.asset_decimals == right.asset_decimals
        && left.transfer_amount_exact == right.transfer_amount_exact
        && left.network_evidence.network == right.network_evidence.network
        && left.network_evidence.amount_step == right.network_evidence.amount_step
        && left.destination.address == right.destination.address
        && left.destination.tag == right.destination.tag
        && debit_not_increased(left, right)
}

fn debit_not_increased(left: &OnchainReplenishmentLeg, right: &OnchainReplenishmentLeg) -> bool {
    let Some(left) = exact_decimal(left.economics.source_debit_upper_bound_exact.as_deref()) else {
        return false;
    };
    let Some(right) = exact_decimal(right.economics.source_debit_upper_bound_exact.as_deref())
    else {
        return false;
    };
    right <= left
}

async fn prepare_transfer(
    state: &AppState,
    plan: &OnchainReplenishmentPlanResponse,
    leg_index: usize,
) -> Result<PreparedTransfer, AppError> {
    if plan.direction != OnchainComparisonDirection::BuyOnchainSellCex
        && plan.direction != OnchainComparisonDirection::BuyCexSellOnchain
    {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_DIRECTION_UNSUPPORTED",
            "当前方向不属于链上/CEX 补仓流程",
        ));
    }
    let leg = plan.legs.get(leg_index).ok_or_else(|| {
        conflict(
            "ONCHAIN_REPLENISHMENT_LEG_INVALID",
            "当前计划没有下一条已授权资金腿",
        )
    })?;
    if leg.destination.status != OnchainReplenishmentDestinationStatus::Verified {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_DESTINATION_UNVERIFIED",
            "目标地址未通过交易所官方地址/白名单证据核验",
        ));
    }
    if leg.asset_address.as_deref().is_none_or(str::is_empty) || leg.asset_decimals.is_none() {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_ASSET_IDENTITY_MISSING",
            "补仓资产缺少链上合约/Mint 或精度，无法核验最终到账",
        ));
    }
    validate_risk_scope(state, leg)?;
    let amount = exact_decimal(leg.transfer_amount_exact.as_deref()).ok_or_else(|| {
        conflict(
            "ONCHAIN_REPLENISHMENT_AMOUNT_NOT_EXACT",
            "补仓数量没有通过网络步长或代币精度编译",
        )
    })?;
    match leg.direction {
        OnchainTransferDirection::WithdrawToChain => {
            prepare_exchange_withdrawal(state, leg, amount).await
        }
        OnchainTransferDirection::DepositToCex => {
            if !state.trading_service().deposit_status_supported(&leg.venue) {
                return Err(conflict(
                    "ONCHAIN_REPLENISHMENT_DEPOSIT_FINALITY_UNSUPPORTED",
                    format!("{} 尚未接入官方充值历史终态核验", leg.venue),
                ));
            }
            let config = super::snapshot(state).config.clone();
            super::replenishment_chain_transfer::prepare(state, &config, leg)
                .await
                .map(PreparedTransfer::ChainDeposit)
                .map_err(|problem| {
                    conflict("ONCHAIN_REPLENISHMENT_CHAIN_TRANSFER_NOT_READY", problem)
                })
        }
    }
}

async fn prepare_exchange_withdrawal(
    state: &AppState,
    leg: &OnchainReplenishmentLeg,
    amount: Decimal,
) -> Result<PreparedTransfer, AppError> {
    if !state
        .trading_service()
        .withdrawal_submission_supported(&leg.venue)
    {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_VENUE_WRITE_UNSUPPORTED",
            format!("{} 尚未接入官方提币写入与终态查询", leg.venue),
        ));
    }
    let source_debit = exact_decimal(leg.economics.source_debit_upper_bound_exact.as_deref())
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_REPLENISHMENT_DEBIT_NOT_EXACT",
                "提币数量与手续费无法编译为精确来源扣款上限",
            )
        })?;
    let wallet_label = if shared_types::venue_family(&leg.venue) == "bybit" {
        "UTA 统一账户（不含资金/理财账户）"
    } else { "现货钱包" };
    let balance = state
        .trading_service()
        .withdrawal_source_balance(&WithdrawalSourceBalanceRequest {
            venue: leg.venue.clone(),
            currency: leg.asset.clone(),
            wallet_type: WithdrawalWalletType::Spot,
        })
        .await
        .map_err(|error| {
            upstream(
                "ONCHAIN_REPLENISHMENT_SOURCE_BALANCE_UNAVAILABLE",
                format!("无法读取提币所使用的{wallet_label}可提余额：{error}"),
            )
        })?;
    if balance.available < source_debit {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_SOURCE_BALANCE_INSUFFICIENT",
            format!(
                "{} {} {wallet_label}可提 {}，低于本次最高扣款 {}",
                leg.venue,
                leg.asset,
                balance.available.normalize(),
                source_debit.normalize()
            ),
        ));
    }
    Ok(PreparedTransfer::ExchangeWithdrawal(PreparedWithdrawal {
        venue: leg.venue.clone(),
        currency: leg.asset.clone(),
        network: leg
            .network_evidence
            .network
            .clone()
            .ok_or_else(|| conflict("ONCHAIN_REPLENISHMENT_NETWORK_MISSING", "提币网络标识缺失"))?,
        address: leg
            .destination
            .address
            .clone()
            .ok_or_else(|| conflict("ONCHAIN_REPLENISHMENT_ADDRESS_MISSING", "提币目标地址缺失"))?,
        tag: leg.destination.tag.clone(),
        amount,
        max_fee: leg.economics.fee_amount_exact.as_deref()
            .and_then(|value| Decimal::from_str_exact(value).ok())
            .filter(|fee| *fee >= Decimal::ZERO)
            .ok_or_else(|| conflict("ONCHAIN_REPLENISHMENT_FEE_MISSING", "提币手续费上限未核实"))?,
    }))
}

fn validate_risk_scope(state: &AppState, leg: &OnchainReplenishmentLeg) -> Result<(), AppError> {
    let risk = state.trading_service().risk_config();
    if !risk.live_trading_enabled {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_LIVE_DISABLED",
            "实盘总开关未开启，已阻止资金动作",
        ));
    }
    if risk.kill_switch_active {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_KILL_SWITCH_ACTIVE",
            "全局急停已开启，已阻止资金动作",
        ));
    }
    if !trading::exchange_allowed(&risk.allowed_exchanges, &leg.venue) {
        return Err(conflict(
            "ONCHAIN_REPLENISHMENT_VENUE_NOT_ALLOWED",
            "该交易所不在当前实盘白名单内",
        ));
    }
    Ok(())
}

fn check_due(run: &OnchainReplenishmentRun, now_ms: i64) -> bool {
    run.transfers.last().is_some_and(|transfer| {
        transfer
            .last_checked_at_ms
            .is_none_or(|checked| now_ms.saturating_sub(checked) >= SOURCE_STATUS_RECHECK_MS)
            && now_ms.saturating_sub(transfer.submission_attempted_at_ms)
                >= SOURCE_STATUS_RECHECK_MS
    })
}

async fn reconcile_source(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    now_ms: i64,
) -> Result<(), String> {
    match active_leg(run)?.direction {
        OnchainTransferDirection::WithdrawToChain => {
            reconcile_withdrawal_source(state, run, now_ms).await
        }
        OnchainTransferDirection::DepositToCex => reconcile_chain_source(state, run, now_ms).await,
    }
}

async fn reconcile_withdrawal_source(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    now_ms: i64,
) -> Result<(), String> {
    let request = withdrawal_status_request(run)?;
    let outcome = state.trading_service().withdrawal_status(&request).await;
    let updated = wait_policy::record_withdrawal_check(
        state.onchain_replenishment_plans(), run, outcome, now_ms,
    )?;
    if updated.status != run.status {
        emit_replenishment_webhook(state, &updated).await;
    }
    Ok(())
}

async fn reconcile_chain_source(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    now_ms: i64,
) -> Result<(), String> {
    let transfer = active_transfer(run)?;
    let transaction_id = transfer
        .transaction_id
        .as_deref()
        .ok_or_else(|| "chain replenishment transaction id is missing".to_owned())?;
    let (outcome, cost) =
        super::replenishment_credit::check_with_cost(state, active_leg(run)?, transaction_id).await;
    let updated = match outcome {
        DestinationCreditCheck::Credited {
            confirmations,
            source,
            ..
        } => state
            .onchain_replenishment_plans()
            .record_chain_source_status(
                &run.run_id,
                transaction_id.to_owned(),
                OnchainReplenishmentTransferStatus::SourceCompleted,
                confirmations,
                source,
                None,
                cost,
                now_ms,
            ),
        DestinationCreditCheck::Rejected { source, problem } => state
            .onchain_replenishment_plans()
            .record_chain_source_status(
                &run.run_id,
                transaction_id.to_owned(),
                OnchainReplenishmentTransferStatus::Paused,
                None,
                source,
                Some(problem),
                cost,
                now_ms,
            ),
        DestinationCreditCheck::Pending {
            confirmations,
            source,
            problem,
        } if cost.is_some() => {
            let timed_out = wait_policy::expired(run, ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS, now_ms);
            state
                .onchain_replenishment_plans()
                .record_chain_source_status(
                    &run.run_id,
                    transaction_id.to_owned(),
                    if timed_out {
                        OnchainReplenishmentTransferStatus::Paused
                    } else {
                        OnchainReplenishmentTransferStatus::Submitted
                    },
                    confirmations,
                    source,
                    Some(if timed_out {
                        format!("链上交易超过本次 10 分钟自动核验窗口：{problem}；已保留交易哈希与实扣费用")
                    } else { problem }),
                    cost,
                    now_ms,
                )
        }
        DestinationCreditCheck::Pending {
            confirmations: _,
            source: _,
            problem,
        } if wait_policy::expired(run, ONCHAIN_REPLENISHMENT_CHAIN_WAIT_MS, now_ms) =>
        {
            state.onchain_replenishment_plans().pause_submission(
                &run.run_id,
                format!("链上交易超过本次 10 分钟自动核验窗口：{problem}；保留原交易，未重新广播"),
                now_ms,
            )
        }
        DestinationCreditCheck::Pending {
            confirmations,
            source,
            problem,
        } => state
            .onchain_replenishment_plans()
            .record_source_check_problem(
                &run.run_id,
                format!(
                    "链上转账仍在等待：{problem} · confirmations={} · source {source}",
                    confirmations.map_or_else(|| "--".to_owned(), |value| value.to_string())
                ),
                now_ms,
            ),
    }?;
    if updated.status != run.status {
        emit_replenishment_webhook(state, &updated).await;
    }
    Ok(())
}

async fn reconcile_destination(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    now_ms: i64,
) -> Result<(), String> {
    match active_leg(run)?.direction {
        OnchainTransferDirection::WithdrawToChain => {
            reconcile_chain_destination(state, run, now_ms).await
        }
        OnchainTransferDirection::DepositToCex => {
            reconcile_exchange_destination(state, run, now_ms).await
        }
    }
}

async fn reconcile_chain_destination(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    now_ms: i64,
) -> Result<(), String> {
    let current = refresh_transaction_id_if_missing(state, run, now_ms).await?;
    if current.status != OnchainReplenishmentRunStatus::AwaitingDestinationCredit {
        if current.status != run.status {
            emit_replenishment_webhook(state, &current).await;
        }
        return Ok(());
    }
    let transfer = active_transfer(&current)?;
    let Some(transaction_id) = transfer.transaction_id.as_deref() else {
        let problem = "交易所已报告提币完成，但尚未提供链上交易哈希".to_owned();
        let updated =
            record_destination_pending(state.onchain_replenishment_plans(), &current, None, "exchange_history", problem, now_ms)?;
        if updated.status != current.status {
            emit_replenishment_webhook(state, &updated).await;
        }
        return Ok(());
    };
    let leg = active_leg(&current)?;
    let outcome = super::replenishment_credit::check(state, leg, transaction_id).await;
    let updated = match outcome {
        DestinationCreditCheck::Credited {
            credited_amount_raw,
            confirmations,
            source,
        } => state
            .onchain_replenishment_plans()
            .record_destination_credit(
                &current.run_id,
                credit_units(credited_amount_raw, leg.asset_decimals)?,
                None,
                confirmations,
                source,
                now_ms,
            )?,
        DestinationCreditCheck::Pending {
            confirmations,
            source,
            problem,
        } => record_destination_pending(state.onchain_replenishment_plans(), &current, confirmations, &source, problem, now_ms)?,
        DestinationCreditCheck::Rejected { source, problem } => {
            state.onchain_replenishment_plans().pause_submission(
                &current.run_id,
                format!("链上到账证据与补仓计划不一致：{problem} · source {source}"),
                now_ms,
            )?
        }
    };
    if updated.status != current.status {
        emit_replenishment_webhook(state, &updated).await;
    }
    Ok(())
}

async fn reconcile_exchange_destination(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    now_ms: i64,
) -> Result<(), String> {
    let request = deposit_status_request(run)?;
    let updated = match state.trading_service().deposit_status(&request).await {
        Ok(Some(evidence)) => match evidence.status {
            DepositStatus::Pending => record_destination_pending(
                state.onchain_replenishment_plans(),
                run,
                evidence.confirmations,
                &evidence.source_url,
                evidence
                    .problem
                    .unwrap_or_else(|| "交易所官方充值历史仍为 pending".to_owned()),
                now_ms,
            ),
            DepositStatus::CreditedLocked | DepositStatus::Completed => {
                record_cex_credit(state.onchain_replenishment_plans(), run, &evidence, now_ms)
            }
            DepositStatus::Blocked => state.onchain_replenishment_plans().pause_submission(
                &run.run_id,
                evidence
                    .problem
                    .unwrap_or_else(|| "交易所充值需要人工处理".to_owned()),
                now_ms,
            ),
            DepositStatus::Failed => state
                .onchain_replenishment_plans()
                .record_destination_failure(
                    &run.run_id,
                    evidence.confirmations,
                    evidence.source_url,
                    evidence
                        .problem
                        .unwrap_or_else(|| "交易所官方充值记录报告终态失败".to_owned()),
                    now_ms,
                ),
        },
        Ok(None) => record_destination_pending(
            state.onchain_replenishment_plans(),
            run,
            None,
            "exchange_deposit_history",
            "交易所官方充值历史尚未索引该链上交易哈希".to_owned(),
            now_ms,
        ),
        Err(error) => record_destination_pending(
            state.onchain_replenishment_plans(),
            run,
            None,
            "exchange_deposit_history",
            format!("交易所充值历史查询失败：{error}"),
            now_ms,
        ),
    }?;
    if updated.status != run.status
        || updated
            .transfers
            .last()
            .and_then(|row| row.withdrawal_unlocked)
            != run.transfers.last().and_then(|row| row.withdrawal_unlocked)
    {
        emit_replenishment_webhook(state, &updated).await;
    }
    Ok(())
}

fn record_cex_credit(
    store: &crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore,
    run: &OnchainReplenishmentRun,
    evidence: &exchange::DepositStatusEvidence,
    now_ms: i64,
) -> Result<OnchainReplenishmentRun, String> {
    let updated = store.record_cex_destination_credit(&run.run_id, evidence, now_ms)?;
    if updated.status == OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        && updated.transfers.last().is_some_and(|transfer| transfer.withdrawal_unlocked == Some(false))
        && wait_policy::expired(&updated, ONCHAIN_REPLENISHMENT_DESTINATION_WAIT_MS, now_ms) {
        return store.pause_submission(&run.run_id,
            "交易所已入账可交易，但提币解锁超过本次 2 小时自动核验窗口；保留到账记录，仅暂停自动核验".into(), now_ms);
    }
    Ok(updated)
}

fn credit_units(raw: u128, decimals: Option<u8>) -> Result<Decimal, String> {
    let decimals = decimals.ok_or_else(|| "到账数量缺少代币精度".to_owned())?;
    let raw = i128::try_from(raw).map_err(|_| "到账数量超出精确记账范围".to_owned())?;
    Decimal::try_from_i128_with_scale(raw, u32::from(decimals))
        .map_err(|_| "到账数量或精度超出精确记账范围".to_owned())
}

async fn refresh_transaction_id_if_missing(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    now_ms: i64,
) -> Result<OnchainReplenishmentRun, String> {
    if active_transfer(run)?.transaction_id.is_some() {
        return Ok(run.clone());
    }
    let request = withdrawal_status_request(run)?;
    match state.trading_service().withdrawal_status(&request).await {
        Ok(Some(evidence)) => apply_source_evidence(state, run, evidence, now_ms),
        Ok(None) => Ok(run.clone()),
        Err(error) => state
            .onchain_replenishment_plans()
            .record_destination_check_problem(
                &run.run_id,
                None,
                "exchange_history".to_owned(),
                format!("交易所提币哈希刷新失败：{error}"),
                now_ms,
            ),
    }
}

fn record_destination_pending(
    store: &crate::services::onchain_replenishment_plan_store::OnchainReplenishmentPlanStore,
    run: &OnchainReplenishmentRun,
    confirmations: Option<u64>,
    source: &str,
    problem: String,
    now_ms: i64,
) -> Result<OnchainReplenishmentRun, String> {
    if wait_policy::expired(run, ONCHAIN_REPLENISHMENT_DESTINATION_WAIT_MS, now_ms) {
        let target = match active_leg(run)?.direction {
            OnchainTransferDirection::WithdrawToChain => "目标链",
            OnchainTransferDirection::DepositToCex => "交易所账户",
        };
        return store.pause_submission(
            &run.run_id,
            format!("{target} 超过本次 2 小时自动核验窗口：{problem}；保留原转账，未重新发送"),
            now_ms,
        );
    }
    store.record_destination_check_problem(
            &run.run_id,
            confirmations,
            source.to_owned(),
            problem,
            now_ms,
        )
}

fn withdrawal_status_request(
    run: &OnchainReplenishmentRun,
) -> Result<WithdrawalStatusRequest, String> {
    let transfer = active_transfer(run)?;
    let leg = active_leg(run)?;
    Ok(WithdrawalStatusRequest {
        venue: leg.venue.clone(),
        currency: leg.asset.clone(),
        network: leg
            .network_evidence
            .network
            .clone()
            .ok_or_else(|| "replenishment network is missing".to_owned())?,
        address: leg
            .destination
            .address
            .clone()
            .ok_or_else(|| "replenishment destination is missing".to_owned())?,
        client_withdrawal_id: transfer.client_transfer_id.clone(),
        tag: leg.destination.tag.clone(),
        provider_withdrawal_id: transfer.provider_transfer_id.clone(),
        submitted_at_ms: transfer.submission_attempted_at_ms,
    })
}

fn deposit_status_request(run: &OnchainReplenishmentRun) -> Result<DepositStatusRequest, String> {
    let transfer = active_transfer(run)?;
    let leg = active_leg(run)?;
    if leg.direction != OnchainTransferDirection::DepositToCex {
        return Err("deposit status request requires a deposit-to-CEX leg".to_owned());
    }
    Ok(DepositStatusRequest {
        venue: leg.venue.clone(),
        currency: leg.asset.clone(),
        network: leg
            .network_evidence
            .network
            .clone()
            .ok_or_else(|| "replenishment network is missing".to_owned())?,
        address: leg
            .destination
            .address
            .clone()
            .ok_or_else(|| "replenishment destination is missing".to_owned())?,
        tag: leg.destination.tag.clone(),
        transaction_id: transfer
            .transaction_id
            .clone()
            .ok_or_else(|| "chain deposit transaction id is missing".to_owned())?,
        amount: exact_decimal(leg.transfer_amount_exact.as_deref())
            .ok_or_else(|| "chain deposit exact amount is missing".to_owned())?,
        submitted_at_ms: transfer.submission_attempted_at_ms,
    })
}

fn active_transfer(
    run: &OnchainReplenishmentRun,
) -> Result<&shared_types::OnchainReplenishmentTransferProgress, String> {
    run.transfers
        .last()
        .ok_or_else(|| "replenishment transfer is missing".to_owned())
}

fn active_leg(run: &OnchainReplenishmentRun) -> Result<&OnchainReplenishmentLeg, String> {
    let transfer = active_transfer(run)?;
    run.plan
        .legs
        .get(transfer.leg_index as usize)
        .ok_or_else(|| "replenishment leg is missing".to_owned())
}

fn apply_source_evidence(
    state: &AppState,
    run: &OnchainReplenishmentRun,
    evidence: exchange::WithdrawalStatusEvidence,
    now_ms: i64,
) -> Result<OnchainReplenishmentRun, String> {
    state
        .onchain_replenishment_plans()
        .record_withdrawal_status(&run.run_id, evidence, now_ms)
}

async fn emit_replenishment_webhook(state: &AppState, run: &OnchainReplenishmentRun) {
    let kind = match run.status {
        OnchainReplenishmentRunStatus::Paused
        | OnchainReplenishmentRunStatus::Failed
        | OnchainReplenishmentRunStatus::AuthorizationExpired => WebhookEventKind::RiskAlert,
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit
        | OnchainReplenishmentRunStatus::ReadyForNextTransfer
        | OnchainReplenishmentRunStatus::Submitting
        | OnchainReplenishmentRunStatus::AwaitingSourceFinality
        | OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        | OnchainReplenishmentRunStatus::Completed => WebhookEventKind::ExecutionResult,
    };
    let leg = display_leg(run);
    let transfer = run.transfers.last();
    let mut event_id = format!(
        "{}:replenishment:{:?}:{}",
        run.run_id,
        run.status,
        run.transfers.len()
    )
    .to_ascii_lowercase();
    if run.read_only_recovery {
        event_id.push_str(&format!(":read-only:{}", run.recovery_checks));
    }
    if run.status == OnchainReplenishmentRunStatus::AwaitingDestinationCredit
        && transfer.and_then(|row| row.withdrawal_unlocked) == Some(false)
    {
        event_id.push_str(":credited-locked");
    }
    let payload = serde_json::json!({
        "title": "CROSSLINE 链上库存补充",
        "message": replenishment_message(run),
        "runId": run.run_id,
        "planId": run.plan.plan_id,
        "status": run.status,
        "venue": leg.map(|leg| &leg.venue),
        "asset": leg.map(|leg| &leg.asset),
        "network": leg.and_then(|leg| leg.network_evidence.network.as_deref()),
        "amount": leg.and_then(|leg| leg.transfer_amount_exact.as_deref()),
        "creditedAmountExact": transfer.and_then(|row| row.credited_amount_exact.as_deref()),
        "reportedDepositAmountExact": transfer.and_then(|row| row.reported_deposit_amount_exact.as_deref()),
        "depositFeeExact": transfer.and_then(|row| row.deposit_fee_exact.as_deref()),
        "withdrawalUnlocked": transfer.and_then(|row| row.withdrawal_unlocked),
        "withdrawalCost": transfer.and_then(|row| row.withdrawal_cost.as_ref()),
        "networkCost": transfer.and_then(|row| row.network_cost.as_ref()),
        "providerTransferId": transfer.and_then(|row| row.provider_transfer_id.as_deref()),
        "transactionId": transfer.and_then(|row| row.transaction_id.as_deref()),
        "confirmations": transfer.and_then(|row| row.confirmations),
        "nextAction": run.next_action,
        "problem": run.problem,
        "readOnlyRecovery": run.read_only_recovery,
        "recoveryChecks": run.recovery_checks,
        "automaticCheckDeadlineMs": run.automatic_wait_deadline_ms(),
    });
    if let Err(error) =
        crate::services::webhook::emit_idempotent(state, kind, event_id, payload).await
    {
        tracing::warn!(%error, run_id = %run.run_id, "on-chain replenishment webhook enqueue failed");
    }
}

fn replenishment_message(run: &OnchainReplenishmentRun) -> String {
    let leg = display_leg(run);
    let is_deposit = leg.is_some_and(|leg| leg.direction == OnchainTransferDirection::DepositToCex);
    let status = match run.status {
        OnchainReplenishmentRunStatus::AuthorizedAwaitingSubmit => "已授权，等待提交",
        OnchainReplenishmentRunStatus::AuthorizationExpired => "授权已过期",
        OnchainReplenishmentRunStatus::ReadyForNextTransfer => "上一条已到账，正在核验下一条",
        OnchainReplenishmentRunStatus::Submitting if is_deposit => "正在确认链上转账提交结果",
        OnchainReplenishmentRunStatus::Submitting => "正在确认交易所提交结果",
        OnchainReplenishmentRunStatus::AwaitingSourceFinality if is_deposit => {
            "链上转账已提交，等待区块确认"
        }
        OnchainReplenishmentRunStatus::AwaitingSourceFinality => "交易所已受理，等待提币终态",
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit
            if run.transfers.last().and_then(|row| row.withdrawal_unlocked) == Some(false) =>
        {
            "交易所已入账可交易，提币待解锁"
        }
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit if is_deposit => {
            "链上转账已确认，等待交易所入账"
        }
        OnchainReplenishmentRunStatus::AwaitingDestinationCredit => "交易所已完成，等待目标链到账",
        OnchainReplenishmentRunStatus::Completed => "目标资产已到账",
        OnchainReplenishmentRunStatus::Paused => "补仓已安全暂停",
        OnchainReplenishmentRunStatus::Failed => "补仓失败",
    };
    let scope = leg.map_or_else(
        || "资金腿待核验".to_owned(),
        |leg| {
            format!(
                "{} {} {}",
                leg.venue.to_uppercase(),
                leg.transfer_amount_exact.as_deref().unwrap_or("--"),
                leg.asset
            )
        },
    );
    let mut message = run.problem.as_ref().map_or_else(
        || format!("{status} · {scope}"),
        |problem| format!("{status} · {scope} · {problem}"),
    );
    if run.read_only_recovery {
        message.push_str(&format!(" · 只读核验 {}/{} 轮，不重发转账", run.recovery_checks, shared_types::ONCHAIN_REPLENISHMENT_RECOVERY_LIMIT));
    }
    if let Some(cost) = run
        .transfers
        .last()
        .and_then(|transfer| transfer.withdrawal_cost.as_ref())
    {
        let label = if cost.confirmed {
            "实扣提币费"
        } else {
            "提币费暂报"
        };
        message.push_str(&format!(" · {label} {} {}", cost.fee_exact, cost.asset));
        if cost.confirmed {
            if let Some(value) = &cost.usd_valuation {
                message.push_str(&format!(" · 提币费折算 ${}", value.usd_amount_exact));
            } else {
                message.push_str(" · 提币费美元折算待核验");
            }
        }
    }
    if run.status == OnchainReplenishmentRunStatus::ReadyForNextTransfer
        && super::replenishment_costs::waiting_for_valuation(run) {
        message.push_str(" · 等待实际费用的美元汇率，暂不执行下一步");
    }
    if let Some(cost) = run
        .transfers
        .last()
        .and_then(|transfer| transfer.network_cost.as_ref())
    {
        if let Some(total) = &cost.total_fee_exact {
            message.push_str(&format!(" · 实扣网络费 {total} {}", cost.asset));
            if let Some(value) = &cost.usd_valuation {
                message.push_str(&format!(" · 网络费折算 ${}", value.usd_amount_exact));
            } else {
                message.push_str(" · 网络费美元折算待核验");
            }
        } else {
            message.push_str(" · 网络总费待核验，不按零费用计算");
        }
    }
    message
}

fn display_leg(run: &OnchainReplenishmentRun) -> Option<&OnchainReplenishmentLeg> {
    let index = if run.status == OnchainReplenishmentRunStatus::ReadyForNextTransfer {
        run.transfers.len()
    } else {
        run.transfers
            .last()
            .map(|transfer| transfer.leg_index as usize)
            .unwrap_or(0)
    };
    run.plan.legs.get(index).or_else(|| run.plan.legs.first())
}

fn exact_decimal(value: Option<&str>) -> Option<Decimal> {
    value?
        .trim()
        .parse::<Decimal>()
        .ok()
        .filter(|value| *value > Decimal::ZERO)
}

fn map_claim_error(error: ReplenishmentSubmitClaimError) -> AppError {
    match error {
        ReplenishmentSubmitClaimError::Missing => {
            not_found("ONCHAIN_REPLENISHMENT_RUN_MISSING", "补仓运行记录不存在")
        }
        ReplenishmentSubmitClaimError::AuthorizationExpired => conflict(
            "ONCHAIN_REPLENISHMENT_AUTHORIZATION_EXPIRED",
            "实盘补仓授权已过期，请重新生成计划并明确授权",
        ),
        ReplenishmentSubmitClaimError::ActorMismatch => conflict(
            "ONCHAIN_REPLENISHMENT_ACTOR_MISMATCH",
            "提交者与本次实盘授权主体不一致",
        ),
        ReplenishmentSubmitClaimError::InvalidLeg => {
            conflict("ONCHAIN_REPLENISHMENT_LEG_INVALID", "补仓资金腿不存在")
        }
        ReplenishmentSubmitClaimError::PreviousLegIncomplete => conflict(
            "ONCHAIN_REPLENISHMENT_PREVIOUS_LEG_INCOMPLETE",
            "上一条补仓资金腿尚未取得目标到账终态",
        ),
        ReplenishmentSubmitClaimError::SourceBusy(problem) => conflict(
            "ONCHAIN_REPLENISHMENT_SOURCE_BUSY", problem,
        ),
        ReplenishmentSubmitClaimError::SourceChanged => conflict(
            "ONCHAIN_REPLENISHMENT_SOURCE_CHANGED",
            "校验期间同一资金来源发生了补库变动，本次未发送请求；请重新核验最新余额后提交",
        ),
        ReplenishmentSubmitClaimError::Persistence(problem) => durability_error(problem),
    }
}

fn durability_error(problem: String) -> AppError {
    conflict(
        "ONCHAIN_REPLENISHMENT_STATE_NOT_DURABLE",
        format!("补仓状态未能持久化，已阻止后续动作：{problem}"),
    )
}

fn bad_request(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::BAD_REQUEST, code, message.into())
}

fn not_found(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::NOT_FOUND, code, message.into())
}

fn conflict(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::CONFLICT, code, message.into())
}

fn upstream(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::BAD_GATEWAY, code, message.into())
}

#[cfg(test)]
mod deposit_credit_tests;

#[cfg(test)]
#[path = "replenishment_submit/continuation_tests.rs"]
mod continuation_tests;

#[cfg(test)]
#[path = "replenishment_submit/withdrawal_cost_tests.rs"]
mod withdrawal_cost_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replenishment_deposit_notices_distinguish_chain_finality_credit_and_unlock() {
        let mut run: OnchainReplenishmentRun = serde_json::from_str(include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../shared-types/fixtures/onchain_replenishment_locked.json"
        )))
        .unwrap();
        assert!(replenishment_message(&run).contains("交易所已入账可交易，提币待解锁"));
        run.transfers[0].withdrawal_unlocked = None;
        assert!(replenishment_message(&run).contains("链上转账已确认，等待交易所入账"));
        run.status = OnchainReplenishmentRunStatus::AwaitingSourceFinality;
        assert!(replenishment_message(&run).contains("链上转账已提交，等待区块确认"));
    }

    #[test]
    fn replenishment_actual_credit_uses_exact_units_without_float_rounding() {
        assert_eq!(
            credit_units(12_500_001, Some(6)).unwrap(),
            Decimal::new(12_500_001, 6)
        );
        assert!(credit_units(u128::MAX, Some(6)).is_err());
        assert!(credit_units(1, Some(29)).is_err());
        assert!(credit_units(1, None).is_err());
    }
}
