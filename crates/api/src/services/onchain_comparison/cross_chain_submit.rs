use crate::services::onchain_cross_chain_run_store::CrossChainLegClaimError;
use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use onchain_monitor::OnchainBridgeQuote;
use shared_types::{
    OnchainComparisonConfig, OnchainCrossChainBridgeExecution, OnchainCrossChainLegKind,
    OnchainCrossChainLegProgress, OnchainCrossChainLegRunStatus, OnchainCrossChainRun,
    OnchainCrossChainRunStatus, OnchainCrossChainSubmitRequest, OnchainCrossChainSwapExecution,
    OnchainUnsignedTransaction,
};

use super::execution_submit::{IndependentChainOutcome, IndependentChainSubmission};

enum PreparedExecution {
    Swap(OnchainCrossChainSwapExecution),
    Bridge {
        contract: OnchainCrossChainBridgeExecution,
        gas_usd: Option<f64>,
    },
}

impl PreparedExecution {
    fn position(&self) -> u8 {
        match self {
            Self::Swap(contract) => contract.position,
            Self::Bridge { contract, .. } => contract.position,
        }
    }

    fn kind(&self) -> OnchainCrossChainLegKind {
        match self {
            Self::Swap(contract) => contract.kind,
            Self::Bridge { contract, .. } => contract.kind,
        }
    }

    fn input_token(&self) -> &str {
        match self {
            Self::Swap(contract) => &contract.input_token,
            Self::Bridge { contract, .. } => &contract.from_token,
        }
    }

    fn input_amount_raw(&self) -> &str {
        match self {
            Self::Swap(contract) => &contract.input_amount_raw,
            Self::Bridge { contract, .. } => &contract.from_amount_raw,
        }
    }

    fn minimum_output_amount_raw(&self) -> &str {
        match self {
            Self::Swap(contract) => &contract.minimum_output_amount_raw,
            Self::Bridge { contract, .. } => &contract.to_amount_min_raw,
        }
    }

    fn provider_transaction_id(&self) -> &str {
        match self {
            Self::Swap(contract) => &contract.execution_id,
            Self::Bridge { contract, .. } => &contract.transaction_id,
        }
    }

    fn transaction(&self) -> &OnchainUnsignedTransaction {
        match self {
            Self::Swap(contract) => &contract.transaction,
            Self::Bridge { contract, .. } => &contract.transaction,
        }
    }

    fn observed_at_ms(&self) -> i64 {
        match self {
            Self::Swap(contract) => contract.quote_observed_at_ms,
            Self::Bridge { contract, .. } => contract.quote_observed_at_ms,
        }
    }

    fn valid_until_ms(&self) -> i64 {
        match self {
            Self::Swap(contract) => contract.valid_until_ms,
            Self::Bridge { contract, .. } => contract.valid_until_ms,
        }
    }

    fn official_docs_url(&self) -> &str {
        match self {
            Self::Swap(contract) => &contract.official_docs_url,
            Self::Bridge { contract, .. } => &contract.official_docs_url,
        }
    }

    fn claim_contracts(
        &self,
    ) -> (
        Option<OnchainCrossChainSwapExecution>,
        Option<OnchainCrossChainBridgeExecution>,
    ) {
        match self {
            Self::Swap(contract) => (Some(contract.clone()), None),
            Self::Bridge { contract, .. } => (None, Some(contract.clone())),
        }
    }
}

struct RemainingEconomics {
    final_quote_amount_raw: u128,
    required_final_amount_raw: u128,
    net_return_bps: f64,
    cost_raw: u128,
    valuation: super::cross_chain::economics::Valuation,
}

pub(crate) async fn submit(
    state: &AppState,
    request: &OnchainCrossChainSubmitRequest,
    actor: &str,
) -> Result<OnchainCrossChainRun, AppError> {
    state
        .onchain_cross_chain_runs()
        .readiness()
        .map_err(|problem| unavailable("ONCHAIN_CROSS_CHAIN_RECOVERY_UNAVAILABLE", problem))?;
    let run_id = request.run_id.trim();
    if run_id.is_empty() {
        return Err(bad_request(
            "ONCHAIN_CROSS_CHAIN_RUN_INVALID",
            "runId 不能为空",
        ));
    }
    let now_ms = common::time::now_ms();
    let run = state
        .onchain_cross_chain_runs()
        .run(run_id, now_ms)
        .ok_or_else(|| not_found("ONCHAIN_CROSS_CHAIN_RUN_MISSING", "跨链闭环运行记录不存在"))?;
    if run.authorization.actor != actor {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_ACTOR_MISMATCH",
            "当前操作者与跨链闭环授权人不一致",
        ));
    }
    if run.status == OnchainCrossChainRunStatus::Completed {
        return Ok(run);
    }
    if matches!(
        run.status,
        OnchainCrossChainRunStatus::Paused
            | OnchainCrossChainRunStatus::Compensating
            | OnchainCrossChainRunStatus::Failed
            | OnchainCrossChainRunStatus::AuthorizationExpired
    ) {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_RUN_NOT_SUBMITTABLE",
            run.problem
                .clone()
                .unwrap_or_else(|| "跨链闭环当前状态不允许继续提交".to_owned()),
        ));
    }
    if run.active_position.is_some() {
        return Ok(run);
    }
    let leg = next_leg(&run)?.clone();
    if request.expected_position != leg.position {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_POSITION_CHANGED",
            "当前步骤已变化，请刷新运行记录；旧请求不会执行下一步",
        ));
    }
    if leg.status != OnchainCrossChainLegRunStatus::RequoteRequired {
        return Ok(run);
    }
    let input_amount_raw = next_input_amount(&run, &leg)?;
    let (source, peer) = execution_configs(state, &run)?;
    let config = if leg.position <= 2 { &source } else { &peer };
    let execution = prepare_current_execution(
        state,
        &source,
        &peer,
        leg.position,
        leg.kind,
        &input_amount_raw,
    )
    .await?;
    let economics = validate_remaining_economics(state, &run, &source, &peer, &execution).await?;
    super::cross_chain::validate_current_execution_contract(
        state,
        config,
        format!("第 {} 腿", leg.position),
        execution.input_token(),
        execution.input_amount_raw(),
        execution.transaction(),
    )
    .await
    .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_PRE_TRADE_REJECTED", problem))?;
    let prepared_submission = prepare_submission(state, config, &execution).await?;
    super::cross_chain_costs::ensure_claimed(state, &run)
        .map_err(|p| conflict("ONCHAIN_CROSS_CHAIN_COST_CLAIM_FAILED", p))?;
    let source_transaction_id = prepared_submission.transaction_id().to_owned();
    let (swap_execution, bridge_execution) = execution.claim_contracts();
    let claim = state
        .onchain_cross_chain_runs()
        .claim_leg(
            &run.run_id,
            actor,
            execution.position(),
            execution.input_amount_raw().to_owned(),
            execution.minimum_output_amount_raw().to_owned(),
            execution.provider_transaction_id().to_owned(),
            swap_execution,
            bridge_execution,
            execution.observed_at_ms(),
            execution.valid_until_ms().min(
                economics
                    .valuation
                    .evidence
                    .observed_at_ms
                    .saturating_add(source.max_age_ms.min(peer.max_age_ms)),
            ),
            economics.final_quote_amount_raw.to_string(),
            economics.required_final_amount_raw.to_string(),
            format!("{:.8}", economics.net_return_bps),
            common::time::now_ms(),
        )
        .map_err(map_claim_error)?;
    if claim.replayed {
        return Ok(claim.run);
    }
    state
        .onchain_cross_chain_runs()
        .record_submission_intent(
            &run.run_id,
            source_transaction_id,
            format!(
                "{} · remaining_final={} · required_final={} · net_bps={:.4} · gas_and_independent_cost_quote_raw={} · usd_asset={} · usd_venue={} · usd_pair={} · usd_bid={} · usd_ask={} · usd_source={} · usd_observed_at_ms={} · currency_risk_bps={}",
                execution.official_docs_url(),
                economics.final_quote_amount_raw,
                economics.required_final_amount_raw,
                economics.net_return_bps,
                economics.cost_raw,
                economics.valuation.evidence.asset,
                economics.valuation.evidence.venue,
                economics.valuation.evidence.symbol,
                economics.valuation.evidence.usd_bid,
                economics.valuation.evidence.usd_ask,
                economics.valuation.evidence.source,
                economics.valuation.evidence.observed_at_ms,
                economics.valuation.risk_bps
            ),
            common::time::now_ms(),
        )
        .map_err(|problem| unavailable("ONCHAIN_CROSS_CHAIN_INTENT_NOT_DURABLE", problem))?;
    let outcome =
        super::execution_submit::broadcast_independent_chain_transaction(prepared_submission).await;
    apply_submission_outcome(state, &run.run_id, execution.official_docs_url(), outcome).await
}

fn next_leg(run: &OnchainCrossChainRun) -> Result<&OnchainCrossChainLegProgress, AppError> {
    run.legs
        .iter()
        .find(|leg| leg.status != OnchainCrossChainLegRunStatus::Completed)
        .ok_or_else(|| conflict("ONCHAIN_CROSS_CHAIN_LEG_MISSING", "跨链闭环没有待执行腿"))
}

fn next_input_amount(
    run: &OnchainCrossChainRun,
    leg: &OnchainCrossChainLegProgress,
) -> Result<String, AppError> {
    if leg.position == 1 {
        return positive_raw(&run.build.initial_quote_amount_raw)
            .map(|_| run.build.initial_quote_amount_raw.clone())
            .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_INPUT_INVALID", problem));
    }
    let previous = run
        .legs
        .iter()
        .find(|row| row.position == leg.position.saturating_sub(1))
        .filter(|row| row.status == OnchainCrossChainLegRunStatus::Completed)
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CROSS_CHAIN_PREVIOUS_LEG_INCOMPLETE",
                "上一腿尚未形成真实到账证据",
            )
        })?;
    let amount = previous
        .actual_output_amount_raw
        .as_deref()
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CROSS_CHAIN_ACTUAL_OUTPUT_MISSING",
                "上一腿缺少真实到账原始数量",
            )
        })?;
    positive_raw(amount)
        .map(|_| amount.to_owned())
        .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_INPUT_INVALID", problem))
}

fn execution_configs(
    state: &AppState,
    run: &OnchainCrossChainRun,
) -> Result<(OnchainComparisonConfig, OnchainComparisonConfig), AppError> {
    let source_anchor = run
        .build
        .swap_executions
        .iter()
        .find(|execution| execution.position == 1)
        .ok_or_else(|| conflict("ONCHAIN_CROSS_CHAIN_CONFIG_MISSING", "缺少源链配置锚点"))?;
    let peer_anchor = run
        .build
        .swap_executions
        .iter()
        .find(|execution| execution.position == 3)
        .ok_or_else(|| conflict("ONCHAIN_CROSS_CHAIN_CONFIG_MISSING", "缺少目标链配置锚点"))?;
    let active = state.onchain_monitor().snapshot().config.clone();
    let mut configs = vec![active];
    configs.extend(
        state
            .onchain_monitor()
            .batch()
            .configs()
            .into_iter()
            .map(|(_, config)| config),
    );
    let source = configs
        .iter()
        .find(|config| config_matches_anchor(config, source_anchor))
        .cloned()
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CROSS_CHAIN_SOURCE_CONFIG_CHANGED",
                "源链配置已不存在或身份已变化，已拒绝继续使用旧授权",
            )
        })?;
    let peer = configs
        .iter()
        .find(|config| config_matches_anchor(config, peer_anchor))
        .cloned()
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CROSS_CHAIN_PEER_CONFIG_CHANGED",
                "目标链配置已不存在或身份已变化，已拒绝继续使用旧授权",
            )
        })?;
    if !source.cross_chain.enabled
        || source.chain.eq_ignore_ascii_case(&peer.chain)
        || !run.build.source_chain.eq_ignore_ascii_case(&source.chain)
        || !run.build.peer_chain.eq_ignore_ascii_case(&peer.chain)
    {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_CONFIG_CHANGED",
            "跨链闭环链路配置已经变化，请重新生成并授权",
        ));
    }
    Ok((source, peer))
}

fn config_matches_anchor(
    config: &OnchainComparisonConfig,
    anchor: &OnchainCrossChainSwapExecution,
) -> bool {
    let token_matches = if config.chain.eq_ignore_ascii_case("solana") {
        match anchor.kind {
            OnchainCrossChainLegKind::SourceSwap => {
                config.quote_mint == anchor.input_token && config.base_mint == anchor.output_token
            }
            OnchainCrossChainLegKind::TargetSwap => {
                config.base_mint == anchor.input_token && config.quote_mint == anchor.output_token
            }
            _ => false,
        }
    } else {
        match anchor.kind {
            OnchainCrossChainLegKind::SourceSwap => {
                config.quote_mint.eq_ignore_ascii_case(&anchor.input_token)
                    && config.base_mint.eq_ignore_ascii_case(&anchor.output_token)
            }
            OnchainCrossChainLegKind::TargetSwap => {
                config.base_mint.eq_ignore_ascii_case(&anchor.input_token)
                    && config.quote_mint.eq_ignore_ascii_case(&anchor.output_token)
            }
            _ => false,
        }
    };
    let wallet_matches = if config.chain.eq_ignore_ascii_case("solana") {
        config.wallet_address == anchor.wallet_address
    } else {
        config
            .wallet_address
            .eq_ignore_ascii_case(&anchor.wallet_address)
    };
    config.chain.eq_ignore_ascii_case(&anchor.chain)
        && config.provider == anchor.provider
        && wallet_matches
        && token_matches
}

async fn prepare_current_execution(
    state: &AppState,
    source: &OnchainComparisonConfig,
    peer: &OnchainComparisonConfig,
    position: u8,
    kind: OnchainCrossChainLegKind,
    input_amount_raw: &str,
) -> Result<PreparedExecution, AppError> {
    match kind {
        OnchainCrossChainLegKind::SourceSwap => super::cross_chain::build_swap_execution(
            state,
            source,
            position,
            kind,
            input_amount_raw,
            position.saturating_sub(1),
        )
        .await
        .map(PreparedExecution::Swap),
        OnchainCrossChainLegKind::TargetSwap => super::cross_chain::build_swap_execution(
            state,
            peer,
            position,
            kind,
            input_amount_raw,
            position.saturating_sub(1),
        )
        .await
        .map(PreparedExecution::Swap),
        OnchainCrossChainLegKind::OutboundBridge => {
            let quote = fetch_bridge(source, peer, true, input_amount_raw).await?;
            Ok(prepared_bridge(position, kind, &quote))
        }
        OnchainCrossChainLegKind::ReturnBridge => {
            let quote = fetch_bridge(peer, source, false, input_amount_raw).await?;
            Ok(prepared_bridge(position, kind, &quote))
        }
    }
}

fn prepared_bridge(
    position: u8,
    kind: OnchainCrossChainLegKind,
    quote: &OnchainBridgeQuote,
) -> PreparedExecution {
    PreparedExecution::Bridge {
        contract: super::cross_chain::bridge_execution_contract(
            position,
            kind,
            position.saturating_sub(1),
            quote,
        ),
        gas_usd: quote.gas_usd,
    }
}

async fn fetch_bridge(
    from: &OnchainComparisonConfig,
    to: &OnchainComparisonConfig,
    base_asset: bool,
    input_amount_raw: &str,
) -> Result<OnchainBridgeQuote, AppError> {
    let (from_token, to_token) = if base_asset {
        (&from.base_mint, &to.base_mint)
    } else {
        (&from.quote_mint, &to.quote_mint)
    };
    super::lifi::fetch_bridge(
        &from.chain,
        &to.chain,
        from_token,
        to_token,
        input_amount_raw,
        &from.wallet_address,
        &to.wallet_address,
        from.slippage_bps,
    )
    .await
    .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_BRIDGE_REQUOTE_FAILED", problem))
}

async fn validate_remaining_economics(
    state: &AppState,
    run: &OnchainCrossChainRun,
    source: &OnchainComparisonConfig,
    peer: &OnchainComparisonConfig,
    current: &PreparedExecution,
) -> Result<RemainingEconomics, AppError> {
    let initial = positive_raw(&run.build.initial_quote_amount_raw)
        .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_INITIAL_AMOUNT_INVALID", problem))?;
    let mut amount = positive_raw(current.minimum_output_amount_raw())
        .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_MINIMUM_OUTPUT_INVALID", problem))?;
    let mut fresh_bridge_gas = match current {
        PreparedExecution::Swap(_) => Vec::new(),
        PreparedExecution::Bridge { gas_usd, .. } => vec![required_gas(*gas_usd)?],
    };
    if current.position() <= 1 {
        let outbound = fetch_bridge(source, peer, true, &amount.to_string()).await?;
        amount = positive_raw(&outbound.to_amount_min_raw)
            .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_MINIMUM_OUTPUT_INVALID", problem))?;
        fresh_bridge_gas.push(required_gas(outbound.gas_usd)?);
    }
    if current.position() <= 2 {
        let target = super::quote::fetch_exact_in(
            peer,
            &peer.provider,
            &peer.base_mint,
            &peer.quote_mint,
            &amount.to_string(),
        )
        .await
        .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_TARGET_REQUOTE_FAILED", problem))?;
        amount = super::execution_build::conservative_minimum_output(
            &target.output_amount_raw,
            peer.slippage_bps,
        )
        .and_then(|value| positive_raw(&value))
        .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_MINIMUM_OUTPUT_INVALID", problem))?;
    }
    if current.position() <= 3 {
        let returned = fetch_bridge(peer, source, false, &amount.to_string()).await?;
        amount = positive_raw(&returned.to_amount_min_raw)
            .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_MINIMUM_OUTPUT_INVALID", problem))?;
        fresh_bridge_gas.push(required_gas(returned.gas_usd)?);
    }
    let build_gas_usd = required_gas(run.build.gas_usd)?;
    let fresh_known_gas = required_gas(Some(source.gas_usd))?
        + required_gas(Some(peer.gas_usd))?
        + fresh_bridge_gas.into_iter().sum::<f64>();
    let gas_usd = build_gas_usd.max(required_gas(Some(fresh_known_gas))?);
    super::usd_valuation::refresh(state, source).await;
    let now_ms = common::time::now_ms();
    let evidence = super::usd_valuation::quote_evidence(state, source, now_ms)
        .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_USD_VALUATION_MISSING", problem))?;
    let valuation = super::cross_chain::economics::Valuation::read(
        source,
        Some(&evidence),
        source.max_age_ms.min(peer.max_age_ms),
        now_ms,
    )
    .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_USD_VALUATION_MISSING", problem))?;
    let exchange_rate_risk_bps = u128::from(valuation.risk_bps);
    let cost_raw = valuation
        .gas_raw(gas_usd + super::cross_chain_costs::current_total_usd(state, source, &run.build, now_ms)
            .map_err(|p| conflict("ONCHAIN_CROSS_CHAIN_COST_UNPROVEN", p))?, source.quote_decimals)
        .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_GAS_CONVERSION_INVALID", problem))?;
    let configured_minimum_bps = source.spread_alert.min_net_spread_bps;
    if !configured_minimum_bps.is_finite() || !(0.0..=10_000.0).contains(&configured_minimum_bps) {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_PROFIT_THRESHOLD_INVALID",
            "最低费后收益阈值必须位于 0% 到 100% 之间",
        ));
    }
    let minimum_net_bps = configured_minimum_bps.ceil() as u128;
    let required_return_bps = minimum_net_bps
        .checked_add(exchange_rate_risk_bps)
        .ok_or_else(|| conflict("ONCHAIN_CROSS_CHAIN_ECONOMICS_OVERFLOW", "汇率风险阈值溢出"))?;
    let required_before_gas = ceil_ratio(
        initial,
        10_000_u128
            .checked_add(required_return_bps)
            .ok_or_else(|| conflict("ONCHAIN_CROSS_CHAIN_ECONOMICS_OVERFLOW", "收益阈值溢出"))?,
        10_000,
    )?;
    let required_final_amount_raw = required_before_gas.checked_add(cost_raw).ok_or_else(|| {
        conflict(
            "ONCHAIN_CROSS_CHAIN_ECONOMICS_OVERFLOW",
            "Gas 成本与最低收益相加溢出",
        )
    })?;
    let net_return_bps = (amount as f64 / initial as f64 - 1.0) * 10_000.0
        - cost_raw as f64 / initial as f64 * 10_000.0
        - exchange_rate_risk_bps as f64;
    if amount < required_final_amount_raw {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_REMAINING_PROFIT_REJECTED",
            format!(
                "第 {} 腿重报价后，最坏到账 {} 低于含 Gas、已归集费用与最低收益要求的 {}",
                current.position(),
                amount,
                required_final_amount_raw
            ),
        ));
    }
    Ok(RemainingEconomics {
        final_quote_amount_raw: amount,
        required_final_amount_raw,
        net_return_bps,
        cost_raw,
        valuation,
    })
}

fn required_gas(usd: Option<f64>) -> Result<f64, AppError> {
    usd.filter(|usd| usd.is_finite() && *usd >= 0.0)
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CROSS_CHAIN_GAS_EVIDENCE_MISSING",
                "跨链 Gas 美元证据缺失或无效；不能使用旧报价掩盖本次缺失成本",
            )
        })
}

fn ceil_ratio(value: u128, numerator: u128, denominator: u128) -> Result<u128, AppError> {
    let product = value.checked_mul(numerator).ok_or_else(|| {
        conflict(
            "ONCHAIN_CROSS_CHAIN_ECONOMICS_OVERFLOW",
            "最低收益原始数量计算溢出",
        )
    })?;
    product
        .checked_add(denominator.saturating_sub(1))
        .map(|rounded| rounded / denominator)
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CROSS_CHAIN_ECONOMICS_OVERFLOW",
                "最低收益向上取整溢出",
            )
        })
}

async fn prepare_submission(
    state: &AppState,
    config: &OnchainComparisonConfig,
    execution: &PreparedExecution,
) -> Result<IndependentChainSubmission, AppError> {
    let result = match execution.kind() {
        OnchainCrossChainLegKind::SourceSwap | OnchainCrossChainLegKind::TargetSwap => {
            super::execution_submit::prepare_independent_chain_transaction(
                state,
                config,
                execution.transaction(),
            )
            .await
        }
        OnchainCrossChainLegKind::OutboundBridge | OnchainCrossChainLegKind::ReturnBridge => {
            super::execution_submit::prepare_independent_rpc_transaction(
                state,
                config,
                execution.transaction(),
            )
            .await
        }
    };
    result.map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_SIGNING_NOT_READY", problem))
}

async fn apply_submission_outcome(
    state: &AppState,
    run_id: &str,
    evidence_source: &str,
    outcome: IndependentChainOutcome,
) -> Result<OnchainCrossChainRun, AppError> {
    let now_ms = common::time::now_ms();
    let notify = !matches!(outcome, IndependentChainOutcome::Pending { .. });
    let run = match outcome {
        IndependentChainOutcome::Confirmed { .. } => state
            .onchain_cross_chain_runs()
            .record_source_confirmed(run_id, evidence_source.to_owned(), now_ms),
        IndependentChainOutcome::Pending { problem, .. } => state
            .onchain_cross_chain_runs()
            .record_check_problem(run_id, problem, evidence_source.to_owned(), now_ms),
        IndependentChainOutcome::Rejected { problem, .. } => state
            .onchain_cross_chain_runs()
            .pause(run_id, problem, now_ms),
    }
    .map_err(|problem| unavailable("ONCHAIN_CROSS_CHAIN_STATE_NOT_DURABLE", problem))?;
    if notify {
        super::cross_chain_reconcile::emit_run_webhook(state, &run).await;
    }
    Ok(run)
}

fn map_claim_error(error: CrossChainLegClaimError) -> AppError {
    match error {
        CrossChainLegClaimError::Missing => {
            not_found("ONCHAIN_CROSS_CHAIN_RUN_MISSING", "跨链闭环运行记录不存在")
        }
        CrossChainLegClaimError::AuthorizationExpired => conflict(
            "ONCHAIN_CROSS_CHAIN_AUTHORIZATION_EXPIRED",
            "首腿提交授权已经过期，请重新生成并授权",
        ),
        CrossChainLegClaimError::ActorMismatch => conflict(
            "ONCHAIN_CROSS_CHAIN_ACTOR_MISMATCH",
            "当前操作者与跨链闭环授权人不一致",
        ),
        CrossChainLegClaimError::InvalidPosition
        | CrossChainLegClaimError::PreviousLegIncomplete => conflict(
            "ONCHAIN_CROSS_CHAIN_SEQUENCE_REJECTED",
            "跨链闭环必须按顺序逐腿完成",
        ),
        CrossChainLegClaimError::InvalidQuote(problem) => {
            conflict("ONCHAIN_CROSS_CHAIN_REQUOTE_REJECTED", problem)
        }
        CrossChainLegClaimError::Persistence(problem) => {
            unavailable("ONCHAIN_CROSS_CHAIN_CLAIM_NOT_DURABLE", problem)
        }
    }
}

fn positive_raw(value: &str) -> Result<u128, String> {
    value
        .trim()
        .parse::<u128>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| "原始数量必须是正整数".to_owned())
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

fn unavailable(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::SERVICE_UNAVAILABLE, code, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profit_threshold_rounds_up_in_raw_units() {
        assert_eq!(
            ceil_ratio(100_000_001, 10_010, 10_000).ok(),
            Some(100_100_002)
        );
    }

    #[test]
    fn missing_gas_is_not_replaced_by_zero_or_previous_quotes() {
        assert!(required_gas(None).is_err());
        assert!(required_gas(Some(f64::NAN)).is_err());
        assert!(required_gas(Some(f64::INFINITY)).is_err());
        assert!(required_gas(Some(-1.0)).is_err());
        assert_eq!(required_gas(Some(0.0)).ok(), Some(0.0));
    }
}
