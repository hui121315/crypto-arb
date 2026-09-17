use common::AppError;
use shared_types::{
    LiveOrderState, OnchainCexOrderPlan, OnchainExecutionLegKind, OnchainExecutionLegStatus,
    OnchainExecutionRunStatus, OnchainExecutionSubmitResponse, OnchainQuoteConversionOrderPlan,
    OnchainQuoteConversionSequence, OrderRecord, OrderSide,
};

use crate::services::onchain_execution_build_store::ClaimedOnchainBuild;
use crate::services::onchain_execution_run_store::{
    OnchainExecutionStage, OnchainQuoteConversionAttempt, PendingOnchainExecution,
};
use crate::state::AppState;

use super::{providers, reconciliation, ResponseContext, RunOutcome};

mod funding;

#[derive(Clone, Copy)]
struct SubmitContext<'a> {
    state: &'a AppState,
    response: ResponseContext<'a>,
    claimed: &'a ClaimedOnchainBuild,
}

struct ChainExecution {
    primary: OnchainCexOrderPlan,
    primary_record: OrderRecord,
    conversion: OnchainQuoteConversionOrderPlan,
    conversion_run: ConversionRun,
}

struct ChainFailure {
    transaction_id: String,
    problem: String,
}

struct RecoveryAccumulator {
    compensation_records: Vec<super::ReversedCexOrder>,
    problems: Vec<String>,
    recovered: bool,
}

#[derive(Clone, Copy)]
struct StageRecord<'a> {
    stage: OnchainExecutionStage,
    primary_plan: &'a OnchainCexOrderPlan,
    primary_record: Option<&'a OrderRecord>,
    conversion: &'a OnchainQuoteConversionOrderPlan,
    conversion_run: &'a ConversionRun,
    transaction_id: Option<&'a str>,
}

struct ConversionAttempt {
    plan: OnchainCexOrderPlan,
    record: OrderRecord,
    settlement: Option<shared_types::OnchainCexSettlement>,
}

struct ConversionRun {
    attempts: Vec<ConversionAttempt>,
    filled_from: f64,
    filled_to: f64,
    accounted_from: rust_decimal::Decimal,
    accounted_to: rust_decimal::Decimal,
    complete: bool,
    receipt_unresolved: bool,
    problem: Option<String>,
}

pub(super) struct RecoveredThreeLeg<'a> {
    pub(super) state: &'a AppState,
    pub(super) response: ResponseContext<'a>,
    pub(super) claimed: &'a ClaimedOnchainBuild,
    pub(super) primary: &'a OnchainCexOrderPlan,
    pub(super) primary_record: &'a OrderRecord,
    pub(super) conversion: &'a OnchainQuoteConversionOrderPlan,
    pub(super) attempts: Vec<OnchainQuoteConversionAttempt>,
}

pub(super) struct RecoveredPrebroadcast<'a> {
    pub(super) state: &'a AppState,
    pub(super) response: ResponseContext<'a>,
    pub(super) claimed: &'a ClaimedOnchainBuild,
    pub(super) primary: Option<(&'a OnchainCexOrderPlan, &'a OrderRecord)>,
    pub(super) conversion: &'a OnchainQuoteConversionOrderPlan,
    pub(super) attempts: Vec<OnchainQuoteConversionAttempt>,
}

pub(super) async fn recover_prebroadcast(
    recovered: RecoveredPrebroadcast<'_>,
) -> OnchainExecutionSubmitResponse {
    let conversion_run = restored_conversion_run(recovered.conversion, recovered.attempts);
    recover_before_chain(
        SubmitContext {
            state: recovered.state,
            response: recovered.response,
            claimed: recovered.claimed,
        },
        recovered.primary,
        recovered.conversion,
        conversion_run,
        "后端在链上广播前重启，已按持久化成交记录回滚 CEX 腿",
    )
    .await
}

pub(super) async fn complete_recovered(
    recovered: RecoveredThreeLeg<'_>,
    transaction_id: String,
) -> OnchainExecutionSubmitResponse {
    let conversion_run = restored_conversion_run(recovered.conversion, recovered.attempts);
    complete_after_chain(
        SubmitContext {
            state: recovered.state,
            response: recovered.response,
            claimed: recovered.claimed,
        },
        recovered.primary_record,
        recovered.conversion,
        conversion_run,
        transaction_id,
    )
    .await
}

pub(super) async fn reject_recovered(
    recovered: RecoveredThreeLeg<'_>,
    transaction_id: String,
    problem: String,
) -> OnchainExecutionSubmitResponse {
    let conversion_run = restored_conversion_run(recovered.conversion, recovered.attempts);
    recover_chain_rejection(
        SubmitContext {
            state: recovered.state,
            response: recovered.response,
            claimed: recovered.claimed,
        },
        recovered.primary,
        recovered.primary_record,
        recovered.conversion,
        conversion_run,
        ChainFailure {
            transaction_id,
            problem,
        },
    )
    .await
}

pub(super) async fn submit(
    state: &AppState,
    response: ResponseContext<'_>,
    claimed: &ClaimedOnchainBuild,
    primary: OnchainCexOrderPlan,
    conversion: OnchainQuoteConversionOrderPlan,
) -> Result<OnchainExecutionSubmitResponse, AppError> {
    let program =
        super::super::three_leg_execution::program(response.build.direction, conversion.sequence)
            .map_err(|problem| super::conflict("ONCHAIN_THREE_LEG_SEQUENCE_INVALID", problem))?;
    debug_assert_eq!(program.steps.len(), 3);
    tracing::debug!(
        run_id = response.run_id,
        program = super::super::three_leg_execution::summary(program),
        "starting recoverable three-leg on-chain execution"
    );
    let prepared = providers::prepare(state, &claimed.config, &response.build.chain_transaction)
        .await
        .map_err(|problem| super::conflict("ONCHAIN_SIGNING_FAILED", problem))?;
    let context = SubmitContext {
        state,
        response,
        claimed,
    };
    super::record_prepared(state, response, claimed, &primary, Some(&conversion))?;
    match conversion.sequence {
        OnchainQuoteConversionSequence::BeforePrimaryCex => {
            submit_conversion_first(context, primary, conversion, prepared).await
        }
        OnchainQuoteConversionSequence::AfterPrimaryCex => {
            submit_primary_first(context, primary, conversion, prepared).await
        }
    }
}

async fn submit_conversion_first(
    context: SubmitContext<'_>,
    primary: OnchainCexOrderPlan,
    conversion: OnchainQuoteConversionOrderPlan,
    prepared: providers::PreparedChainSubmission,
) -> Result<OnchainExecutionSubmitResponse, AppError> {
    let limits = funding::before_primary(&context.claimed.config, &primary, &conversion)
        .map_err(|problem| super::conflict("ONCHAIN_QUOTE_FUNDING_INVALID", problem))?;
    let conversion_run =
        execute_conversion(context, &conversion, empty_conversion(None), limits).await;
    if !conversion_run.complete {
        return Ok(recover_before_chain(
            context,
            None,
            &conversion,
            conversion_run,
            "Quote 换汇未完成，CEX 主单和链上交易均未提交",
        )
        .await);
    }
    record_stage(
        context,
        StageRecord {
            stage: OnchainExecutionStage::QuoteConversionFilled,
            primary_plan: &primary,
            primary_record: None,
            conversion: &conversion,
            conversion_run: &conversion_run,
            transaction_id: None,
        },
    )?;
    let primary_record = match super::submit_cex_order(
        context.state,
        context.response.run_id,
        &context.claimed.config,
        &primary,
        super::OnchainCexActionKind::Primary,
    )
    .await
    {
        Ok(record) => record,
        Err(problem) => {
            let mut conversion_run = conversion_run;
            conversion_run.problem = Some(problem);
            return Ok(recover_before_chain(
                context,
                None,
                &conversion,
                conversion_run,
                "CEX 主单提交失败，开始回滚 Quote 换汇",
            )
            .await);
        }
    };
    if !super::fully_filled(&primary_record, primary.sizing_plan.rounded_contracts) {
        return Ok(recover_before_chain(
            context,
            Some((&primary, &primary_record)),
            &conversion,
            conversion_run,
            "CEX 主单未全额成交，链上交易未广播",
        )
        .await);
    }
    record_stage(
        context,
        StageRecord {
            stage: OnchainExecutionStage::PrimaryCexFilled,
            primary_plan: &primary,
            primary_record: Some(&primary_record),
            conversion: &conversion,
            conversion_run: &conversion_run,
            transaction_id: None,
        },
    )?;
    submit_chain(
        context,
        ChainExecution {
            primary,
            primary_record,
            conversion,
            conversion_run,
        },
        prepared,
    )
    .await
}

async fn submit_primary_first(
    context: SubmitContext<'_>,
    primary: OnchainCexOrderPlan,
    conversion: OnchainQuoteConversionOrderPlan,
    prepared: providers::PreparedChainSubmission,
) -> Result<OnchainExecutionSubmitResponse, AppError> {
    let primary_record = match super::submit_cex_order(
        context.state,
        context.response.run_id,
        &context.claimed.config,
        &primary,
        super::OnchainCexActionKind::Primary,
    )
    .await
    {
        Ok(record) => record,
        Err(problem) => {
            return Ok(three_leg_response(
                context.response,
                None,
                &conversion,
                &empty_conversion(Some(problem.clone())),
                RunOutcome {
                    status: OnchainExecutionRunStatus::Failed,
                    chain_transaction_id: None,
                    remaining_exposure_usd: 0.0,
                    message: "CEX 主单未提交，链上交易和 Quote 换汇均未执行".to_owned(),
                    problem: Some(problem),
                },
            ));
        }
    };
    if !super::fully_filled(&primary_record, primary.sizing_plan.rounded_contracts) {
        return Ok(super::compensate_or_expose(
            super::CompensationContext {
                state: context.state,
                response: context.response,
                claimed: context.claimed,
                plan: &primary,
                reason: "CEX 主单未全额成交，链上交易和 Quote 换汇均未执行",
            },
            primary_record,
        )
        .await);
    }
    let conversion_run = empty_conversion(None);
    record_stage(
        context,
        StageRecord {
            stage: OnchainExecutionStage::PrimaryCexFilled,
            primary_plan: &primary,
            primary_record: Some(&primary_record),
            conversion: &conversion,
            conversion_run: &conversion_run,
            transaction_id: None,
        },
    )?;
    submit_chain(
        context,
        ChainExecution {
            primary,
            primary_record,
            conversion,
            conversion_run,
        },
        prepared,
    )
    .await
}

async fn submit_chain(
    context: SubmitContext<'_>,
    execution: ChainExecution,
    prepared: providers::PreparedChainSubmission,
) -> Result<OnchainExecutionSubmitResponse, AppError> {
    let receipts = execution
        .conversion_run
        .attempts
        .iter()
        .filter_map(|attempt| attempt.settlement.clone())
        .collect::<Vec<_>>();
    let aligned = match super::primary_alignment::align(
        context.state,
        context.claimed,
        &execution.primary,
        &execution.primary_record,
        &receipts,
        prepared,
    )
    .await
    {
        Ok(aligned) => aligned,
        Err(problem) => {
            return Ok(recover_before_chain(
                context,
                Some((&execution.primary, &execution.primary_record)),
                &execution.conversion,
                execution.conversion_run,
                &problem,
            )
            .await)
        }
    };
    let prepared = aligned.prepared;
    let context = SubmitContext {
        claimed: &aligned.claimed,
        response: ResponseContext {
            build: &aligned.claimed.response,
            ..context.response
        },
        ..context
    };
    let transaction_id = prepared.transaction_id().to_owned();
    record_stage(
        context,
        StageRecord {
            stage: OnchainExecutionStage::ChainBroadcasting,
            primary_plan: &execution.primary,
            primary_record: Some(&execution.primary_record),
            conversion: &execution.conversion,
            conversion_run: &execution.conversion_run,
            transaction_id: Some(&transaction_id),
        },
    )?;
    let outcome = providers::broadcast(prepared).await;
    resolve_chain_outcome(context, execution, outcome).await
}

async fn resolve_chain_outcome(
    context: SubmitContext<'_>,
    execution: ChainExecution,
    outcome: providers::ChainSubmissionOutcome,
) -> Result<OnchainExecutionSubmitResponse, AppError> {
    let ChainExecution {
        primary,
        primary_record,
        conversion,
        conversion_run,
    } = execution;
    match outcome {
        providers::ChainSubmissionOutcome::Confirmed { transaction_id } => {
            Ok(complete_after_chain(
                context,
                &primary_record,
                &conversion,
                conversion_run,
                transaction_id,
            )
            .await)
        }
        providers::ChainSubmissionOutcome::Pending {
            transaction_id,
            problem,
        } => {
            let pending = three_leg_response(
                context.response,
                Some(&primary_record),
                &conversion,
                &conversion_run,
                RunOutcome {
                    status: OnchainExecutionRunStatus::AwaitingChainFinality,
                    chain_transaction_id: Some(transaction_id.clone()),
                    remaining_exposure_usd: super::order_exposure_usd(&primary_record, &primary),
                    message: "链上交易已广播，系统保留现有 CEX 腿并继续追踪终态".to_owned(),
                    problem: Some(problem),
                },
            );
            let context = reconciliation::PendingChainExecution {
                state: context.state.clone(),
                run_id: context.response.run_id.to_owned(),
                started_at_ms: context.response.started_at_ms,
                claimed: context.claimed.clone(),
                cex_plan: primary,
                cex_record: primary_record,
                quote_conversion_plan: Some(conversion),
                quote_conversion_attempts: attempts(&conversion_run),
                pending: pending.clone(),
                transaction_id,
            };
            if let Err(problem) = super::record_pending(&context.state, &context) {
                tracing::warn!(run_id = %context.run_id, %problem, "chain is already broadcast; retaining read-only finality tracking");
            }
            reconciliation::spawn(context);
            Ok(pending)
        }
        providers::ChainSubmissionOutcome::Rejected {
            transaction_id,
            problem,
        } => Ok(recover_chain_rejection(
            context,
            &primary,
            &primary_record,
            &conversion,
            conversion_run,
            ChainFailure {
                transaction_id,
                problem,
            },
        )
        .await),
    }
}

async fn complete_after_chain(
    context: SubmitContext<'_>,
    primary_record: &OrderRecord,
    conversion: &OnchainQuoteConversionOrderPlan,
    mut conversion_run: ConversionRun,
    transaction_id: String,
) -> OnchainExecutionSubmitResponse {
    match funding::after_chain(
        context.state,
        &context.claimed.config,
        primary_record,
        &context.response.build.cex_order.instrument_spec,
        conversion,
    )
    .await
    {
        Ok(limits) if conversion.sequence == OnchainQuoteConversionSequence::AfterPrimaryCex => {
            conversion_run = execute_conversion(context, conversion, conversion_run, limits).await;
        }
        Ok(limits) => {
            funding::refresh(context.state, conversion, &mut conversion_run, limits).await;
        }
        Err(problem) => {
            conversion_run.complete = false;
            conversion_run.receipt_unresolved = true;
            conversion_run.problem = Some(problem);
        }
    }
    let complete = conversion_run.complete;
    let problem = conversion_run.problem.clone();
    let mut result = three_leg_response(
        context.response,
        Some(primary_record),
        conversion,
        &conversion_run,
        RunOutcome {
            status: if complete {
                OnchainExecutionRunStatus::Completed
            } else if conversion_run.receipt_unresolved {
                OnchainExecutionRunStatus::FinalityUnresolved
            } else {
                OnchainExecutionRunStatus::Exposed
            },
            chain_transaction_id: Some(transaction_id),
            remaining_exposure_usd: if complete {
                0.0
            } else {
                (conversion.planned_from_amount - conversion_run.filled_from).max(0.0)
            },
            message: if complete {
                "三腿均已确认完成".to_owned()
            } else {
                "主交易与链上交易已完成，换汇净到账未达标或仍待核算".to_owned()
            },
            problem,
        },
    );
    // Conversion accounting can remain unresolved after the chain transaction succeeds.
    if let Some(leg) = result
        .legs
        .iter_mut()
        .find(|leg| leg.kind == OnchainExecutionLegKind::Chain)
    {
        leg.status = OnchainExecutionLegStatus::Confirmed;
    }
    result
}

async fn recover_chain_rejection(
    context: SubmitContext<'_>,
    primary: &OnchainCexOrderPlan,
    primary_record: &OrderRecord,
    conversion: &OnchainQuoteConversionOrderPlan,
    conversion_run: ConversionRun,
    failure: ChainFailure,
) -> OnchainExecutionSubmitResponse {
    let mut recovered = recover_before_chain(
        context,
        Some((primary, primary_record)),
        conversion,
        conversion_run,
        "链上交易明确失败，按已成交 CEX 腿逆序回滚",
    )
    .await;
    super::attach_chain_transaction(
        &mut recovered,
        context.response.build,
        failure.transaction_id,
        shared_types::OnchainExecutionLegStatus::Rejected,
    );
    recovered.problem = Some(match recovered.problem.take() {
        Some(recovery_problem) => format!("{}；{recovery_problem}", failure.problem),
        None => failure.problem,
    });
    recovered
}

async fn execute_conversion(
    context: SubmitContext<'_>,
    original: &OnchainQuoteConversionOrderPlan,
    mut run: ConversionRun,
    limits: funding::Limits,
) -> ConversionRun {
    if !refresh_conversion_progress(original, &mut run)
        || !funding::refresh(context.state, original, &mut run, limits).await
        || run.complete
    {
        return run;
    }
    let max_attempts = super::super::three_leg_execution::MAX_QUOTE_CONVERSION_ATTEMPTS;
    let used_attempts = run.attempts.len();
    if used_attempts >= usize::from(max_attempts) {
        run.problem = Some("Quote 换汇已达到 3 次尝试上限；重启不会重置次数".into());
        return run;
    }
    let initial_fits = match funding::initial_plan_fits(original, &run, limits) {
        Ok(fits) => fits,
        Err(problem) => {
            run.problem = Some(problem);
            return run;
        }
    };
    let mut plan = if used_attempts == 0 && initial_fits {
        original.clone()
    } else {
        match next_conversion_plan(context, original, &mut run, limits).await {
            Some(plan) => plan,
            None => return run,
        }
    };
    for attempt in (used_attempts + 1)..=usize::from(max_attempts) {
        if let Err(problem) = funding::validate_next(original, &plan, &run, limits) {
            run.problem = Some(problem);
            return run;
        }
        let record = match super::submit_cex_order(
            context.state,
            context.response.run_id,
            &context.claimed.config,
            &plan.order,
            super::OnchainCexActionKind::QuoteConversion,
        )
        .await
        {
            Ok(record) => record,
            Err(problem) => {
                run.problem = Some(format!("Quote 换汇第 {attempt} 次提交失败：{problem}"));
                return run;
            }
        };
        let Some((filled_from, filled_to)) =
            append_conversion_attempt(original, &mut run, plan.order.clone(), record)
        else {
            return run;
        };
        if !refresh_conversion_progress(original, &mut run)
            || !funding::refresh(context.state, original, &mut run, limits).await
            || run.complete
        {
            return run;
        }
        if filled_from <= 0.0 && filled_to <= 0.0 {
            run.problem = Some("Quote 换汇没有确认任何成交".to_owned());
            return run;
        }
        if attempt == usize::from(max_attempts) {
            run.problem = Some("Quote 换汇连续 3 次仍未补齐目标数量".to_owned());
            return run;
        }
        match next_conversion_plan(context, original, &mut run, limits).await {
            Some(next) => plan = next,
            None => return run,
        }
    }
    run
}

async fn next_conversion_plan(
    context: SubmitContext<'_>,
    original: &OnchainQuoteConversionOrderPlan,
    run: &mut ConversionRun,
    limits: funding::Limits,
) -> Option<OnchainQuoteConversionOrderPlan> {
    let (remaining_input, remaining_output) = match funding::remaining(original, run, limits) {
        Ok(amounts) => amounts,
        Err(problem) => {
            run.problem = Some(problem);
            return None;
        }
    };
    match super::super::execution_build::replan_quote_conversion_residual(
        context.state,
        &context.claimed.config,
        context.response.build,
        original,
        remaining_input,
        remaining_output,
    )
    .await
    {
        Ok(Some(next)) => Some(next),
        Ok(None) => {
            run.problem = Some("换汇下单数量为零，但实际净到账仍未达到目标".into());
            None
        }
        Err(problem) => {
            run.problem = Some(problem);
            None
        }
    }
}

fn conversion_fill_amounts(
    plan: &OnchainCexOrderPlan,
    record: &OrderRecord,
) -> Result<(f64, f64), String> {
    if record.intent.client_order_id != plan.client_order_id
        || !record.intent.exchange.eq_ignore_ascii_case(&plan.venue)
        || !record
            .intent
            .symbol
            .eq_ignore_ascii_case(&plan.native_symbol)
        || record.intent.side != plan.side
    {
        return Err("Quote 换汇回执与原订单身份不一致".into());
    }
    if !super::terminal(record.state) {
        return Err("Quote 换汇订单终态未确认；不能重试剩余数量或反向补偿".into());
    }
    let contracts = record
        .filled_quantity
        .filter(|value| value.is_finite() && *value >= 0.0)
        .ok_or_else(|| "Quote 换汇缺少有效的实际成交数量".to_owned())?;
    if !plan.sizing_plan.rounded_contracts.is_finite()
        || plan.sizing_plan.rounded_contracts <= 0.0
        || contracts > plan.sizing_plan.rounded_contracts * (1.0 + 1e-9)
        || !plan.sizing_plan.contract_size.is_finite()
        || plan.sizing_plan.contract_size <= 0.0
        || (record.state == LiveOrderState::Filled
            && contracts < plan.sizing_plan.rounded_contracts * (1.0 - 1e-9))
    {
        return Err("Quote 换汇成交数量与请求规格或终态矛盾".into());
    }
    if contracts == 0.0 {
        return Ok((0.0, 0.0));
    }
    let price = record
        .filled_price
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| "Quote 换汇缺少实际成交均价；不能用预估价代替".to_owned())?;
    let base = contracts * plan.sizing_plan.contract_size;
    let quote = base * price;
    if !base.is_finite() || base <= 0.0 || !quote.is_finite() || quote <= 0.0 {
        return Err("Quote 换汇实际成交金额非法".into());
    }
    Ok(match plan.side {
        OrderSide::Buy => (quote, base),
        OrderSide::Sell => (base, quote),
    })
}

fn refresh_conversion_progress(
    original: &OnchainQuoteConversionOrderPlan,
    run: &mut ConversionRun,
) -> bool {
    if run.receipt_unresolved {
        run.complete = false;
        return false;
    }
    match super::super::execution_build::quote_conversion_progress(
        original,
        run.filled_from,
        run.filled_to,
    ) {
        Ok(complete) => {
            run.complete = complete;
            true
        }
        Err(problem) => {
            run.complete = false;
            run.problem = Some(problem);
            false
        }
    }
}

fn append_conversion_attempt(
    original: &OnchainQuoteConversionOrderPlan,
    run: &mut ConversionRun,
    plan: OnchainCexOrderPlan,
    record: OrderRecord,
) -> Option<(f64, f64)> {
    let amounts = if !plan.venue.eq_ignore_ascii_case(&original.order.venue)
        || !plan
            .native_symbol
            .eq_ignore_ascii_case(&original.order.native_symbol)
        || plan.side != original.order.side
        || plan.sizing_plan.contract_size != original.order.sizing_plan.contract_size
    {
        Err("Quote 换汇回执属于其他交易对或方向；不能计入本次换汇".into())
    } else if run.attempts.iter().any(|attempt| {
        attempt.plan.client_order_id == plan.client_order_id
            || attempt.record.intent.id == record.intent.id
    }) {
        Err("Quote 换汇恢复记录包含重复订单；需核对回执，不能重复计入成交".into())
    } else {
        conversion_fill_amounts(&plan, &record)
    };
    run.attempts.push(ConversionAttempt {
        plan,
        record,
        settlement: None,
    });
    match amounts {
        Ok((from, to)) => {
            run.filled_from += from;
            run.filled_to += to;
            Some((from, to))
        }
        Err(problem) => {
            run.receipt_unresolved = true;
            run.complete = false;
            run.problem = Some(problem);
            None
        }
    }
}

async fn recover_before_chain(
    context: SubmitContext<'_>,
    primary: Option<(&OnchainCexOrderPlan, &OrderRecord)>,
    conversion: &OnchainQuoteConversionOrderPlan,
    conversion_run: ConversionRun,
    reason: &str,
) -> OnchainExecutionSubmitResponse {
    if conversion_run.receipt_unresolved
        || context
            .state
            .onchain_execution_run_store()
            .unresolved_cex_action(context.response.run_id)
        || primary.is_some_and(|(_, record)| !super::terminal(record.state))
        || conversion_run
            .attempts
            .iter()
            .any(|attempt| !super::terminal(attempt.record.state))
    {
        return three_leg_response(
            context.response,
            primary.map(|(_, record)| record),
            conversion,
            &conversion_run,
            RunOutcome {
                status: OnchainExecutionRunStatus::FinalityUnresolved,
                chain_transaction_id: None,
                remaining_exposure_usd: primary
                    .map(|(plan, record)| super::order_exposure_usd(record, plan))
                    .unwrap_or(0.0),
                message: "CEX 成交回执未核清；已停止后续腿和反向补偿，先核对原订单".into(),
                problem: conversion_run
                    .problem
                    .clone()
                    .or_else(|| Some(reason.into())),
            },
        );
    }
    let mut recovery = RecoveryAccumulator {
        compensation_records: Vec::new(),
        problems: conversion_run.problem.clone().into_iter().collect(),
        recovered: true,
    };
    if let Some((plan, record)) = primary.filter(|(_, record)| has_fill(record)) {
        reverse_record(context, plan, record, "primary", &mut recovery).await;
    }
    for (index, attempt) in conversion_run.attempts.iter().enumerate().rev() {
        if !has_fill(&attempt.record) {
            continue;
        }
        if !recovery.recovered {
            recovery
                .problems
                .push("上一笔回滚的净资产尚未核平，后续换汇回滚已停止".into());
            break;
        }
        reverse_record(
            context,
            &attempt.plan,
            &attempt.record,
            &format!("quote-{index}"),
            &mut recovery,
        )
        .await;
    }
    let has_exposure = primary.is_some_and(|(_, record)| has_fill(record))
        || conversion_run
            .attempts
            .iter()
            .any(|row| has_fill(&row.record));
    let status = if !has_exposure {
        OnchainExecutionRunStatus::Failed
    } else if recovery.recovered {
        OnchainExecutionRunStatus::Compensated
    } else {
        OnchainExecutionRunStatus::Exposed
    };
    let mut result = three_leg_response(
        context.response,
        primary.map(|(_, record)| record),
        conversion,
        &conversion_run,
        RunOutcome {
            status,
            chain_transaction_id: None,
            remaining_exposure_usd: if recovery.recovered {
                0.0
            } else {
                conversion.planned_from_amount
            },
            message: reason.to_owned(),
            problem: (!recovery.problems.is_empty()).then(|| recovery.problems.join("；")),
        },
    );
    for reversed in recovery.compensation_records {
        let record = &reversed.record;
        let position = u8::try_from(result.legs.len() + 1).unwrap_or(u8::MAX);
        let mut leg = super::order_leg_result(
            position,
            shared_types::OnchainExecutionLegKind::Compensation,
            record,
            &reversed.plan.instrument_spec,
        );
        leg.recovery_residual = reversed.residual_evidence();
        result.legs.push(leg);
        result.compensation_order_id = Some(record.intent.id.clone());
    }
    result
}

async fn reverse_record(
    context: SubmitContext<'_>,
    plan: &OnchainCexOrderPlan,
    record: &OrderRecord,
    leg_key: &str,
    recovery: &mut RecoveryAccumulator,
) {
    match super::reverse_filled_order(
        super::CompensationContext {
            state: context.state,
            response: context.response,
            claimed: context.claimed,
            plan,
            reason: "三腿执行补偿",
        },
        plan,
        record,
        leg_key,
    )
    .await
    {
        Ok(reversed) if reversed.recovered() => {
            recovery.compensation_records.push(reversed);
        }
        Ok(reversed) => {
            recovery.problems.push(
                reversed
                    .recovery_problem
                    .clone()
                    .unwrap_or_else(|| format!("{} 回滚净资产未归零", plan.native_symbol)),
            );
            recovery.compensation_records.push(reversed);
            recovery.recovered = false;
        }
        Err(problem) => {
            recovery.problems.push(problem);
            recovery.recovered = false;
        }
    }
}

fn record_stage(context: SubmitContext<'_>, stage_record: StageRecord<'_>) -> Result<(), AppError> {
    let StageRecord {
        stage,
        primary_plan,
        primary_record,
        conversion,
        conversion_run,
        transaction_id,
    } = stage_record;
    let status = if matches!(
        stage,
        OnchainExecutionStage::ChainBroadcasting | OnchainExecutionStage::AwaitingChainFinality
    ) {
        OnchainExecutionRunStatus::AwaitingChainFinality
    } else {
        OnchainExecutionRunStatus::Executing
    };
    let current = three_leg_response(
        context.response,
        primary_record,
        conversion,
        conversion_run,
        RunOutcome {
            status,
            chain_transaction_id: transaction_id.map(str::to_owned),
            remaining_exposure_usd: primary_record
                .map(|record| super::order_exposure_usd(record, primary_plan))
                .unwrap_or(conversion_run.filled_from),
            message: stage_message(stage).to_owned(),
            problem: None,
        },
    );
    super::record_checkpoint(
        context.state,
        &PendingOnchainExecution {
            response: current,
            config: context.claimed.config.clone(),
            build: context.claimed.response.clone(),
            stage,
            primary_plan: primary_plan.clone(),
            primary_record: primary_record.cloned(),
            quote_conversion_plan: Some(conversion.clone()),
            quote_conversion_records: records(conversion_run),
            quote_conversion_attempts: attempts(conversion_run),
            transaction_id: transaction_id.map(str::to_owned),
            active_cex_order: None,
            active_cex_kind: None,
            active_cex_record: None,
        },
    )
    .map_err(super::durability_error)
}

fn stage_message(stage: OnchainExecutionStage) -> &'static str {
    match stage {
        OnchainExecutionStage::Prepared => "执行计划已保存，尚未提交资金动作",
        OnchainExecutionStage::CexActionSubmitting => "CEX 资金动作提交中，结果未知时禁止重复下单",
        OnchainExecutionStage::QuoteConversionFilled => "Quote 换汇已完成，准备提交 CEX 主单",
        OnchainExecutionStage::PrimaryCexFilled => "CEX 腿已成交，准备广播链上交易",
        OnchainExecutionStage::ChainBroadcasting => "链上交易正在广播，已保存恢复检查点",
        OnchainExecutionStage::AwaitingChainFinality => "链上交易已广播，等待终态确认",
    }
}

fn three_leg_response(
    response: ResponseContext<'_>,
    primary: Option<&OrderRecord>,
    conversion: &OnchainQuoteConversionOrderPlan,
    conversion_run: &ConversionRun,
    outcome: RunOutcome,
) -> OnchainExecutionSubmitResponse {
    let transaction_id = outcome.chain_transaction_id.clone();
    let chain_status = super::chain_status(outcome.status);
    let mut result = super::response(response, primary, outcome);
    let mut legs = Vec::new();
    let conversion_first = conversion.sequence == OnchainQuoteConversionSequence::BeforePrimaryCex;
    let conversion_position = if conversion_first { 1 } else { 3 };
    for attempt in &conversion_run.attempts {
        let mut leg = super::order_leg_result(
            conversion_position,
            shared_types::OnchainExecutionLegKind::QuoteConversion,
            &attempt.record,
            &attempt.plan.instrument_spec,
        );
        if let Some(settlement) = &attempt.settlement {
            leg.settlement = Some(settlement.clone());
        }
        legs.push(leg);
    }
    if let Some(primary) = primary {
        legs.push(super::order_leg_result(
            if conversion_first { 2 } else { 1 },
            shared_types::OnchainExecutionLegKind::PrimaryCex,
            primary,
            &response.build.cex_order.instrument_spec,
        ));
    }
    if let Some(transaction_id) = transaction_id {
        legs.push(super::chain_leg_result(
            response.build,
            &transaction_id,
            chain_status,
            result.problem.as_deref().unwrap_or(&result.message),
        ));
        if let Some(chain) = legs.last_mut() {
            chain.position = if conversion_first { 3 } else { 2 };
        }
    }
    legs.sort_by_key(|leg| leg.position);
    result.legs = legs;
    result
}

fn records(run: &ConversionRun) -> Vec<OrderRecord> {
    run.attempts
        .iter()
        .map(|attempt| attempt.record.clone())
        .collect()
}

fn attempts(run: &ConversionRun) -> Vec<OnchainQuoteConversionAttempt> {
    run.attempts
        .iter()
        .map(|attempt| OnchainQuoteConversionAttempt {
            plan: attempt.plan.clone(),
            record: attempt.record.clone(),
        })
        .collect()
}

fn restored_conversion_run(
    conversion: &OnchainQuoteConversionOrderPlan,
    attempts: Vec<OnchainQuoteConversionAttempt>,
) -> ConversionRun {
    let mut run = empty_conversion(None);
    for attempt in attempts {
        append_conversion_attempt(conversion, &mut run, attempt.plan, attempt.record);
    }
    refresh_conversion_progress(conversion, &mut run);
    run
}

fn empty_conversion(problem: Option<String>) -> ConversionRun {
    ConversionRun {
        attempts: Vec::new(),
        filled_from: 0.0,
        filled_to: 0.0,
        accounted_from: rust_decimal::Decimal::ZERO,
        accounted_to: rust_decimal::Decimal::ZERO,
        complete: false,
        receipt_unresolved: false,
        problem,
    }
}

fn has_fill(record: &OrderRecord) -> bool {
    record
        .filled_quantity
        .is_some_and(|quantity| quantity.is_finite() && quantity > 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conversion_test_limits(plan: &OnchainQuoteConversionOrderPlan) -> funding::Limits {
        let mut primary = plan.order.clone();
        primary.side = OrderSide::Buy;
        let mut config = shared_types::OnchainComparisonConfig::default();
        config.cex_taker_fee_bps = 0.0;
        funding::before_primary(&config, &primary, plan).unwrap()
    }

    fn with_zero_fee_receipts(mut run: ConversionRun) -> ConversionRun {
        for attempt in &mut run.attempts {
            let (debit, credit) = conversion_fill_amounts(&attempt.plan, &attempt.record).unwrap();
            let mut receipt =
                super::super::settlement::seed(&attempt.record, &attempt.plan.instrument_spec)
                    .unwrap();
            receipt.status = shared_types::OnchainCexSettlementStatus::Complete;
            receipt.debit_amount = Some(debit.to_string());
            receipt.credit_amount = Some(credit.to_string());
            attempt.settlement = Some(receipt);
        }
        run
    }

    #[tokio::test]
    async fn cex_compensation_stops_dependent_conversion_after_unreconciled_primary() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = common::config::AppConfig::default();
        config.history.enabled = false;
        config.storage.data_dir = dir.path().to_string_lossy().into();
        let state = AppState::new(config).await.unwrap();
        let saved = crate::services::onchain_execution_run_store::test_checkpoint();
        let claimed = ClaimedOnchainBuild {
            response: saved.build,
            config: saved.config,
        };
        let context = SubmitContext {
            state: &state,
            claimed: &claimed,
            response: ResponseContext {
                run_id: "dependent-recovery",
                build: &claimed.response,
                started_at_ms: 1,
            },
        };
        let (plan, record) = fixture();
        let conversion = restored_conversion_run(
            &plan,
            vec![OnchainQuoteConversionAttempt {
                plan: plan.order.clone(),
                record: record.clone(),
            }],
        );
        let response = recover_before_chain(
            context,
            Some((&saved.primary_plan, saved.primary_record.as_ref().unwrap())),
            &plan,
            conversion,
            "local test",
        )
        .await;
        assert_eq!(response.status, OnchainExecutionRunStatus::Exposed);
        assert!(response.problem.unwrap().contains("后续换汇回滚已停止"));
        assert!(response.compensation_order_id.is_none());
        assert!(state.trading_service().list_orders().is_empty());
        state
            .trading_service()
            .drain_sql_ledger_and_shutdown()
            .await
            .unwrap();
    }

    fn fixture() -> (OnchainQuoteConversionOrderPlan, OrderRecord) {
        let saved = crate::services::onchain_execution_run_store::test_checkpoint();
        let mut order = saved.primary_plan;
        order.sizing_plan.rounded_contracts = 1.0;
        order.sizing_plan.contract_size = 1.0;
        let plan = OnchainQuoteConversionOrderPlan {
            sequence: OnchainQuoteConversionSequence::BeforePrimaryCex,
            from_asset: "SOL".into(),
            to_asset: "USD".into(),
            planned_from_amount: 1.0,
            planned_to_amount: 100.0,
            order,
        };
        (plan, saved.primary_record.unwrap())
    }

    #[test]
    fn actual_conversion_price_is_required_even_with_a_valid_reference_price() {
        let (plan, mut record) = fixture();
        for price in [
            None,
            Some(0.0),
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
        ] {
            record.filled_price = price;
            assert!(conversion_fill_amounts(&plan.order, &record).is_err());
            let restored = restored_conversion_run(
                &plan,
                vec![OnchainQuoteConversionAttempt {
                    plan: plan.order.clone(),
                    record: record.clone(),
                }],
            );
            assert!(!restored.complete);
            assert!(restored.receipt_unresolved);
            assert_eq!(restored.filled_to, 0.0);
        }
        record.filled_price = Some(101.0);
        assert_eq!(
            conversion_fill_amounts(&plan.order, &record).unwrap(),
            (1.0, 101.0)
        );
    }

    #[test]
    fn conversion_requires_consistent_identity_quantity_and_terminal_state() {
        let (plan, record) = fixture();
        for quantity in [
            None,
            Some(-1.0),
            Some(f64::NAN),
            Some(f64::INFINITY),
            Some(1.1),
            Some(0.0),
            Some(0.5),
        ] {
            let mut invalid = record.clone();
            invalid.filled_quantity = quantity;
            assert!(conversion_fill_amounts(&plan.order, &invalid).is_err());
        }
        for field in 0..5 {
            let mut invalid = record.clone();
            match field {
                0 => invalid.intent.client_order_id = "other".into(),
                1 => invalid.intent.exchange = "other".into(),
                2 => invalid.intent.symbol = "other".into(),
                3 => invalid.intent.side = OrderSide::Buy,
                _ => invalid.state = LiveOrderState::PartiallyFilled,
            }
            assert!(conversion_fill_amounts(&plan.order, &invalid).is_err());
        }
    }

    #[test]
    fn cancelled_partial_conversion_still_counts_as_real_funds() {
        let (plan, mut record) = fixture();
        record.state = LiveOrderState::Cancelled;
        record.filled_quantity = Some(0.25);
        assert!(has_fill(&record));
        assert_eq!(
            conversion_fill_amounts(&plan.order, &record).unwrap(),
            (0.25, 25.0)
        );
        record.filled_quantity = Some(0.0);
        record.filled_price = None;
        assert!(!has_fill(&record));
        assert_eq!(
            conversion_fill_amounts(&plan.order, &record).unwrap(),
            (0.0, 0.0)
        );
    }

    #[test]
    fn restored_conversion_does_not_double_count_duplicate_orders() {
        let (plan, record) = fixture();
        let attempt = OnchainQuoteConversionAttempt {
            plan: plan.order.clone(),
            record,
        };
        let restored = restored_conversion_run(&plan, vec![attempt.clone(), attempt]);
        assert!(restored.receipt_unresolved);
        assert!(!restored.complete);
        assert_eq!(restored.filled_to, 100.0);
    }

    #[test]
    fn restored_conversion_cannot_count_a_valid_receipt_from_another_market() {
        let (original, mut record) = fixture();
        let mut plan = original.order.clone();
        plan.native_symbol = "BTC/USD".into();
        record.intent.symbol = plan.native_symbol.clone();
        assert!(conversion_fill_amounts(&plan, &record).is_ok());
        let restored = restored_conversion_run(
            &original,
            vec![OnchainQuoteConversionAttempt { plan, record }],
        );
        assert!(restored.receipt_unresolved);
        assert!(!restored.complete);
        assert_eq!(restored.filled_from, 0.0);
        assert_eq!(restored.filled_to, 0.0);
    }

    #[tokio::test]
    async fn recovered_conversion_checks_completion_and_attempt_limit_before_any_order() {
        let state = AppState::new(common::config::AppConfig::default())
            .await
            .unwrap();
        let saved = crate::services::onchain_execution_run_store::test_checkpoint();
        let claimed = ClaimedOnchainBuild {
            response: saved.build,
            config: saved.config,
        };
        let context = SubmitContext {
            state: &state,
            claimed: &claimed,
            response: ResponseContext {
                run_id: "recovered-conversion",
                build: &claimed.response,
                started_at_ms: 1,
            },
        };
        let (plan, record) = fixture();
        let complete = restored_conversion_run(
            &plan,
            vec![OnchainQuoteConversionAttempt {
                plan: plan.order.clone(),
                record: record.clone(),
            }],
        );
        let complete = execute_conversion(
            context,
            &plan,
            with_zero_fee_receipts(complete),
            conversion_test_limits(&plan),
        )
        .await;
        assert!(complete.complete);
        assert_eq!(complete.attempts.len(), 1);

        let attempts = (0..3)
            .map(|index| {
                let mut plan = plan.order.clone();
                let mut record = record.clone();
                plan.client_order_id = format!("partial-{index}");
                record.intent.client_order_id = plan.client_order_id.clone();
                record.intent.id = format!("partial-order-{index}");
                record.state = LiveOrderState::Cancelled;
                record.filled_quantity = Some(0.1);
                OnchainQuoteConversionAttempt { plan, record }
            })
            .collect();
        let partial = restored_conversion_run(&plan, attempts);
        let partial = execute_conversion(
            context,
            &plan,
            with_zero_fee_receipts(partial),
            conversion_test_limits(&plan),
        )
        .await;
        assert!(!partial.complete);
        assert_eq!(partial.attempts.len(), 3);
        assert!(partial.problem.unwrap().contains("重启不会重置次数"));
        assert!(state
            .trading_service()
            .get_order_by_client_order_id(&plan.order.client_order_id)
            .is_none());
    }

    #[tokio::test]
    async fn missing_conversion_price_retains_unknown_status_and_never_reverses_blindly() {
        let state = AppState::new(common::config::AppConfig::default())
            .await
            .unwrap();
        let saved = crate::services::onchain_execution_run_store::test_checkpoint();
        let claimed = ClaimedOnchainBuild {
            response: saved.build,
            config: saved.config,
        };
        let context = SubmitContext {
            state: &state,
            claimed: &claimed,
            response: ResponseContext {
                run_id: "missing-price",
                build: &claimed.response,
                started_at_ms: 1,
            },
        };
        let (plan, mut record) = fixture();
        record.filled_price = None;
        let restored = restored_conversion_run(
            &plan,
            vec![OnchainQuoteConversionAttempt {
                plan: plan.order.clone(),
                record,
            }],
        );
        let restored =
            execute_conversion(context, &plan, restored, conversion_test_limits(&plan)).await;
        assert!(restored.receipt_unresolved);
        let response = recover_before_chain(context, None, &plan, restored, "test").await;
        assert_eq!(
            response.status,
            OnchainExecutionRunStatus::FinalityUnresolved
        );
        assert!(response.compensation_order_id.is_none());
        assert!(response.problem.unwrap().contains("实际成交均价"));
    }
}
