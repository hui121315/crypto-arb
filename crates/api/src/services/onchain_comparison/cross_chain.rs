use crate::services::onchain_cross_chain_run_store::CrossChainAuthorizeError;
use crate::state::AppState;
use axum::http::StatusCode;
use base64::{engine::general_purpose::STANDARD as B64_STANDARD, Engine as _};
use common::AppError;
use onchain_monitor::{
    OnchainBridgeQuote, OnchainCrossChainQuoteSet, OnchainQuotePair, OnchainWalletInventory,
    ProviderQuote,
};
use shared_types::{
    onchain_quote_provider, OnchainComparisonConfig, OnchainComparisonSnapshot,
    OnchainCrossChainAuthorizeRequest, OnchainCrossChainBridgeExecution,
    OnchainCrossChainBuildRequest, OnchainCrossChainBuildResponse,
    OnchainCrossChainInventoryRequirement, OnchainCrossChainInventoryStatus, OnchainCrossChainLeg,
    OnchainCrossChainLegKind, OnchainCrossChainQuality, OnchainCrossChainRun,
    OnchainCrossChainRunsResponse, OnchainCrossChainSnapshot, OnchainCrossChainSwapExecution,
    ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE,
};

use super::quote::fetch_exact_in;

pub(super) mod economics;

const SOLANA_FEE_DOCS: &str = "https://solana.com/docs/rpc/http/getfeeformessage";
const ETHEREUM_RPC_DOCS: &str = "https://ethereum.org/developers/docs/apis/json-rpc/";

pub(super) const REFRESH_INTERVAL_MS: i64 = 30_000;
const BUILD_ID_KEY: &[u8] = b"crossline-onchain-cross-chain-preview-v1";
const SWAP_EXECUTION_ID_KEY: &[u8] = b"crossline-onchain-cross-chain-swap-v1";

pub(crate) async fn build_preview(
    state: &AppState,
    request: &OnchainCrossChainBuildRequest,
) -> Result<OnchainCrossChainBuildResponse, AppError> {
    let snapshot = state.onchain_monitor().snapshot();
    let (peer_item_id, peer) = peer_config(state, &snapshot.config).ok_or_else(|| {
        conflict(
            "ONCHAIN_CROSS_CHAIN_PEER_MISSING",
            "跨链目标市场已不存在，请重新选择目标链市场",
        )
    })?;
    let quotes = state
        .onchain_monitor()
        .cross_chain_quotes()
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CROSS_CHAIN_QUOTE_PENDING",
                "跨链闭环交易合同尚未进入缓存，请等待下一轮按需刷新",
            )
        })?;
    if !quotes.matches_configs(&snapshot.config, &peer_item_id, &peer) {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_SNAPSHOT_CHANGED",
            "跨链交易合同不属于当前源链与目标链配置，请重新生成预览",
        ));
    }
    let max_age_ms = snapshot.config.max_age_ms.min(peer.max_age_ms).max(1);
    let now_ms = common::time::now_ms();
    let mut response = build_preview_from_snapshot(
        &snapshot,
        request,
        now_ms,
        max_age_ms,
        Some(quotes.as_ref()),
    )?;
    let swap_executions =
        build_swap_execution_contracts(state, &snapshot.config, &peer, &response.legs).await?;
    response.valid_until_ms = response.valid_until_ms.min(
        swap_executions
            .iter()
            .map(|execution| execution.valid_until_ms)
            .min()
            .unwrap_or(response.valid_until_ms),
    );
    if response.valid_until_ms <= common::time::now_ms() {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_QUOTE_STALE",
            "四腿交易合同在构建完成前已经过期，请重新生成",
        ));
    }
    response.swap_executions = swap_executions;
    super::cross_chain_costs::resolve(state, &snapshot.config, &peer, request, &mut response, common::time::now_ms())
        .map_err(|p| conflict("ONCHAIN_CROSS_CHAIN_COST_UNPROVEN", p))?;
    response.build_id = execution_build_id(&response, &response.swap_executions);
    for blocker in execution_contract_blockers(state, &snapshot.config, &peer, &response).await {
        push_unique(&mut response.blockers, &blocker);
    }
    response.submit_ready = response.blockers.is_empty();
    state
        .onchain_cross_chain_runs()
        .insert_build(response.clone(), now_ms)
        .map_err(|problem| {
            AppError::domain(
                StatusCode::SERVICE_UNAVAILABLE,
                "ONCHAIN_CROSS_CHAIN_LEDGER_UNAVAILABLE",
                format!("跨链闭环恢复账本不可写：{problem}"),
            )
        })?;
    Ok(response)
}

pub(crate) fn authorize(
    state: &AppState,
    request: &OnchainCrossChainAuthorizeRequest,
    actor: &str,
) -> Result<OnchainCrossChainRun, AppError> {
    if request.confirmation != ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            "ONCHAIN_CROSS_CHAIN_CONFIRMATION_REQUIRED",
            format!("必须完整输入授权短语：{ONCHAIN_CROSS_CHAIN_AUTHORIZATION_PHRASE}"),
        ));
    }
    let build_id = request.build_id.trim();
    let idempotency_key = request.idempotency_key.trim();
    if build_id.is_empty() || idempotency_key.is_empty() {
        return Err(AppError::domain(
            StatusCode::BAD_REQUEST,
            "ONCHAIN_CROSS_CHAIN_AUTHORIZATION_INVALID",
            "buildId 与 idempotencyKey 均不能为空",
        ));
    }
    state
        .onchain_cross_chain_runs()
        .readiness()
        .map_err(|problem| conflict("ONCHAIN_CROSS_CHAIN_RECOVERY_UNAVAILABLE", problem))?;
    state
        .onchain_cross_chain_runs()
        .authorize(build_id, idempotency_key, actor, common::time::now_ms())
        .map(|outcome| outcome.run)
        .map_err(map_authorize_error)
}

pub(crate) fn recent_runs(state: &AppState, limit: usize) -> OnchainCrossChainRunsResponse {
    state
        .onchain_cross_chain_runs()
        .runs(limit, common::time::now_ms())
}

fn map_authorize_error(error: CrossChainAuthorizeError) -> AppError {
    match error {
        CrossChainAuthorizeError::Missing => AppError::domain(
            StatusCode::NOT_FOUND,
            "ONCHAIN_CROSS_CHAIN_BUILD_MISSING",
            "跨链闭环合同不存在或已被清理，请重新生成",
        ),
        CrossChainAuthorizeError::Expired => conflict(
            "ONCHAIN_CROSS_CHAIN_BUILD_EXPIRED",
            "跨链闭环合同已过期，请基于最新逐腿报价重新生成",
        ),
        CrossChainAuthorizeError::NotReady(problem) => {
            conflict("ONCHAIN_CROSS_CHAIN_BUILD_NOT_READY", problem)
        }
        CrossChainAuthorizeError::IdempotencyConflict => conflict(
            "ONCHAIN_CROSS_CHAIN_IDEMPOTENCY_CONFLICT",
            "该幂等键已绑定另一条跨链闭环运行记录",
        ),
        CrossChainAuthorizeError::Persistence(problem) => conflict(
            "ONCHAIN_CROSS_CHAIN_AUTHORIZATION_NOT_DURABLE",
            format!("跨链闭环授权未能持久化，未创建运行记录：{problem}"),
        ),
    }
}

fn build_preview_from_snapshot(
    snapshot: &OnchainComparisonSnapshot,
    request: &OnchainCrossChainBuildRequest,
    now_ms: i64,
    max_age_ms: i64,
    quotes: Option<&OnchainCrossChainQuoteSet>,
) -> Result<OnchainCrossChainBuildResponse, AppError> {
    if !snapshot.config.enabled || !snapshot.config.cross_chain.enabled {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_DISABLED",
            "跨链闭环监控尚未启用",
        ));
    }
    let route = &snapshot.cross_chain;
    let quote_observed_at_ms = route.quote_observed_at_ms.ok_or_else(|| {
        conflict(
            "ONCHAIN_CROSS_CHAIN_QUOTE_PENDING",
            "跨链闭环还没有完整的四腿报价",
        )
    })?;
    if quote_observed_at_ms != request.expected_quote_observed_at_ms {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_SNAPSHOT_CHANGED",
            "跨链闭环报价已经变化，请按最新快照重新生成预览",
        ));
    }
    validate_preview_route(route)?;
    let mut valid_until_ms = quote_observed_at_ms.saturating_add(max_age_ms.max(1));
    if let Some(valuation) = &route.quote_usd_valuation {
        valid_until_ms = valid_until_ms.min(valuation.observed_at_ms.saturating_add(max_age_ms));
    }
    let bridge_executions = if let Some(quotes) = quotes {
        validate_execution_quotes(route, quotes, quote_observed_at_ms)?;
        valid_until_ms = valid_until_ms
            .min(quotes.outbound_bridge.valid_until_ms)
            .min(quotes.return_bridge.valid_until_ms);
        bridge_execution_contracts(quotes)
    } else {
        Vec::new()
    };
    if now_ms > valid_until_ms {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_QUOTE_STALE",
            "跨链闭环报价已经过期，请等待下一轮按需刷新",
        ));
    }
    let initial_quote_amount_raw = route.initial_quote_amount_raw.clone().ok_or_else(|| {
        conflict(
            "ONCHAIN_CROSS_CHAIN_AMOUNT_MISSING",
            "跨链闭环缺少源链入场数量",
        )
    })?;
    let final_quote_amount_raw = route.final_quote_amount_raw.clone().ok_or_else(|| {
        conflict(
            "ONCHAIN_CROSS_CHAIN_AMOUNT_MISSING",
            "跨链闭环缺少回链最少到账数量",
        )
    })?;
    let peer_chain = route
        .peer_chain
        .clone()
        .ok_or_else(|| conflict("ONCHAIN_CROSS_CHAIN_PEER_MISSING", "跨链闭环缺少目标链身份"))?;
    let mut blockers = preview_blockers(&snapshot.config, route);
    if let Err(problem) = economics::Valuation::read(
        &snapshot.config,
        route.quote_usd_valuation.as_ref(),
        max_age_ms,
        now_ms,
    ) {
        push_unique(&mut blockers, &problem);
    }
    let warnings = preview_warnings(route, &bridge_executions);
    let build_id = preview_build_id(snapshot, route, quote_observed_at_ms);
    Ok(OnchainCrossChainBuildResponse {
        approval_costs: Vec::new(),
        replenishment_costs: Vec::new(),
        build_id,
        provider: route.provider.clone(),
        source_chain: snapshot.config.chain.clone(),
        peer_chain,
        legs: route.legs.clone(),
        bridge_executions,
        swap_executions: Vec::new(),
        inventory: route.inventory.clone(),
        initial_quote_amount_raw,
        final_quote_amount_raw,
        gross_return_bps: route.gross_return_bps,
        stablecoin_risk_bps: route.stablecoin_risk_bps,
        bridge_fee_usd: route.bridge_fee_usd,
        gas_usd: route.gas_usd,
        quote_usd_valuation: route.quote_usd_valuation.clone(),
        total_cost_bps: route.total_cost_bps,
        net_return_bps: route.net_return_bps,
        estimated_duration_seconds: route.estimated_duration_seconds,
        quote_observed_at_ms,
        quote_latency_ms: route.quote_latency_ms,
        built_at_ms: now_ms,
        valid_until_ms,
        atomic: false,
        monitor_only: false,
        preview_ready: true,
        // The public builder still has to obtain swap contracts and check execution readiness.
        submit_ready: false,
        blockers,
        warnings,
    })
}

async fn build_swap_execution_contracts(
    state: &AppState,
    source: &OnchainComparisonConfig,
    peer: &OnchainComparisonConfig,
    legs: &[OnchainCrossChainLeg],
) -> Result<Vec<OnchainCrossChainSwapExecution>, AppError> {
    let source_leg = legs
        .iter()
        .find(|leg| leg.kind == OnchainCrossChainLegKind::SourceSwap)
        .ok_or_else(|| conflict("ONCHAIN_CROSS_CHAIN_ROUTE_INCOMPLETE", "缺少源链兑换腿"))?;
    let target_leg = legs
        .iter()
        .find(|leg| leg.kind == OnchainCrossChainLegKind::TargetSwap)
        .ok_or_else(|| conflict("ONCHAIN_CROSS_CHAIN_ROUTE_INCOMPLETE", "缺少目标链兑换腿"))?;
    let source_execution = build_swap_execution(
        state,
        source,
        source_leg.position,
        source_leg.kind,
        &source_leg.input_amount_raw,
        0,
    )
    .await?;
    let target_execution = build_swap_execution(
        state,
        peer,
        target_leg.position,
        target_leg.kind,
        &target_leg.input_amount_raw,
        2,
    )
    .await?;
    [
        (source_leg, source_execution),
        (target_leg, target_execution),
    ]
    .into_iter()
    .map(|(leg, execution)| {
        validate_swap_execution(leg, &execution)?;
        Ok(execution)
    })
    .collect()
}

pub(super) async fn build_swap_execution(
    state: &AppState,
    config: &OnchainComparisonConfig,
    position: u8,
    kind: OnchainCrossChainLegKind,
    input_amount_raw: &str,
    rebuild_after_position: u8,
) -> Result<OnchainCrossChainSwapExecution, AppError> {
    let direction = match kind {
        OnchainCrossChainLegKind::SourceSwap => {
            shared_types::OnchainComparisonDirection::BuyOnchainSellCex
        }
        OnchainCrossChainLegKind::TargetSwap => {
            shared_types::OnchainComparisonDirection::BuyCexSellOnchain
        }
        _ => {
            return Err(conflict(
                "ONCHAIN_CROSS_CHAIN_LEG_KIND_INVALID",
                "跨链桥腿不能按同链兑换构建",
            ));
        }
    };
    let contract = super::execution_build::build_firm_chain_contract(
        state,
        config,
        direction,
        input_amount_raw,
    )
    .await?;
    let expected_tokens = match kind {
        OnchainCrossChainLegKind::SourceSwap => (&config.quote_mint, &config.base_mint),
        OnchainCrossChainLegKind::TargetSwap => (&config.base_mint, &config.quote_mint),
        _ => unreachable!(),
    };
    if !token_matches(config, &contract.quote.input_address, expected_tokens.0)
        || !token_matches(config, &contract.quote.output_address, expected_tokens.1)
        || contract.quote.input_amount_raw != input_amount_raw
    {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_SNAPSHOT_CHANGED",
            format!("第 {position} 腿最新同链交易的代币或输入数量已变化"),
        ));
    }
    let execution_id = swap_execution_id(position, config, &contract);
    Ok(OnchainCrossChainSwapExecution {
        execution_id,
        position,
        kind,
        provider: config.provider.clone(),
        chain: config.chain.clone(),
        wallet_address: config.wallet_address.clone(),
        input_token: contract.quote.input_address,
        output_token: contract.quote.output_address,
        input_amount_raw: contract.quote.input_amount_raw,
        quoted_output_amount_raw: contract.quote.output_amount_raw,
        minimum_output_amount_raw: contract.minimum_output_amount_raw,
        transaction: contract.transaction,
        quote_observed_at_ms: contract.quote_observed_at_ms,
        valid_until_ms: contract.valid_until_ms,
        rebuild_after_position,
        official_docs_url: contract.official_docs_url,
    })
}

fn validate_swap_execution(
    leg: &OnchainCrossChainLeg,
    execution: &OnchainCrossChainSwapExecution,
) -> Result<(), AppError> {
    if execution.position != leg.position
        || execution.kind != leg.kind
        || !execution.input_token.eq_ignore_ascii_case(&leg.from_token)
        || !execution.output_token.eq_ignore_ascii_case(&leg.to_token)
        || execution.input_amount_raw != leg.input_amount_raw
    {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_SNAPSHOT_CHANGED",
            format!("第 {} 腿可签交易的代币或输入数量已变化", leg.position),
        ));
    }
    let minimum = execution.minimum_output_amount_raw.parse::<u128>().ok();
    let output = execution.quoted_output_amount_raw.parse::<u128>().ok();
    if !minimum
        .zip(output)
        .is_some_and(|(minimum, output)| minimum > 0 && output >= minimum)
    {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_MINIMUM_OUTPUT_REJECTED",
            format!("第 {} 腿最新可签交易缺少有效最低到账量", leg.position),
        ));
    }
    Ok(())
}

fn token_matches(config: &OnchainComparisonConfig, actual: &str, expected: &str) -> bool {
    if config.chain.eq_ignore_ascii_case("solana") {
        actual == expected
    } else {
        actual.eq_ignore_ascii_case(expected)
    }
}

fn swap_execution_id(
    position: u8,
    config: &OnchainComparisonConfig,
    contract: &super::execution_build::FirmChainContract,
) -> String {
    let transaction = match &contract.transaction {
        shared_types::OnchainUnsignedTransaction::SolanaVersioned {
            request_id,
            transaction_base64,
            ..
        } => format!("solana:{request_id}:{transaction_base64}"),
        shared_types::OnchainUnsignedTransaction::EvmCall {
            chain_id,
            from,
            to,
            data,
            value,
            gas,
            ..
        } => format!("evm:{chain_id}:{from}:{to}:{data}:{value}:{gas}"),
    };
    let canonical = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}|{}",
        position,
        config.provider,
        config.chain,
        config.wallet_address,
        contract.quote.input_address,
        contract.quote.output_address,
        contract.quote.input_amount_raw,
        contract.quote.output_amount_raw,
        contract.valid_until_ms,
        transaction,
    );
    let digest = common::signing::hmac_sha256_hex(SWAP_EXECUTION_ID_KEY, canonical.as_bytes());
    format!("onchain-cross-swap-{}", &digest[..24])
}

struct ExecutionReadinessRequest<'a> {
    label: String,
    config: &'a OnchainComparisonConfig,
    input_token: &'a str,
    input_amount_raw: &'a str,
    transaction: &'a shared_types::OnchainUnsignedTransaction,
    require_current_balance: bool,
}

async fn execution_contract_blockers(
    state: &AppState,
    source: &OnchainComparisonConfig,
    peer: &OnchainComparisonConfig,
    build: &OnchainCrossChainBuildResponse,
) -> Vec<String> {
    let mut requests = Vec::with_capacity(
        build
            .swap_executions
            .len()
            .saturating_add(build.bridge_executions.len()),
    );
    for execution in &build.swap_executions {
        let config = if execution.position == 1 {
            source
        } else {
            peer
        };
        requests.push(ExecutionReadinessRequest {
            label: format!("第 {} 腿同链兑换", execution.position),
            config,
            input_token: &execution.input_token,
            input_amount_raw: &execution.input_amount_raw,
            transaction: &execution.transaction,
            require_current_balance: execution.position == 1,
        });
    }
    for execution in &build.bridge_executions {
        let config = if execution.position == 2 {
            source
        } else {
            peer
        };
        requests.push(ExecutionReadinessRequest {
            label: format!("第 {} 腿跨链桥", execution.position),
            config,
            input_token: &execution.from_token,
            input_amount_raw: &execution.from_amount_raw,
            transaction: &execution.transaction,
            require_current_balance: false,
        });
    }
    futures::future::join_all(
        requests
            .into_iter()
            .map(|request| inspect_execution_contract(state, request)),
    )
    .await
    .into_iter()
    .flatten()
    .collect()
}

async fn inspect_execution_contract(
    state: &AppState,
    request: ExecutionReadinessRequest<'_>,
) -> Option<String> {
    let Some(required) = request
        .input_amount_raw
        .parse::<u128>()
        .ok()
        .filter(|amount| *amount > 0)
    else {
        return Some(format!("{}输入数量不是正数原始整数", request.label));
    };
    if request.require_current_balance {
        let balance = match super::wallet_inventory::exact_asset_balance(
            state,
            request.config,
            request.input_token,
        )
        .await
        {
            Ok(balance) => balance,
            Err(problem) => {
                return Some(format!("{}精确输入余额无法核验：{problem}", request.label));
            }
        };
        if balance.amount_raw < required {
            return Some(format!(
                "{}输入余额不足：可用原始数量 {}，需要 {} · source {}",
                request.label, balance.amount_raw, required, balance.source
            ));
        }
    }
    if let Err(problem) = inspect_native_fee_balance(state, &request, required).await {
        return Some(format!("{} Gas 余额无法核验：{problem}", request.label));
    }
    let shared_types::OnchainUnsignedTransaction::EvmCall {
        allowance_spender: Some(spender),
        ..
    } = request.transaction
    else {
        return None;
    };
    match super::allowance::inspect_exact_allowance(
        state,
        request.config,
        request.input_token,
        spender,
        request.input_amount_raw,
    )
    .await
    {
        Ok(evidence) if evidence.sufficient => None,
        Ok(evidence) => Some(format!(
            "{} ERC-20 授权不足：当前 {}，需要 {} · {}",
            request.label,
            evidence.current_amount_raw,
            request.input_amount_raw,
            evidence.official_docs_url
        )),
        Err(problem) => Some(format!("{} ERC-20 授权无法核验：{problem}", request.label)),
    }
}

async fn inspect_native_fee_balance(
    state: &AppState,
    request: &ExecutionReadinessRequest<'_>,
    required_input_raw: u128,
) -> Result<(), String> {
    let rpc_url = super::wallet_inventory::exact_read_rpc_url(state, request.config).await?;
    let (url, client) = super::rpc_target::rpc_target(&rpc_url).await?;
    let (available, required, source) = match request.transaction {
        shared_types::OnchainUnsignedTransaction::SolanaVersioned {
            transaction_base64, ..
        } => {
            let message = solana_message_base64(transaction_base64)?;
            let fee = super::rpc::rpc_result(
                &client,
                url.as_str(),
                "getFeeForMessage",
                serde_json::json!([message, { "commitment": "confirmed" }]),
                321,
            );
            let balance = super::rpc::rpc_result(
                &client,
                url.as_str(),
                "getBalance",
                serde_json::json!([
                    request.config.wallet_address,
                    { "commitment": "confirmed" }
                ]),
                322,
            );
            let (fee, balance) = tokio::join!(fee, balance);
            let fee = fee?
                .get("value")
                .and_then(serde_json::Value::as_u64)
                .map(u128::from)
                .ok_or_else(|| {
                    format!("Solana getFeeForMessage 未返回有效费用 · {SOLANA_FEE_DOCS}")
                })?;
            let available = balance?
                .get("value")
                .and_then(serde_json::Value::as_u64)
                .map(u128::from)
                .ok_or_else(|| "Solana getBalance 未返回有效 lamports".to_owned())?;
            let input = (request.input_token == super::wallet_inventory::SOLANA_WRAPPED_SOL_MINT)
                .then_some(required_input_raw)
                .unwrap_or_default();
            let required = input
                .checked_add(fee)
                .ok_or_else(|| "Solana 输入与网络费相加溢出".to_owned())?;
            (available, required, SOLANA_FEE_DOCS)
        }
        shared_types::OnchainUnsignedTransaction::EvmCall {
            gas,
            gas_price,
            value,
            ..
        } => {
            let gas_limit = parse_hex_quantity(gas)
                .ok_or_else(|| "EVM 交易 gas 不是合法 hex quantity".to_owned())?;
            let value = parse_hex_quantity(value)
                .ok_or_else(|| "EVM 交易 value 不是合法 hex quantity".to_owned())?;
            if request
                .input_token
                .eq_ignore_ascii_case(shared_types::EVM_NATIVE_TOKEN_ADDRESS)
                && value < required_input_raw
            {
                return Err("EVM 原生币交易 value 小于本腿输入数量".to_owned());
            }
            let gas_price = match gas_price.as_deref().and_then(parse_hex_quantity) {
                Some(price) => price,
                None => super::rpc::rpc_result(
                    &client,
                    url.as_str(),
                    "eth_gasPrice",
                    serde_json::json!([]),
                    323,
                )
                .await?
                .as_str()
                .and_then(parse_hex_quantity)
                .ok_or_else(|| "eth_gasPrice 未返回合法 hex quantity".to_owned())?,
            };
            let required = gas_limit
                .checked_mul(gas_price)
                .and_then(|fee| fee.checked_add(value))
                .ok_or_else(|| "EVM value 与最大网络费相加溢出".to_owned())?;
            let available = super::rpc::rpc_result(
                &client,
                url.as_str(),
                "eth_getBalance",
                serde_json::json!([request.config.wallet_address, "pending"]),
                324,
            )
            .await?
            .as_str()
            .and_then(parse_hex_quantity)
            .ok_or_else(|| "eth_getBalance 未返回合法 hex quantity".to_owned())?;
            (available, required, ETHEREUM_RPC_DOCS)
        }
    };
    if available < required {
        return Err(format!(
            "原生币可用原始数量 {available}，低于本腿 value 与网络费上限 {required} · {source}"
        ));
    }
    Ok(())
}

fn parse_hex_quantity(value: &str) -> Option<u128> {
    let value = value.strip_prefix("0x")?;
    if value.is_empty() {
        return None;
    }
    u128::from_str_radix(value, 16).ok()
}

fn solana_message_base64(transaction_base64: &str) -> Result<String, String> {
    let transaction = B64_STANDARD
        .decode(transaction_base64)
        .map_err(|_| "Solana 可签交易不是合法 base64".to_owned())?;
    let (signatures, prefix_len) = decode_shortvec(&transaction)?;
    let message_offset = signatures
        .checked_mul(64)
        .and_then(|length| prefix_len.checked_add(length))
        .ok_or_else(|| "Solana 签名区长度溢出".to_owned())?;
    let message = transaction
        .get(message_offset..)
        .filter(|message| !message.is_empty())
        .ok_or_else(|| "Solana 可签交易缺少消息体".to_owned())?;
    Ok(B64_STANDARD.encode(message))
}

fn decode_shortvec(bytes: &[u8]) -> Result<(usize, usize), String> {
    let mut value = 0_usize;
    for (index, byte) in bytes.iter().copied().take(3).enumerate() {
        value |= usize::from(byte & 0x7f) << (index * 7);
        if byte & 0x80 == 0 {
            return Ok((value, index + 1));
        }
    }
    Err("Solana 签名数量 shortvec 无效".to_owned())
}

pub(super) async fn validate_current_execution_contract(
    state: &AppState,
    config: &OnchainComparisonConfig,
    label: String,
    input_token: &str,
    input_amount_raw: &str,
    transaction: &shared_types::OnchainUnsignedTransaction,
) -> Result<(), String> {
    inspect_execution_contract(
        state,
        ExecutionReadinessRequest {
            label,
            config,
            input_token,
            input_amount_raw,
            transaction,
            require_current_balance: true,
        },
    )
    .await
    .map_or(Ok(()), Err)
}

fn validate_preview_route(route: &OnchainCrossChainSnapshot) -> Result<(), AppError> {
    let expected = [
        OnchainCrossChainLegKind::SourceSwap,
        OnchainCrossChainLegKind::OutboundBridge,
        OnchainCrossChainLegKind::TargetSwap,
        OnchainCrossChainLegKind::ReturnBridge,
    ];
    let ordered = route.preview_ready
        && route.legs.len() == expected.len()
        && route
            .legs
            .iter()
            .zip(expected)
            .enumerate()
            .all(|(index, (leg, kind))| leg.position == (index + 1) as u8 && leg.kind == kind);
    let closed = ordered
        && route.legs[0]
            .to_token
            .eq_ignore_ascii_case(&route.legs[1].from_token)
        && route.legs[1]
            .to_token
            .eq_ignore_ascii_case(&route.legs[2].from_token)
        && route.legs[2]
            .to_token
            .eq_ignore_ascii_case(&route.legs[3].from_token)
        && route.legs[3]
            .to_token
            .eq_ignore_ascii_case(&route.legs[0].from_token)
        && route.legs[0]
            .from_chain
            .eq_ignore_ascii_case(&route.legs[0].to_chain)
        && route.legs[1]
            .from_chain
            .eq_ignore_ascii_case(&route.legs[0].to_chain)
        && route.legs[1]
            .to_chain
            .eq_ignore_ascii_case(&route.legs[2].from_chain)
        && route.legs[2]
            .from_chain
            .eq_ignore_ascii_case(&route.legs[2].to_chain)
        && route.legs[3]
            .from_chain
            .eq_ignore_ascii_case(&route.legs[2].to_chain)
        && route.legs[3]
            .to_chain
            .eq_ignore_ascii_case(&route.legs[0].from_chain);
    if !closed {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_ROUTE_INCOMPLETE",
            "跨链闭环必须包含源链兑换、Base 跨链、目标链兑换和 Quote 回链四条腿",
        ));
    }
    Ok(())
}

fn preview_blockers(
    source: &OnchainComparisonConfig,
    route: &OnchainCrossChainSnapshot,
) -> Vec<String> {
    let mut blockers = Vec::new();
    if route.quality != OnchainCrossChainQuality::Fresh {
        if let Some(problem) = route.problem.as_deref() {
            push_unique(&mut blockers, problem);
        }
    }
    if route.net_return_bps.is_none() {
        push_unique(&mut blockers, "成本证据不完整，不能确认费后净收益");
    } else if route
        .net_return_bps
        .is_some_and(|net| net <= 0.0 || net < source.spread_alert.min_net_spread_bps)
    {
        push_unique(
            &mut blockers,
            &format!(
                "当前费后回报低于配置的最低收益 {:.4}%",
                source.spread_alert.min_net_spread_bps / 100.0
            ),
        );
    }
    for item in &route.inventory {
        if item.status == OnchainCrossChainInventoryStatus::Insufficient {
            push_unique(
                &mut blockers,
                &format!(
                    "{} {} 库存{}",
                    item.chain,
                    item.asset,
                    match item.status {
                        OnchainCrossChainInventoryStatus::Insufficient => "不足",
                        OnchainCrossChainInventoryStatus::Unknown => "待核验",
                        OnchainCrossChainInventoryStatus::Ready => "已就绪",
                    }
                ),
            );
        }
    }
    blockers
}

fn preview_warnings(
    route: &OnchainCrossChainSnapshot,
    bridge_executions: &[OnchainCrossChainBridgeExecution],
) -> Vec<String> {
    let mut warnings = vec![
        "路径包含两次跨链桥，无法原子成交；每腿完成后会按真实到账重新报价".to_owned(),
        format!(
            "已预留 {:.4}% 报价币汇率风险缓冲",
            f64::from(route.stablecoin_risk_bps) / 100.0
        ),
    ];
    for execution in bridge_executions {
        push_unique(
            &mut warnings,
            &format!(
                "第 {} 腿已取得 LI.FI 可签交易，但必须在第 {} 腿确认后按实际到账数量重新报价",
                execution.position, execution.rebuild_after_position
            ),
        );
    }
    warnings
}

fn validate_execution_quotes(
    route: &OnchainCrossChainSnapshot,
    quotes: &OnchainCrossChainQuoteSet,
    quote_observed_at_ms: i64,
) -> Result<(), AppError> {
    if quotes.observed_at_ms != quote_observed_at_ms {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_SNAPSHOT_CHANGED",
            "跨链交易合同与当前四腿快照不是同一批证据",
        ));
    }
    let bridge_rows = [
        (&route.legs[1], &quotes.outbound_bridge),
        (&route.legs[3], &quotes.return_bridge),
    ];
    if bridge_rows.iter().any(|(leg, quote)| {
        leg.route_id.as_deref() != Some(quote.route_id.as_str())
            || leg.input_amount_raw != quote.from_amount_raw
            || leg.minimum_output_amount_raw.as_deref() != Some(quote.to_amount_min_raw.as_str())
            || !leg.from_token.eq_ignore_ascii_case(&quote.from_token)
            || !leg.to_token.eq_ignore_ascii_case(&quote.to_token)
    }) {
        return Err(conflict(
            "ONCHAIN_CROSS_CHAIN_SNAPSHOT_CHANGED",
            "跨链交易合同的路径、代币或数量与当前四腿快照不一致",
        ));
    }
    Ok(())
}

fn bridge_execution_contracts(
    quotes: &OnchainCrossChainQuoteSet,
) -> Vec<OnchainCrossChainBridgeExecution> {
    [
        (
            2,
            OnchainCrossChainLegKind::OutboundBridge,
            1,
            &quotes.outbound_bridge,
        ),
        (
            4,
            OnchainCrossChainLegKind::ReturnBridge,
            3,
            &quotes.return_bridge,
        ),
    ]
    .into_iter()
    .map(|(position, kind, rebuild_after_position, quote)| {
        bridge_execution_contract(position, kind, rebuild_after_position, quote)
    })
    .collect()
}

pub(super) fn bridge_execution_contract(
    position: u8,
    kind: OnchainCrossChainLegKind,
    rebuild_after_position: u8,
    quote: &OnchainBridgeQuote,
) -> OnchainCrossChainBridgeExecution {
    OnchainCrossChainBridgeExecution {
        position,
        kind,
        provider: quote.provider.clone(),
        route_id: quote.route_id.clone(),
        transaction_id: quote.transaction_id.clone(),
        tool: quote.tool.clone(),
        from_chain_id: quote.from_chain_id,
        to_chain_id: quote.to_chain_id,
        from_address: quote.from_address.clone(),
        to_address: quote.to_address.clone(),
        from_token: quote.from_token.clone(),
        to_token: quote.to_token.clone(),
        from_amount_raw: quote.from_amount_raw.clone(),
        to_amount_min_raw: quote.to_amount_min_raw.clone(),
        approval_address: quote.approval_address.clone(),
        transaction: quote.transaction.clone(),
        quote_observed_at_ms: quote.observed_at_ms,
        valid_until_ms: quote.valid_until_ms,
        rebuild_after_position,
        official_docs_url: quote.official_docs_url.clone(),
    }
}

fn push_unique(rows: &mut Vec<String>, message: &str) {
    if !message.is_empty() && !rows.iter().any(|row| row == message) {
        rows.push(message.to_owned());
    }
}

fn preview_build_id(
    snapshot: &OnchainComparisonSnapshot,
    route: &OnchainCrossChainSnapshot,
    quote_observed_at_ms: i64,
) -> String {
    let legs = route
        .legs
        .iter()
        .map(|leg| {
            format!(
                "{}:{}:{}:{}:{}",
                leg.position,
                leg.provider,
                leg.route_id.as_deref().unwrap_or_default(),
                leg.input_amount_raw,
                leg.minimum_output_amount_raw
                    .as_deref()
                    .unwrap_or(&leg.expected_output_amount_raw)
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    let canonical = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        snapshot.config.chain,
        route.peer_chain.as_deref().unwrap_or_default(),
        route.peer_item_id,
        quote_observed_at_ms,
        legs,
        serde_json::to_string(&route.quote_usd_valuation).unwrap_or_default(),
        route.stablecoin_risk_bps
    );
    let digest = common::signing::hmac_sha256_hex(BUILD_ID_KEY, canonical.as_bytes());
    format!("onchain-cross-preview-{}", &digest[..24])
}

fn execution_build_id(
    response: &OnchainCrossChainBuildResponse,
    swaps: &[OnchainCrossChainSwapExecution],
) -> String {
    let swap_identity = swaps
        .iter()
        .map(|swap| {
            let transaction = match &swap.transaction {
                shared_types::OnchainUnsignedTransaction::SolanaVersioned {
                    request_id, ..
                } => request_id.clone(),
                shared_types::OnchainUnsignedTransaction::EvmCall {
                    chain_id,
                    from,
                    to,
                    data,
                    value,
                    ..
                } => format!("{chain_id}:{from}:{to}:{data}:{value}"),
            };
            format!(
                "{}:{}:{}:{}:{}",
                swap.position,
                swap.input_amount_raw,
                swap.quoted_output_amount_raw,
                swap.valid_until_ms,
                transaction
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    let canonical = format!(
        "{}|{}|{}|{}|{}",
        response.build_id, response.built_at_ms, response.valid_until_ms, swap_identity,
        serde_json::json!({"approval":response.approval_costs,"replenishment":response.replenishment_costs})
    );
    let digest = common::signing::hmac_sha256_hex(BUILD_ID_KEY, canonical.as_bytes());
    format!("onchain-cross-build-{}", &digest[..24])
}

fn conflict(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::CONFLICT, code, message.into())
}

pub(super) fn peer_config(
    state: &AppState,
    source: &OnchainComparisonConfig,
) -> Option<(String, OnchainComparisonConfig)> {
    let requested = source.cross_chain.peer_item_id.trim();
    (!requested.is_empty()).then_some(())?;
    state
        .onchain_monitor()
        .batch()
        .configs()
        .into_iter()
        .find(|(item_id, _)| item_id == requested)
}

pub(super) async fn fetch(
    source: &OnchainComparisonConfig,
    peer_item_id: &str,
    peer: &OnchainComparisonConfig,
    primary: &OnchainQuotePair,
    started_at_ms: i64,
) -> Result<OnchainCrossChainQuoteSet, String> {
    if !source.cross_chain.enabled || source.cross_chain.provider != "lifi" {
        return Err("cross-chain monitoring is disabled or has no supported provider".to_owned());
    }
    if source.chain.eq_ignore_ascii_case(&peer.chain) {
        return Err("cross-chain peer must use a different chain".to_owned());
    }
    if !primary.matches_config(source) {
        return Err("source DEX quote does not match the cross-chain asset scope".to_owned());
    }
    if source.wallet_address.trim().is_empty() || peer.wallet_address.trim().is_empty() {
        return Err("cross-chain monitoring requires wallet addresses on both chains".to_owned());
    }
    ensure_quote(
        &primary.reverse,
        &source.quote_mint,
        &source.base_mint,
        &source.quote_amount_raw,
        "source DEX buy",
    )?;

    let outbound_bridge = super::lifi::fetch_bridge(
        &source.chain,
        &peer.chain,
        &source.base_mint,
        &peer.base_mint,
        &primary.reverse.output_amount_raw,
        &source.wallet_address,
        &peer.wallet_address,
        source.slippage_bps,
    )
    .await?;
    let target_swap = fetch_exact_in(
        peer,
        &peer.provider,
        &peer.base_mint,
        &peer.quote_mint,
        &outbound_bridge.to_amount_min_raw,
    )
    .await?;
    ensure_quote(
        &target_swap,
        &peer.base_mint,
        &peer.quote_mint,
        &outbound_bridge.to_amount_min_raw,
        "target DEX sell",
    )?;
    let return_bridge = super::lifi::fetch_bridge(
        &peer.chain,
        &source.chain,
        &peer.quote_mint,
        &source.quote_mint,
        &target_swap.output_amount_raw,
        &peer.wallet_address,
        &source.wallet_address,
        peer.slippage_bps,
    )
    .await?;

    let observed_at_ms = common::time::now_ms();
    Ok(OnchainCrossChainQuoteSet {
        peer_item_id: peer_item_id.to_owned(),
        source_chain: source.chain.clone(),
        source_provider: source.provider.clone(),
        source_base_address: source.base_mint.clone(),
        source_quote_address: source.quote_mint.clone(),
        source_wallet_address: source.wallet_address.clone(),
        peer_chain: peer.chain.clone(),
        peer_provider: peer.provider.clone(),
        peer_base_address: peer.base_mint.clone(),
        peer_quote_address: peer.quote_mint.clone(),
        peer_wallet_address: peer.wallet_address.clone(),
        source_swap: primary.reverse.clone(),
        outbound_bridge,
        target_swap,
        return_bridge,
        observed_at_ms,
        request_latency_ms: observed_at_ms.saturating_sub(started_at_ms),
    })
}

fn ensure_quote(
    quote: &ProviderQuote,
    input_token: &str,
    output_token: &str,
    input_amount_raw: &str,
    label: &str,
) -> Result<(), String> {
    if !quote.input_address.eq_ignore_ascii_case(input_token)
        || !quote.output_address.eq_ignore_ascii_case(output_token)
    {
        return Err(format!("{label} returned a different token path"));
    }
    if quote.input_amount_raw != input_amount_raw {
        return Err(format!("{label} changed the exact input amount"));
    }
    if !quote
        .output_amount_raw
        .parse::<u128>()
        .is_ok_and(|amount| amount > 0)
    {
        return Err(format!("{label} returned an invalid output amount"));
    }
    Ok(())
}

pub(super) fn attach(state: &AppState, snapshot: &mut OnchainComparisonSnapshot, now_ms: i64) {
    let source = &snapshot.config;
    if !source.cross_chain.enabled {
        snapshot.cross_chain = disabled_snapshot(source, now_ms);
        return;
    }
    let Some((peer_item_id, peer)) = peer_config(state, source) else {
        snapshot.cross_chain = missing_peer_snapshot(source, now_ms);
        return;
    };
    let Some(quotes) = state.onchain_monitor().cross_chain_quotes() else {
        snapshot.cross_chain = unavailable_snapshot(
            source,
            Some(&peer),
            state
                .onchain_monitor()
                .cross_chain_problem()
                .as_deref()
                .map(String::as_str),
            now_ms,
        );
        return;
    };
    if !quotes.matches_configs(source, &peer_item_id, &peer) {
        snapshot.cross_chain = unavailable_snapshot(
            source,
            Some(&peer),
            Some("缓存中的跨链闭环不属于当前源链与目标链配置，正在重新取证"),
            now_ms,
        );
        return;
    }
    let valuation = super::usd_valuation::quote_evidence(state, source, now_ms).ok();
    snapshot.cross_chain = project(
        source,
        &peer,
        &quotes,
        state.onchain_monitor().wallet_inventory().as_deref(),
        valuation.as_ref(),
        now_ms,
    );
}

pub(super) fn attach_batch(
    state: &AppState,
    item_id: &str,
    snapshot: &mut OnchainComparisonSnapshot,
    now_ms: i64,
) {
    let source = &snapshot.config;
    if !source.cross_chain.enabled {
        snapshot.cross_chain = disabled_snapshot(source, now_ms);
        return;
    }
    let Some((peer_item_id, peer)) = peer_config(state, source) else {
        snapshot.cross_chain = missing_peer_snapshot(source, now_ms);
        return;
    };
    let Some(quotes) = state.onchain_monitor().batch().cross_chain_quotes(item_id) else {
        let problem = state.onchain_monitor().batch().cross_chain_problem(item_id);
        snapshot.cross_chain =
            unavailable_snapshot(source, Some(&peer), problem.as_deref(), now_ms);
        return;
    };
    if quotes.matches_configs(source, &peer_item_id, &peer) {
        let valuation = super::usd_valuation::quote_evidence(state, source, now_ms).ok();
        snapshot.cross_chain = project(source, &peer, &quotes, None, valuation.as_ref(), now_ms);
    } else {
        snapshot.cross_chain = unavailable_snapshot(
            source,
            Some(&peer),
            Some("批量监控中的跨链闭环不属于当前源链与目标链配置，正在重新取证"),
            now_ms,
        );
    }
}

fn disabled_snapshot(source: &OnchainComparisonConfig, now_ms: i64) -> OnchainCrossChainSnapshot {
    OnchainCrossChainSnapshot {
        provider: source.cross_chain.provider.clone(),
        peer_item_id: source.cross_chain.peer_item_id.clone(),
        quality: OnchainCrossChainQuality::Disabled,
        observed_at_ms: now_ms,
        ..Default::default()
    }
}

fn missing_peer_snapshot(
    source: &OnchainComparisonConfig,
    now_ms: i64,
) -> OnchainCrossChainSnapshot {
    OnchainCrossChainSnapshot {
        provider: source.cross_chain.provider.clone(),
        peer_item_id: source.cross_chain.peer_item_id.clone(),
        quality: OnchainCrossChainQuality::PeerMissing,
        problem: Some("所选目标链市场已不存在；请从监控列表重新选择目标市场".to_owned()),
        observed_at_ms: now_ms,
        ..Default::default()
    }
}

fn unavailable_snapshot(
    source: &OnchainComparisonConfig,
    peer: Option<&OnchainComparisonConfig>,
    problem: Option<&str>,
    now_ms: i64,
) -> OnchainCrossChainSnapshot {
    OnchainCrossChainSnapshot {
        provider: source.cross_chain.provider.clone(),
        peer_item_id: source.cross_chain.peer_item_id.clone(),
        peer_chain: peer.map(|peer| peer.chain.clone()),
        quality: if problem.is_some() {
            OnchainCrossChainQuality::UpstreamUnavailable
        } else {
            OnchainCrossChainQuality::Pending
        },
        problem: problem.map(str::to_owned),
        observed_at_ms: now_ms,
        ..Default::default()
    }
}

pub(super) fn project(
    source: &OnchainComparisonConfig,
    peer: &OnchainComparisonConfig,
    quotes: &OnchainCrossChainQuoteSet,
    wallet_inventory: Option<&OnchainWalletInventory>,
    quote_usd_valuation: Option<&shared_types::OnchainUsdValuation>,
    now_ms: i64,
) -> OnchainCrossChainSnapshot {
    let initial = quotes.source_swap.input_amount_raw.parse::<u128>().ok();
    let final_amount = quotes.return_bridge.to_amount_min_raw.parse::<u128>().ok();
    let gross_return_bps = initial
        .zip(final_amount)
        .and_then(|(initial, final_amount)| {
            (initial > 0 && final_amount > 0)
                .then(|| (final_amount as f64 / initial as f64 - 1.0) * 10_000.0)
        });
    let execution_buffer_bps = source.slippage_bps.max(0.0) + peer.slippage_bps.max(0.0);
    let bridge_fee_usd =
        sum_complete([quotes.outbound_bridge.fee_usd, quotes.return_bridge.fee_usd]);
    let bridge_gas_usd =
        sum_complete([quotes.outbound_bridge.gas_usd, quotes.return_bridge.gas_usd]);
    let gas_usd = sum_complete([bridge_gas_usd, Some(source.gas_usd), Some(peer.gas_usd)]);
    let stablecoin_risk_bps = if source.quote_token.eq_ignore_ascii_case("USD") {
        0
    } else {
        source.cross_chain.stablecoin_risk_bps
    };
    let max_age_ms = source.max_age_ms.min(peer.max_age_ms);
    let valuation = economics::Valuation::read(source, quote_usd_valuation, max_age_ms, now_ms);
    let gas_raw = valuation
        .as_ref()
        .map_err(Clone::clone)
        .and_then(|valuation| {
            valuation.gas_raw(
                gas_usd.ok_or("桥报价缺少完整 Gas 美元证据")?,
                source.quote_decimals,
            )
        });
    let gas_bps = gas_raw
        .as_ref()
        .ok()
        .zip(initial.filter(|amount| *amount > 0))
        .map(|(gas_raw, initial)| *gas_raw as f64 / initial as f64 * 10_000.0);
    let total_cost_bps =
        gas_bps.map(|gas| gas + execution_buffer_bps + f64::from(stablecoin_risk_bps));
    let net_return_bps = gross_return_bps
        .zip(total_cost_bps)
        .map(|(gross, cost)| gross - cost);
    let age_ms = now_ms.saturating_sub(quotes.observed_at_ms);
    let quality = if age_ms < 0 || age_ms > max_age_ms {
        OnchainCrossChainQuality::Stale
    } else if net_return_bps.is_none() {
        OnchainCrossChainQuality::EvidencePending
    } else if net_return_bps.is_some_and(|net| net <= 0.0) {
        OnchainCrossChainQuality::NoNetProfit
    } else {
        OnchainCrossChainQuality::Fresh
    };
    let estimated_duration_seconds = quotes
        .outbound_bridge
        .execution_duration_seconds
        .zip(quotes.return_bridge.execution_duration_seconds)
        .map(|(outbound, returned)| outbound.saturating_add(returned));
    let problem = match quality {
        OnchainCrossChainQuality::Fresh => Some(
            "跨链路径含两次非原子桥接；已完成闭环报价，但当前只监控和构建预览，不自动提交"
                .to_owned(),
        ),
        OnchainCrossChainQuality::NoNetProfit => {
            Some("完整往返路径扣除 DEX 缓冲和 Gas 后没有正收益".to_owned())
        }
        OnchainCrossChainQuality::Stale => {
            Some("跨链闭环报价已超过最短证据时效，等待下一轮按需刷新".to_owned())
        }
        OnchainCrossChainQuality::EvidencePending => Some(
            gas_raw
                .err()
                .unwrap_or_else(|| "跨链成本或数量证据不完整，保留原始报价供观察".to_owned()),
        ),
        _ => None,
    };
    OnchainCrossChainSnapshot {
        provider: "lifi".to_owned(),
        peer_item_id: quotes.peer_item_id.clone(),
        peer_chain: Some(peer.chain.clone()),
        quality,
        legs: vec![
            swap_leg(
                1,
                OnchainCrossChainLegKind::SourceSwap,
                source,
                &quotes.source_swap,
                quotes.observed_at_ms,
            ),
            bridge_leg(
                2,
                OnchainCrossChainLegKind::OutboundBridge,
                source,
                peer,
                &quotes.outbound_bridge,
            ),
            swap_leg(
                3,
                OnchainCrossChainLegKind::TargetSwap,
                peer,
                &quotes.target_swap,
                quotes.observed_at_ms,
            ),
            bridge_leg(
                4,
                OnchainCrossChainLegKind::ReturnBridge,
                peer,
                source,
                &quotes.return_bridge,
            ),
        ],
        initial_quote_amount_raw: Some(quotes.source_swap.input_amount_raw.clone()),
        final_quote_amount_raw: Some(quotes.return_bridge.to_amount_min_raw.clone()),
        gross_return_bps,
        execution_buffer_bps: Some(execution_buffer_bps),
        stablecoin_risk_bps,
        bridge_fee_usd,
        gas_usd,
        quote_usd_valuation: valuation.ok().map(|valuation| valuation.evidence),
        total_cost_bps,
        net_return_bps,
        estimated_duration_seconds,
        inventory: inventory_requirements(source, peer, wallet_inventory),
        atomic: false,
        preview_ready: true,
        submit_ready: false,
        problem,
        quote_observed_at_ms: Some(quotes.observed_at_ms),
        quote_latency_ms: Some(quotes.request_latency_ms),
        observed_at_ms: now_ms,
    }
}

fn swap_leg(
    position: u8,
    kind: OnchainCrossChainLegKind,
    config: &OnchainComparisonConfig,
    quote: &ProviderQuote,
    observed_at_ms: i64,
) -> OnchainCrossChainLeg {
    let docs =
        onchain_quote_provider(&config.provider).map_or("", |provider| provider.official_docs_url);
    OnchainCrossChainLeg {
        position,
        kind,
        provider: config.provider.clone(),
        from_chain: config.chain.clone(),
        to_chain: config.chain.clone(),
        from_asset: if quote.input_address.eq_ignore_ascii_case(&config.base_mint) {
            config.base_token.clone()
        } else {
            config.quote_token.clone()
        },
        to_asset: if quote.output_address.eq_ignore_ascii_case(&config.base_mint) {
            config.base_token.clone()
        } else {
            config.quote_token.clone()
        },
        from_token: quote.input_address.clone(),
        to_token: quote.output_address.clone(),
        input_amount_raw: quote.input_amount_raw.clone(),
        expected_output_amount_raw: quote.output_amount_raw.clone(),
        minimum_output_amount_raw: None,
        input_decimals: if quote.input_address.eq_ignore_ascii_case(&config.base_mint) {
            config.base_decimals
        } else {
            config.quote_decimals
        },
        output_decimals: if quote.output_address.eq_ignore_ascii_case(&config.base_mint) {
            config.base_decimals
        } else {
            config.quote_decimals
        },
        fee_usd: None,
        gas_usd: Some(config.gas_usd.max(0.0)),
        estimated_duration_seconds: None,
        route_id: quote.router.clone(),
        route_tools: Vec::new(),
        official_docs_url: docs.to_owned(),
        observed_at_ms,
    }
}

fn bridge_leg(
    position: u8,
    kind: OnchainCrossChainLegKind,
    from: &OnchainComparisonConfig,
    to: &OnchainComparisonConfig,
    quote: &OnchainBridgeQuote,
) -> OnchainCrossChainLeg {
    OnchainCrossChainLeg {
        position,
        kind,
        provider: quote.provider.clone(),
        from_chain: from.chain.clone(),
        to_chain: to.chain.clone(),
        from_asset: if quote.from_token.eq_ignore_ascii_case(&from.base_mint) {
            from.base_token.clone()
        } else {
            from.quote_token.clone()
        },
        to_asset: if quote.to_token.eq_ignore_ascii_case(&to.base_mint) {
            to.base_token.clone()
        } else {
            to.quote_token.clone()
        },
        from_token: quote.from_token.clone(),
        to_token: quote.to_token.clone(),
        input_amount_raw: quote.from_amount_raw.clone(),
        expected_output_amount_raw: quote.to_amount_raw.clone(),
        minimum_output_amount_raw: Some(quote.to_amount_min_raw.clone()),
        input_decimals: if quote.from_token.eq_ignore_ascii_case(&from.base_mint) {
            from.base_decimals
        } else {
            from.quote_decimals
        },
        output_decimals: if quote.to_token.eq_ignore_ascii_case(&to.base_mint) {
            to.base_decimals
        } else {
            to.quote_decimals
        },
        fee_usd: quote.fee_usd,
        gas_usd: quote.gas_usd,
        estimated_duration_seconds: quote.execution_duration_seconds,
        route_id: Some(quote.route_id.clone()),
        route_tools: quote.route_tools.clone(),
        official_docs_url: quote.official_docs_url.clone(),
        observed_at_ms: quote.observed_at_ms,
    }
}

fn inventory_requirements(
    source: &OnchainComparisonConfig,
    peer: &OnchainComparisonConfig,
    wallet: Option<&OnchainWalletInventory>,
) -> Vec<OnchainCrossChainInventoryRequirement> {
    let required = source.quote_amount_raw.parse::<u128>().ok();
    let available = wallet
        .filter(|wallet| wallet.matches_config(source))
        .and_then(|wallet| wallet.quote.available)
        .and_then(|available| units_to_raw(available, source.quote_decimals));
    let status = match (required, available) {
        (Some(required), Some(available)) if available >= required => {
            OnchainCrossChainInventoryStatus::Ready
        }
        (Some(_), Some(_)) => OnchainCrossChainInventoryStatus::Insufficient,
        _ => OnchainCrossChainInventoryStatus::Unknown,
    };
    let gas_requirement =
        |config: &OnchainComparisonConfig, available: Option<f64>, source: bool| {
            let decimals = if config.chain.eq_ignore_ascii_case("solana") {
                9
            } else {
                18
            };
            let available_amount_raw = available.and_then(|units| units_to_raw(units, decimals));
            OnchainCrossChainInventoryRequirement {
                chain: config.chain.clone(),
                wallet_address: config.wallet_address.clone(),
                asset: format!("{} Gas", config.chain.to_uppercase()),
                token_address: String::new(),
                required_amount_raw: None,
                available_amount_raw: available_amount_raw.map(|amount| amount.to_string()),
                status: if available.is_some_and(|amount| amount > 0.0) {
                    OnchainCrossChainInventoryStatus::Ready
                } else {
                    OnchainCrossChainInventoryStatus::Unknown
                },
                problem: Some(
                    if source {
                        "源链 Gas 有余额，但构建前仍需按真实交易重新估算所需数量"
                    } else {
                        "目标链 Gas 余额需在构建前按目标钱包实时核验"
                    }
                    .to_owned(),
                ),
            }
        };
    let source_gas_available = wallet
        .filter(|wallet| wallet.matches_config(source))
        .and_then(|wallet| wallet.gas.available);
    vec![
        OnchainCrossChainInventoryRequirement {
            chain: source.chain.clone(),
            wallet_address: source.wallet_address.clone(),
            asset: source.quote_token.clone(),
            token_address: source.quote_mint.clone(),
            required_amount_raw: required.map(|amount| amount.to_string()),
            available_amount_raw: available.map(|amount| amount.to_string()),
            status,
            problem: (status == OnchainCrossChainInventoryStatus::Unknown)
                .then(|| "源链 Quote 余额尚未由 RPC 证明".to_owned())
                .or_else(|| {
                    (status == OnchainCrossChainInventoryStatus::Insufficient)
                        .then(|| "源链 Quote 余额不足以启动闭环".to_owned())
                }),
        },
        gas_requirement(source, source_gas_available, true),
        gas_requirement(peer, None, false),
    ]
}

fn units_to_raw(units: f64, decimals: u8) -> Option<u128> {
    let raw = units * 10_f64.powi(i32::from(decimals));
    (raw.is_finite() && raw >= 0.0 && raw <= u128::MAX as f64).then(|| raw.floor() as u128)
}

fn sum_complete<const N: usize>(values: [Option<f64>; N]) -> Option<f64> {
    values.into_iter().try_fold(0.0, |total, value| {
        value
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(|value| total + value)
            .filter(|total| total.is_finite())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(chain: &str, quote: &str, quote_decimals: u8) -> OnchainComparisonConfig {
        OnchainComparisonConfig {
            enabled: true,
            chain: chain.to_owned(),
            provider: "zeroex_swap_v2".to_owned(),
            base_token: "PUPS".to_owned(),
            quote_token: quote.to_owned(),
            base_mint: format!("{chain}-base"),
            quote_mint: format!("{chain}-quote"),
            base_decimals: 18,
            quote_decimals,
            quote_amount_raw: "100000000".to_owned(),
            wallet_address: format!("{chain}-wallet"),
            gas_usd: 0.10,
            slippage_bps: 5.0,
            max_age_ms: 10_000,
            ..OnchainComparisonConfig::default()
        }
    }

    #[test]
    fn extracts_message_from_a_serialized_solana_transaction() {
        let mut transaction = vec![2_u8];
        transaction.extend([0_u8; 128]);
        transaction.extend([0x80, 0x01, 0x02, 0x03]);
        let encoded = B64_STANDARD.encode(transaction);
        let expected = B64_STANDARD.encode([0x80, 0x01, 0x02, 0x03]);

        assert_eq!(
            solana_message_base64(&encoded).ok().as_deref(),
            Some(expected.as_str())
        );
        assert!(solana_message_base64(&B64_STANDARD.encode([2_u8, 0_u8])).is_err());
    }

    fn bridge(
        from: &OnchainComparisonConfig,
        to: &OnchainComparisonConfig,
        amount: &str,
        output: &str,
    ) -> OnchainBridgeQuote {
        OnchainBridgeQuote {
            provider: "lifi".to_owned(),
            route_id: "route".to_owned(),
            transaction_id: "transaction".to_owned(),
            tool: "across".to_owned(),
            route_tools: vec!["across".to_owned()],
            from_chain_id: 1,
            to_chain_id: 2,
            from_token: from.base_mint.clone(),
            to_token: to.base_mint.clone(),
            from_address: from.wallet_address.clone(),
            to_address: to.wallet_address.clone(),
            from_amount_raw: amount.to_owned(),
            to_amount_raw: output.to_owned(),
            to_amount_min_raw: output.to_owned(),
            fee_usd: Some(0.20),
            gas_usd: Some(0.05),
            execution_duration_seconds: Some(10),
            approval_address: Some("0x5555555555555555555555555555555555555555".to_owned()),
            transaction: shared_types::OnchainUnsignedTransaction::EvmCall {
                chain_id: 1,
                from: from.wallet_address.clone(),
                to: "0x6666666666666666666666666666666666666666".to_owned(),
                data: "0x1234".to_owned(),
                value: "0x0".to_owned(),
                gas: "0x5208".to_owned(),
                gas_price: Some("0x10".to_owned()),
                max_priority_fee_per_gas: None,
                allowance_spender: Some("0x5555555555555555555555555555555555555555".to_owned()),
            },
            official_docs_url: super::super::lifi::LIFI_QUOTE_DOCS.to_owned(),
            observed_at_ms: 1_000,
            valid_until_ms: 56_000,
        }
    }

    #[test]
    fn closed_cycle_uses_return_bridge_minimum_and_all_gas_budgets() {
        let source = config("base", "USDC", 6);
        let peer = config("arbitrum", "USDT", 6);
        let mut outbound = bridge(&source, &peer, "1000000000000000000", "995000000000000000");
        outbound.to_token = peer.base_mint.clone();
        let mut returned = bridge(&peer, &source, "101300000", "101300000");
        returned.from_token = peer.quote_mint.clone();
        returned.to_token = source.quote_mint.clone();
        let quotes = OnchainCrossChainQuoteSet {
            peer_item_id: "peer".to_owned(),
            source_chain: source.chain.clone(),
            source_provider: source.provider.clone(),
            source_base_address: source.base_mint.clone(),
            source_quote_address: source.quote_mint.clone(),
            source_wallet_address: source.wallet_address.clone(),
            peer_chain: peer.chain.clone(),
            peer_provider: peer.provider.clone(),
            peer_base_address: peer.base_mint.clone(),
            peer_quote_address: peer.quote_mint.clone(),
            peer_wallet_address: peer.wallet_address.clone(),
            source_swap: ProviderQuote {
                input_address: source.quote_mint.clone(),
                output_address: source.base_mint.clone(),
                input_amount_raw: "100000000".to_owned(),
                output_amount_raw: "1000000000000000000".to_owned(),
                router: Some("source".to_owned()),
            },
            outbound_bridge: outbound,
            target_swap: ProviderQuote {
                input_address: peer.base_mint.clone(),
                output_address: peer.quote_mint.clone(),
                input_amount_raw: "995000000000000000".to_owned(),
                output_amount_raw: "101300000".to_owned(),
                router: Some("target".to_owned()),
            },
            return_bridge: returned,
            observed_at_ms: 1_000,
            request_latency_ms: 100,
        };
        let valuation = super::super::usd_valuation::fixture("USDC", 1.0, 1_000);
        let snapshot = project(&source, &peer, &quotes, None, Some(&valuation), 1_100);

        assert_eq!(snapshot.quality, OnchainCrossChainQuality::Fresh);
        assert!((snapshot.gross_return_bps.unwrap_or_default() - 130.0).abs() < 0.001);
        assert!((snapshot.gas_usd.unwrap_or_default() - 0.30).abs() < 0.001);
        assert!((snapshot.net_return_bps.unwrap_or_default() - 39.849246).abs() < 0.001);
        assert_eq!(snapshot.stablecoin_risk_bps, 50);
        assert_eq!(snapshot.legs.len(), 4);
        assert!(snapshot.preview_ready);
        assert!(!snapshot.submit_ready);
        assert!(!snapshot.atomic);

        let missing = project(&source, &peer, &quotes, None, None, 1_100);
        assert_eq!(missing.quality, OnchainCrossChainQuality::EvidencePending);
        assert!(missing.net_return_bps.is_none());
        assert_eq!(missing.legs.len(), 4);
        assert!(missing.problem.unwrap().contains("USDC/USD"));
        let depegged = super::super::usd_valuation::fixture("USDC", 0.25, 1_000);
        let low_rate = project(&source, &peer, &quotes, None, Some(&depegged), 1_100);
        assert_eq!(low_rate.quality, OnchainCrossChainQuality::NoNetProfit);
        assert!(low_rate.net_return_bps.unwrap() < 0.0);
        assert_eq!(low_rate.quote_usd_valuation, Some(depegged));
        let mut larger_config = source.clone();
        larger_config.quote_amount_raw = "200000000".to_owned();
        let actual_size = project(
            &larger_config,
            &peer,
            &quotes,
            None,
            Some(&valuation),
            1_100,
        );
        assert_eq!(actual_size.net_return_bps, snapshot.net_return_bps);
        let mut unknown_gas = quotes.clone();
        unknown_gas.outbound_bridge.gas_usd = None;
        assert!(
            project(&source, &peer, &unknown_gas, None, Some(&valuation), 1_100)
                .net_return_bps
                .is_none()
        );
        let mut invalid_gas = source.clone();
        invalid_gas.gas_usd = f64::NAN;
        assert!(
            project(&invalid_gas, &peer, &quotes, None, Some(&valuation), 1_100)
                .net_return_bps
                .is_none()
        );
    }

    #[test]
    fn nonstable_source_quote_does_not_invent_gas_conversion() {
        let source = config("base", "WETH", 6);
        let peer = config("arbitrum", "WETH", 6);
        let outbound = bridge(&source, &peer, "1", "1");
        let returned = bridge(&peer, &source, "101000000", "100800000");
        let quotes = OnchainCrossChainQuoteSet {
            peer_item_id: "peer".to_owned(),
            source_chain: source.chain.clone(),
            source_provider: source.provider.clone(),
            source_base_address: source.base_mint.clone(),
            source_quote_address: source.quote_mint.clone(),
            source_wallet_address: source.wallet_address.clone(),
            peer_chain: peer.chain.clone(),
            peer_provider: peer.provider.clone(),
            peer_base_address: peer.base_mint.clone(),
            peer_quote_address: peer.quote_mint.clone(),
            peer_wallet_address: peer.wallet_address.clone(),
            source_swap: ProviderQuote {
                input_address: source.quote_mint.clone(),
                output_address: source.base_mint.clone(),
                input_amount_raw: "100000000".to_owned(),
                output_amount_raw: "1".to_owned(),
                router: None,
            },
            outbound_bridge: outbound,
            target_swap: ProviderQuote {
                input_address: peer.base_mint.clone(),
                output_address: peer.quote_mint.clone(),
                input_amount_raw: "1".to_owned(),
                output_amount_raw: "101000000".to_owned(),
                router: None,
            },
            return_bridge: returned,
            observed_at_ms: 1_000,
            request_latency_ms: 100,
        };
        let snapshot = project(&source, &peer, &quotes, None, None, 1_100);

        assert_eq!(snapshot.quality, OnchainCrossChainQuality::EvidencePending);
        assert!(snapshot.net_return_bps.is_none());
        let valuation = super::super::usd_valuation::fixture("WETH", 2_000.0, 1_000);
        let valued = project(&source, &peer, &quotes, None, Some(&valuation), 1_100);
        assert!(valued.net_return_bps.is_some());
        assert!(valued.quote_usd_valuation.is_some());
        let mut stale = valuation;
        stale.observed_at_ms = -source.max_age_ms;
        let stale = project(&source, &peer, &quotes, None, Some(&stale), 1_100);
        assert_eq!(stale.quality, OnchainCrossChainQuality::EvidencePending);
    }

    #[test]
    fn preview_locks_the_four_legs_and_waits_for_execution_contracts() {
        let snapshot = preview_snapshot();
        let request = OnchainCrossChainBuildRequest {
            approval_run_ids: Vec::new(),
            replenishment_run_ids: Vec::new(),
            expected_quote_observed_at_ms: 1_000,
        };
        let first = build_preview_from_snapshot(&snapshot, &request, 1_100, 10_000, None)
            .expect("fresh four-leg preview");
        let replay = build_preview_from_snapshot(&snapshot, &request, 1_200, 10_000, None)
            .expect("same evidence produces the same preview identity");

        assert_eq!(first.build_id, replay.build_id);
        assert_eq!(first.legs.len(), 4);
        assert!(first.preview_ready);
        assert!(!first.monitor_only);
        assert!(!first.atomic);
        assert!(!first.submit_ready);
        assert!(first.swap_executions.is_empty());
        assert!(first
            .warnings
            .iter()
            .any(|row| row.contains("无法原子成交")));
    }

    #[test]
    fn preview_binds_valuation_and_expires_with_the_earliest_evidence() {
        let mut snapshot = preview_snapshot();
        let request = OnchainCrossChainBuildRequest {
            approval_run_ids: Vec::new(),
            replenishment_run_ids: Vec::new(),
            expected_quote_observed_at_ms: 1_000,
        };
        let original = build_preview_from_snapshot(&snapshot, &request, 1_100, 500, None).unwrap();
        assert_eq!(original.valid_until_ms, 1_500);
        snapshot
            .cross_chain
            .quote_usd_valuation
            .as_mut()
            .unwrap()
            .observed_at_ms = 900;
        snapshot
            .cross_chain
            .quote_usd_valuation
            .as_mut()
            .unwrap()
            .usd_bid = 0.9;
        let updated = build_preview_from_snapshot(&snapshot, &request, 1_100, 500, None).unwrap();
        assert_eq!(updated.valid_until_ms, 1_400);
        assert_ne!(original.build_id, updated.build_id);
        assert_eq!(updated.quote_usd_valuation.as_ref().unwrap().usd_bid, 0.9);
        assert!(build_preview_from_snapshot(&snapshot, &request, 1_401, 500, None).is_err());
        snapshot.cross_chain.quote_usd_valuation = None;
        let missing = build_preview_from_snapshot(&snapshot, &request, 1_100, 500, None).unwrap();
        assert!(missing
            .blockers
            .iter()
            .any(|problem| problem.contains("USDC/USD")));
        assert!(!missing.submit_ready);
    }

    #[test]
    fn preview_keeps_bridge_transactions_out_of_snapshots_and_honors_provider_expiry() {
        let snapshot = preview_snapshot();
        let source = snapshot.config.clone();
        let peer = config("arbitrum", "USDT", 6);
        let mut outbound = bridge(&source, &peer, "100000000", "100500000");
        outbound.route_id = "route-1".to_owned();
        outbound.from_token = snapshot.cross_chain.legs[1].from_token.clone();
        outbound.to_token = snapshot.cross_chain.legs[1].to_token.clone();
        outbound.valid_until_ms = 5_000;
        let mut returned = bridge(&peer, &source, "100000000", "100500000");
        returned.route_id = "route-3".to_owned();
        returned.from_token = snapshot.cross_chain.legs[3].from_token.clone();
        returned.to_token = snapshot.cross_chain.legs[3].to_token.clone();
        returned.valid_until_ms = 6_000;
        let quotes = OnchainCrossChainQuoteSet {
            peer_item_id: "peer".to_owned(),
            source_chain: source.chain.clone(),
            source_provider: source.provider.clone(),
            source_base_address: source.base_mint.clone(),
            source_quote_address: source.quote_mint.clone(),
            source_wallet_address: source.wallet_address.clone(),
            peer_chain: peer.chain.clone(),
            peer_provider: peer.provider.clone(),
            peer_base_address: peer.base_mint.clone(),
            peer_quote_address: peer.quote_mint.clone(),
            peer_wallet_address: peer.wallet_address.clone(),
            source_swap: ProviderQuote {
                input_address: source.quote_mint.clone(),
                output_address: source.base_mint.clone(),
                input_amount_raw: "100000000".to_owned(),
                output_amount_raw: "100000000".to_owned(),
                router: Some("route-0".to_owned()),
            },
            outbound_bridge: outbound,
            target_swap: ProviderQuote {
                input_address: peer.base_mint.clone(),
                output_address: peer.quote_mint.clone(),
                input_amount_raw: "100500000".to_owned(),
                output_amount_raw: "100000000".to_owned(),
                router: Some("route-2".to_owned()),
            },
            return_bridge: returned,
            observed_at_ms: 1_000,
            request_latency_ms: 100,
        };
        let build = build_preview_from_snapshot(
            &snapshot,
            &OnchainCrossChainBuildRequest {
                approval_run_ids: Vec::new(),
                replenishment_run_ids: Vec::new(),
                expected_quote_observed_at_ms: 1_000,
            },
            1_100,
            10_000,
            Some(&quotes),
        )
        .expect("bridge contracts");

        assert_eq!(build.bridge_executions.len(), 2);
        assert_eq!(build.valid_until_ms, 5_000);
        assert_eq!(build.bridge_executions[0].rebuild_after_position, 1);
        assert_eq!(build.bridge_executions[1].rebuild_after_position, 3);
        assert!(build
            .warnings
            .iter()
            .any(|row| row.contains("第 2 腿已取得 LI.FI 可签交易")));
    }

    #[test]
    fn preview_rejects_changed_or_expired_bridge_evidence() {
        let snapshot = preview_snapshot();
        assert!(build_preview_from_snapshot(
            &snapshot,
            &OnchainCrossChainBuildRequest {
                approval_run_ids: Vec::new(),
                replenishment_run_ids: Vec::new(),
                expected_quote_observed_at_ms: 999,
            },
            1_100,
            10_000,
            None,
        )
        .is_err());
        assert!(build_preview_from_snapshot(
            &snapshot,
            &OnchainCrossChainBuildRequest {
                approval_run_ids: Vec::new(),
                replenishment_run_ids: Vec::new(),
                expected_quote_observed_at_ms: 1_000,
            },
            11_001,
            10_000,
            None,
        )
        .is_err());
    }

    fn preview_snapshot() -> OnchainComparisonSnapshot {
        let mut source = config("base", "USDC", 6);
        source.cross_chain = shared_types::OnchainCrossChainConfig {
            enabled: true,
            peer_item_id: "peer".to_owned(),
            provider: "lifi".to_owned(),
            stablecoin_risk_bps: 50,
        };
        let legs = [
            OnchainCrossChainLegKind::SourceSwap,
            OnchainCrossChainLegKind::OutboundBridge,
            OnchainCrossChainLegKind::TargetSwap,
            OnchainCrossChainLegKind::ReturnBridge,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| OnchainCrossChainLeg {
            position: (index + 1) as u8,
            kind,
            provider: if matches!(
                kind,
                OnchainCrossChainLegKind::OutboundBridge | OnchainCrossChainLegKind::ReturnBridge
            ) {
                "lifi".to_owned()
            } else {
                "zeroex_swap_v2".to_owned()
            },
            from_chain: if index < 2 { "base" } else { "arbitrum" }.to_owned(),
            to_chain: if index == 0 || index == 3 {
                "base"
            } else {
                "arbitrum"
            }
            .to_owned(),
            from_asset: if index == 0 || index == 3 {
                "USDC"
            } else {
                "PUPS"
            }
            .to_owned(),
            to_asset: if index < 2 { "PUPS" } else { "USDC" }.to_owned(),
            from_token: ["source-usdc", "source-pups", "peer-pups", "peer-usdc"][index].to_owned(),
            to_token: ["source-pups", "peer-pups", "peer-usdc", "source-usdc"][index].to_owned(),
            input_amount_raw: "100000000".to_owned(),
            expected_output_amount_raw: "101000000".to_owned(),
            minimum_output_amount_raw: Some("100500000".to_owned()),
            input_decimals: 6,
            output_decimals: 6,
            fee_usd: Some(0.1),
            gas_usd: Some(0.1),
            estimated_duration_seconds: Some(10),
            route_id: Some(format!("route-{index}")),
            route_tools: vec!["test".to_owned()],
            official_docs_url: super::super::lifi::LIFI_QUOTE_DOCS.to_owned(),
            observed_at_ms: 1_000,
        })
        .collect();
        OnchainComparisonSnapshot {
            config: source,
            cross_chain: OnchainCrossChainSnapshot {
                provider: "lifi".to_owned(),
                peer_item_id: "peer".to_owned(),
                peer_chain: Some("arbitrum".to_owned()),
                quality: OnchainCrossChainQuality::Fresh,
                legs,
                initial_quote_amount_raw: Some("100000000".to_owned()),
                final_quote_amount_raw: Some("101000000".to_owned()),
                gross_return_bps: Some(100.0),
                execution_buffer_bps: Some(10.0),
                stablecoin_risk_bps: 50,
                bridge_fee_usd: Some(0.2),
                gas_usd: Some(0.4),
                quote_usd_valuation: Some(super::super::usd_valuation::fixture("USDC", 1.0, 1_000)),
                total_cost_bps: Some(50.0),
                net_return_bps: Some(50.0),
                estimated_duration_seconds: Some(20),
                inventory: Vec::new(),
                atomic: false,
                preview_ready: true,
                submit_ready: false,
                problem: None,
                quote_observed_at_ms: Some(1_000),
                quote_latency_ms: Some(120),
                observed_at_ms: 1_100,
            },
            ..Default::default()
        }
    }
}
