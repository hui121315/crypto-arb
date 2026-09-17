use crate::state::AppState;
use axum::http::StatusCode;
use common::AppError;
use onchain_monitor::ProviderQuote;
use serde_json::json;
use shared_types::{
    plan_leg_sizing, plan_leg_sizing_for_base_quantity, FeeProduct, OnchainCexOrderPlan,
    OnchainComparisonDirection, OnchainComparisonQuality, OnchainExecutionBuildRequest,
    OnchainExecutionBuildResponse, OrderBookInfo, OrderIntent, OrderSide, OrderSource, OrderType,
    StrategyKind, TimeInForce, VenueInstrument,
};

mod providers;
mod quote_conversion_order;

const BUILD_VALIDITY_MS: i64 = 5_000;
const EXECUTION_BOOK_DEPTH: u32 = 100;

struct BuildResponseParts {
    build_id: String,
    direction: OnchainComparisonDirection,
    chain: FirmChainContract,
    cex_order: OnchainCexOrderPlan,
    quote_conversion_order: Option<shared_types::OnchainQuoteConversionOrderPlan>,
    economics: ExecutableEconomics,
    quote_usd_valuation: shared_types::OnchainUsdValuation,
    replenishment_costs: Vec<shared_types::OnchainExecutionReplenishmentCost>,
    approval_costs: Vec<shared_types::OnchainExecutionApprovalCost>,
    quote_observed_at_ms: i64,
    cex_observed_at_ms: i64,
}

pub(super) struct FirmChainContract {
    pub(super) quote: ProviderQuote,
    pub(super) minimum_output_amount_raw: String,
    pub(super) transaction: shared_types::OnchainUnsignedTransaction,
    pub(super) official_docs_url: String,
    pub(super) quote_observed_at_ms: i64,
    pub(super) valid_until_ms: i64,
}

pub(crate) async fn build(
    state: &AppState,
    request: &OnchainExecutionBuildRequest,
) -> Result<OnchainExecutionBuildResponse, AppError> {
    let snapshot = state.onchain_monitor().snapshot();
    validate_snapshot(&snapshot, request)?;
    let config = snapshot.config.clone();
    let replenishment_costs = super::replenishment_allocation::resolve(
        state,
        &config,
        request.direction,
        &request.replenishment_run_ids,
        common::time::now_ms(),
    )
    .map_err(|problem| conflict("ONCHAIN_REPLENISHMENT_COST_UNPROVEN", problem))?;
    let build_id = format!("onchain-build-{}", uuid::Uuid::new_v4());
    let instrument = resolve_cex_instrument(state, &config)?;
    let input_amount_raw = monitored_input_amount(&snapshot, request.direction)?;
    let chain =
        build_firm_chain_contract(state, &config, request.direction, &input_amount_raw).await?;
    let quote_observed_at_ms = chain.quote_observed_at_ms;
    let cex = state
        .market_data()
        .refresh_spot_orderbook_from_ws(
            state.aggregator(),
            &config.cex_venue,
            &config.cex_symbol,
            EXECUTION_BOOK_DEPTH,
            common::time::now_ms(),
        )
        .await;
    let now_ms = common::time::now_ms();
    let book = cex.value.as_ref().ok_or_else(|| {
        conflict(
            "ONCHAIN_CEX_DEPTH_MISSING",
            format!(
                "构建时没有取得新鲜 CEX Spot 完整盘口：{}",
                cex.last_error
                    .as_deref()
                    .unwrap_or("官方 WS 深度尚未返回首帧")
            ),
        )
    })?;
    let cex_observed_at_ms = cex
        .freshness_ms
        .filter(|age| *age <= config.max_age_ms)
        .map(|age| now_ms.saturating_sub(age.max(0)))
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CEX_DEPTH_STALE",
                "CEX Spot 盘口已过期，已拒绝使用旧深度构建订单",
            )
        })?;
    let client_order_id = format!("xl{}", &uuid::Uuid::new_v4().simple().to_string()[..18]);
    let cex_order = cex_order_plan(
        &config,
        request.direction,
        &chain.quote,
        &instrument,
        book,
        client_order_id,
    )
    .map_err(|problem| conflict("ONCHAIN_CEX_ORDER_BUILD_REJECTED", problem))?;
    validate_execution_target(state, &config, &cex_order)?;
    verify_cex_preflight(state, &config, &cex_order, now_ms).await?;
    let (quote_conversion_order, conversion_observed_at_ms) = build_quote_conversion_order(
        state,
        &config,
        request.direction,
        snapshot.quote_conversion.as_ref(),
        &cex_order,
    )
    .await?;
    let cex_observed_at_ms = conversion_observed_at_ms.map_or(cex_observed_at_ms, |observed| {
        cex_observed_at_ms.min(observed)
    });
    let valuation = super::usd_valuation::quote_evidence(state, &config, common::time::now_ms())
        .map_err(|problem| conflict("ONCHAIN_USD_VALUATION_MISSING", problem))?;
    let cex_observed_at_ms = cex_observed_at_ms.min(valuation.observed_at_ms);
    let mut economics = executable_economics(
        &config,
        request.direction,
        &chain.quote,
        &cex_order,
        quote_conversion_order.as_ref(),
        valuation.usd_bid,
    )
    .map_err(|problem| conflict("ONCHAIN_NET_PROFIT_RECHECK_FAILED", problem))?;
    let approval_costs = super::approval_allocation::resolve(
        state, &config, request.direction, &chain.transaction,
        &request.approval_run_ids, common::time::now_ms(),
    ).map_err(|problem| conflict("ONCHAIN_APPROVAL_COST_UNPROVEN", problem))?;
    let approval_cost = super::approval_allocation::total_usd(&approval_costs)
        .map_err(|problem| conflict("ONCHAIN_APPROVAL_COST_UNPROVEN", problem))?;
    let cost = super::replenishment_allocation::total_usd(&replenishment_costs)
        .map_err(|problem| conflict("ONCHAIN_REPLENISHMENT_COST_UNPROVEN", problem))?;
    economics.net_profit_usd -= cost + approval_cost;
    economics.net_spread_bps = economics.net_profit_usd / economics.capital_usd * 10_000.0;
    validate_executable_profit(&config, &economics)?;
    finalize_build_response(
        state,
        config,
        BuildResponseParts {
            build_id,
            direction: request.direction,
            chain,
            cex_order,
            quote_conversion_order,
            economics,
            quote_usd_valuation: valuation,
            replenishment_costs,
            approval_costs,
            quote_observed_at_ms,
            cex_observed_at_ms,
        },
    )
}

pub(super) async fn build_firm_chain_contract(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    input_amount_raw: &str,
) -> Result<FirmChainContract, AppError> {
    ensure_okx_allowance(state, config, direction, input_amount_raw).await?;
    let providers::FirmChainBuild {
        quote,
        minimum_output_amount_raw,
        transaction,
        official_docs_url,
    } = providers::build(config, direction, input_amount_raw)
        .await
        .map_err(firm_build_error)?;
    let quote_observed_at_ms = common::time::now_ms();
    let valid_until_ms = build_valid_until(&transaction, quote_observed_at_ms, config.max_age_ms);
    if valid_until_ms <= quote_observed_at_ms {
        return Err(conflict(
            "ONCHAIN_FIRM_BUILD_EXPIRED",
            "firm quote 在构建完成前已经过期，请重新构建",
        ));
    }
    Ok(FirmChainContract {
        quote,
        minimum_output_amount_raw,
        transaction,
        official_docs_url,
        quote_observed_at_ms,
        valid_until_ms,
    })
}

pub(super) fn conservative_minimum_output(
    output_amount_raw: &str,
    slippage_bps: f64,
) -> Result<String, String> {
    if !slippage_bps.is_finite() || !(0.0..=10_000.0).contains(&slippage_bps) {
        return Err("slippage must be between 0 and 10000 basis points".to_owned());
    }
    providers::minimum_output_from_bps(output_amount_raw, slippage_bps.floor() as u32)
}

async fn ensure_okx_allowance(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    input_amount_raw: &str,
) -> Result<(), AppError> {
    if config.provider != "okx_dex_v6" {
        return Ok(());
    }
    let input_token = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => &config.quote_mint,
        OnchainComparisonDirection::BuyCexSellOnchain => &config.base_mint,
    };
    if input_token.eq_ignore_ascii_case(shared_types::EVM_NATIVE_TOKEN_ADDRESS) {
        return Ok(());
    }
    let plan = super::allowance::build_plan(state, config, direction, input_amount_raw)
        .await
        .map_err(|problem| conflict("ONCHAIN_TOKEN_APPROVAL_CHECK_FAILED", problem))?;
    if plan.transactions.is_empty() {
        return Ok(());
    }
    Err(token_approval_required(
        &plan.token_address,
        &plan.spender,
        &plan.required_amount_raw,
        &plan.current_allowance_raw,
        &plan.official_docs_url,
    ))
}

fn firm_build_error(error: providers::FirmBuildError) -> AppError {
    match error {
        providers::FirmBuildError::TokenApprovalRequired(requirement) => token_approval_required(
            &requirement.token_address,
            &requirement.spender,
            &requirement.required_amount_raw,
            &requirement.current_allowance_raw,
            "https://docs.0x.org/docs/core-concepts/contracts",
        ),
        providers::FirmBuildError::Rejected(problem) => {
            conflict("ONCHAIN_FIRM_BUILD_REJECTED", problem)
        }
    }
}

fn token_approval_required(
    token_address: &str,
    spender: &str,
    required_amount_raw: &str,
    current_allowance_raw: &str,
    official_docs_url: &str,
) -> AppError {
    AppError::domain(
        StatusCode::CONFLICT,
        "ONCHAIN_TOKEN_APPROVAL_REQUIRED",
        "输入代币授权不足；请先完成独立 ERC-20 授权，再重新构建双腿交易计划",
    )
    .with_details(json!({
        "tokenAddress": token_address,
        "spender": spender,
        "requiredAmountRaw": required_amount_raw,
        "currentAllowanceRaw": current_allowance_raw,
        "officialDocsUrl": official_docs_url,
    }))
}

fn finalize_build_response(
    state: &AppState,
    config: shared_types::OnchainComparisonConfig,
    parts: BuildResponseParts,
) -> Result<OnchainExecutionBuildResponse, AppError> {
    let signer_problem = super::execution_submit::readiness(state, &config).err();
    if let Some(plan) = parts.quote_conversion_order.as_ref() {
        super::three_leg_execution::program(parts.direction, plan.sequence)
            .map_err(|problem| conflict("ONCHAIN_THREE_LEG_SEQUENCE_INVALID", problem))?;
    }
    let submit_ready = signer_problem.is_none();
    let blockers = signer_problem.into_iter().collect();
    let built_at_ms = common::time::now_ms();
    let valid_until_ms = build_valid_until(
        &parts.chain.transaction,
        parts.quote_observed_at_ms,
        config.max_age_ms,
    );
    if valid_until_ms <= built_at_ms {
        return Err(conflict(
            "ONCHAIN_FIRM_BUILD_EXPIRED",
            "firm quote 在 CEX 深度与远程预检完成前已经过期，请重新构建",
        ));
    }
    let FirmChainContract {
        quote,
        minimum_output_amount_raw,
        transaction,
        official_docs_url,
        ..
    } = parts.chain;
    let response = OnchainExecutionBuildResponse {
        build_id: parts.build_id,
        direction: parts.direction,
        provider: config.provider.clone(),
        chain: config.chain.clone(),
        wallet_address: config.wallet_address.clone(),
        input_token: quote.input_address,
        output_token: quote.output_address,
        input_amount_raw: quote.input_amount_raw,
        output_amount_raw: quote.output_amount_raw,
        settlement_assets: Some(settlement_assets(&config, parts.direction)),
        minimum_output_amount_raw: Some(minimum_output_amount_raw),
        chain_transaction: transaction,
        chain_input_adjustment: None,
        cex_order: parts.cex_order,
        quote_conversion_order: parts.quote_conversion_order,
        quote_usd_valuation: Some(parts.quote_usd_valuation),
        replenishment_costs: parts.replenishment_costs,
        approval_costs: parts.approval_costs,
        estimated_net_profit_usd: parts.economics.net_profit_usd,
        estimated_net_spread_bps: parts.economics.net_spread_bps,
        quote_observed_at_ms: parts.quote_observed_at_ms,
        cex_observed_at_ms: parts.cex_observed_at_ms,
        built_at_ms,
        valid_until_ms,
        official_docs_url,
        build_ready: true,
        submit_ready,
        blockers,
    };
    state
        .onchain_execution_builds()
        .insert(response.clone(), config, built_at_ms);
    Ok(response)
}

fn settlement_assets(
    config: &shared_types::OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
) -> shared_types::OnchainSwapAssets {
    let base = shared_types::OnchainExecutionToken {
        symbol: config.base_token.clone(),
        address: config.base_mint.clone(),
        decimals: config.base_decimals,
    };
    let quote = shared_types::OnchainExecutionToken {
        symbol: config.quote_token.clone(),
        address: config.quote_mint.clone(),
        decimals: config.quote_decimals,
    };
    match direction {
        OnchainComparisonDirection::BuyCexSellOnchain => shared_types::OnchainSwapAssets {
            input: base,
            output: quote,
        },
        OnchainComparisonDirection::BuyOnchainSellCex => shared_types::OnchainSwapAssets {
            input: quote,
            output: base,
        },
    }
}

fn resolve_cex_instrument(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
) -> Result<VenueInstrument, AppError> {
    state
        .instrument_registry()
        .resolve_hedge_instrument_for_product(
            &config.cex_venue,
            &config.cex_symbol,
            FeeProduct::Spot,
        )
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CEX_INSTRUMENT_STALE",
                "CEX Spot instrument 规格已失效，请等待官方 registry 刷新",
            )
        })
}

async fn build_quote_conversion_order(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    evidence: Option<&shared_types::OnchainQuoteConversionEvidence>,
    primary_order: &OnchainCexOrderPlan,
) -> Result<
    (
        Option<shared_types::OnchainQuoteConversionOrderPlan>,
        Option<i64>,
    ),
    AppError,
> {
    let Some(evidence) = evidence else {
        return Ok((None, None));
    };
    let instrument = state
        .instrument_registry()
        .resolve_hedge_instrument_for_product(&config.cex_venue, &evidence.symbol, FeeProduct::Spot)
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_QUOTE_CONVERSION_INSTRUMENT_STALE",
                format!(
                    "{} {} 换算交易对的官方规格已失效",
                    config.cex_venue.to_uppercase(),
                    evidence.symbol
                ),
            )
        })?;
    let read = state
        .market_data()
        .refresh_spot_orderbook_from_ws(
            state.aggregator(),
            &config.cex_venue,
            &evidence.symbol,
            EXECUTION_BOOK_DEPTH,
            common::time::now_ms(),
        )
        .await;
    let now_ms = common::time::now_ms();
    let book = read.value.as_ref().ok_or_else(|| {
        conflict(
            "ONCHAIN_QUOTE_CONVERSION_DEPTH_MISSING",
            format!(
                "构建时没有取得 {} {} 新鲜完整盘口：{}",
                config.cex_venue.to_uppercase(),
                evidence.symbol,
                read.last_error
                    .as_deref()
                    .unwrap_or("官方 WS 深度尚未返回首帧")
            ),
        )
    })?;
    let observed_at_ms = read
        .freshness_ms
        .filter(|age| *age <= config.max_age_ms)
        .map(|age| now_ms.saturating_sub(age.max(0)))
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_QUOTE_CONVERSION_DEPTH_STALE",
                "Quote 换算完整盘口已过期，拒绝构建三腿计划",
            )
        })?;
    let client_order_id = format!("xl{}", &uuid::Uuid::new_v4().simple().to_string()[..18]);
    let plan = quote_conversion_order::plan(quote_conversion_order::PlanRequest {
        config,
        direction,
        evidence,
        instrument: &instrument,
        book,
        primary_quote_amount: primary_order.estimated_quote_amount,
        client_order_id,
    })
    .map_err(|problem| conflict("ONCHAIN_QUOTE_CONVERSION_BUILD_REJECTED", problem))?;
    validate_execution_target(state, config, &plan.order)?;
    verify_cex_preflight(state, config, &plan.order, now_ms).await?;
    Ok((Some(plan), Some(observed_at_ms)))
}

fn validate_executable_profit(
    config: &shared_types::OnchainComparisonConfig,
    economics: &ExecutableEconomics,
) -> Result<(), AppError> {
    let minimum_bps = config.spread_alert.min_net_spread_bps.max(0.0);
    if economics.net_profit_usd > 0.0 && economics.net_spread_bps >= minimum_bps {
        return Ok(());
    }
    Err(conflict(
        "ONCHAIN_NET_PROFIT_RECHECK_FAILED",
        format!(
            "firm quote 重算后净收益 ${:.4} / {:.4}% 未达到 {:.4}% 门槛",
            economics.net_profit_usd,
            economics.net_spread_bps / 100.0,
            minimum_bps / 100.0
        ),
    ))
}

fn validate_snapshot(
    snapshot: &shared_types::OnchainComparisonSnapshot,
    request: &OnchainExecutionBuildRequest,
) -> Result<(), AppError> {
    if !snapshot.config.enabled
        || !matches!(
            snapshot.quality,
            OnchainComparisonQuality::Fresh | OnchainComparisonQuality::LowLiquidity
        )
    {
        return Err(conflict(
            "ONCHAIN_OPPORTUNITY_NOT_FRESH",
            "当前链上/CEX 机会没有通过身份、收益与新鲜度门槛",
        ));
    }
    if snapshot.quote_observed_at_ms != Some(request.expected_quote_observed_at_ms)
        || snapshot.cex_observed_at_ms != Some(request.expected_cex_observed_at_ms)
    {
        return Err(conflict(
            "ONCHAIN_BUILD_SNAPSHOT_CHANGED",
            "报价已经变化，请用最新快照重新构建",
        ));
    }
    let direction = snapshot
        .execution_readiness
        .directions
        .iter()
        .find(|row| row.direction == request.direction)
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_EXECUTION_READINESS_MISSING",
                "当前方向没有执行准备度证据",
            )
        })?;
    if !direction.build_ready {
        return Err(conflict(
            "ONCHAIN_EXECUTION_NOT_BUILDABLE",
            direction
                .blockers
                .first()
                .cloned()
                .unwrap_or_else(|| "双边执行证据尚未通过".to_owned()),
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
            "ONCHAIN_DIRECTION_NOT_PROFITABLE",
            format!("所选方向没有达到最低费后净差 {:.4}%", minimum_bps / 100.0),
        ));
    }
    Ok(())
}

pub(super) fn monitored_input_amount(
    snapshot: &shared_types::OnchainComparisonSnapshot,
    direction: OnchainComparisonDirection,
) -> Result<String, AppError> {
    let observed_at_ms = snapshot.quote_observed_at_ms.ok_or_else(|| {
        conflict(
            "ONCHAIN_QUOTE_AMOUNT_MISSING",
            "当前快照没有链上询价数量，已拒绝用配置默认值替代",
        )
    })?;
    let (input_token, output_token) = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => (
            snapshot.config.quote_mint.as_str(),
            snapshot.config.base_mint.as_str(),
        ),
        OnchainComparisonDirection::BuyCexSellOnchain => (
            snapshot.config.base_mint.as_str(),
            snapshot.config.quote_mint.as_str(),
        ),
    };
    let evidence = snapshot.quote_evidence.iter().find(|row| {
        row.observed_at_ms == observed_at_ms
            && token_identity_matches(&snapshot.config.chain, &row.input_mint, input_token)
            && token_identity_matches(&snapshot.config.chain, &row.output_mint, output_token)
    });
    evidence
        .filter(|row| {
            row.input_amount_raw
                .parse::<u128>()
                .is_ok_and(|amount| amount > 0)
        })
        .map(|row| row.input_amount_raw.clone())
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_QUOTE_AMOUNT_MISSING",
                "当前方向缺少与监控快照一致的链上输入数量，请等待下一轮报价",
            )
        })
}

fn token_identity_matches(chain: &str, left: &str, right: &str) -> bool {
    if chain.eq_ignore_ascii_case("solana") {
        left == right
    } else {
        left.eq_ignore_ascii_case(right)
    }
}

fn cex_order_plan(
    config: &shared_types::OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    quote: &onchain_monitor::ProviderQuote,
    instrument: &VenueInstrument,
    book: &OrderBookInfo,
    client_order_id: String,
) -> Result<OnchainCexOrderPlan, String> {
    let (side, reference_price, raw_base, exact_quantity) = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => (
            OrderSide::Sell,
            book.best_bid()
                .ok_or_else(|| "CEX Spot bid 为空".to_owned())?,
            quote.output_amount_raw.as_str(),
            false,
        ),
        OnchainComparisonDirection::BuyCexSellOnchain => (
            OrderSide::Buy,
            book.best_ask()
                .ok_or_else(|| "CEX Spot ask 为空".to_owned())?,
            quote.input_amount_raw.as_str(),
            true,
        ),
    };
    let base_quantity = super::quote::raw_units(raw_base, config.base_decimals)
        .ok_or_else(|| "firm quote 的基础币数量非法".to_owned())?;
    let sizing = if exact_quantity {
        plan_leg_sizing_for_base_quantity(base_quantity, instrument, reference_price)
    } else {
        plan_leg_sizing(base_quantity * reference_price, instrument, reference_price)
    }
    .map_err(|block| format!("CEX Spot 数量无法按官方步长构建：{}", block.code()))?;
    let estimated_quote_amount = consume_book(
        match side {
            OrderSide::Buy => &book.asks,
            OrderSide::Sell => &book.bids,
        },
        sizing.rounded_base_qty,
        reference_price,
        config.slippage_bps,
        side,
    )?;
    Ok(OnchainCexOrderPlan {
        venue: config.cex_venue.clone(),
        native_symbol: instrument.native_symbol.clone(),
        client_order_id,
        side,
        base_quantity: sizing.rounded_base_qty,
        reference_price,
        estimated_quote_amount,
        instrument_spec: instrument.clone(),
        sizing_plan: sizing,
    })
}

fn validate_execution_target(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    plan: &OnchainCexOrderPlan,
) -> Result<(), AppError> {
    if let Some(problem) = execution_target_problem(
        cex_notional_usd(state, config, plan)?.0,
        config.min_liquidity_usd,
    ) {
        return Err(conflict("ONCHAIN_CEX_DEPTH_TARGET_UNMET", problem));
    }
    Ok(())
}

fn cex_notional_usd(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    plan: &OnchainCexOrderPlan,
) -> Result<(f64, f64), AppError> {
    let asset = plan
        .instrument_spec
        .quote_asset
        .as_deref()
        .ok_or_else(|| conflict("ONCHAIN_USD_VALUATION_MISSING", "CEX 官方规格缺少报价资产"))?;
    let valuation = super::usd_valuation::evidence(state, config, asset, common::time::now_ms())
        .map_err(|problem| conflict("ONCHAIN_USD_VALUATION_MISSING", problem))?;
    let lower = plan.estimated_quote_amount * valuation.usd_bid;
    let upper = plan.estimated_quote_amount * valuation.usd_ask;
    if !lower.is_finite() || lower <= 0.0 || !upper.is_finite() {
        return Err(conflict(
            "ONCHAIN_USD_VALUATION_INVALID",
            "CEX 美元名义金额无效",
        ));
    }
    Ok((lower, upper))
}

fn execution_target_problem(actual: f64, minimum: f64) -> Option<String> {
    let minimum = minimum.max(0.0);
    (!(actual.is_finite() && actual + 1e-9 >= minimum)).then(|| {
        format!(
            "已读取 100 档 CEX 盘口，但本次计划可执行名义 ${actual:.2} 低于目标 ${minimum:.2}；请增大链上询价金额或降低目标可成交额"
        )
    })
}

async fn verify_cex_preflight(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    plan: &OnchainCexOrderPlan,
    now_ms: i64,
) -> Result<(), AppError> {
    let risk = state.trading_service().risk_config();
    if !risk.live_trading_enabled {
        return Err(conflict(
            "ONCHAIN_LIVE_TRADING_DISABLED",
            "实盘总开关未开启，链上/CEX 计划禁止进入提交准备态",
        ));
    }
    if risk.kill_switch_active {
        return Err(conflict(
            "ONCHAIN_KILL_SWITCH_ACTIVE",
            "全局急停已启用，链上/CEX 计划禁止构建",
        ));
    }
    if !trading::exchange_allowed(&risk.allowed_exchanges, &plan.venue)
        || !trading::symbol_allowed(&risk.allowed_symbols, &plan.native_symbol)
    {
        return Err(conflict(
            "ONCHAIN_CEX_RISK_SCOPE_REJECTED",
            "CEX Spot 腿不在当前实盘交易所或标的白名单内",
        ));
    }
    let (_, notional_usd) = cex_notional_usd(state, config, plan)?;
    if notional_usd > risk.max_order_notional {
        return Err(conflict(
            "ONCHAIN_CEX_NOTIONAL_LIMIT",
            format!(
                "CEX Spot 名义 ${:.2} 超过单笔上限 ${:.2}",
                notional_usd, risk.max_order_notional
            ),
        ));
    }
    if state.trading_service().open_order_count() >= risk.max_open_orders {
        return Err(conflict(
            "ONCHAIN_CEX_OPEN_ORDER_LIMIT",
            "当前未决订单数量已达到风险上限",
        ));
    }
    let capability = state
        .trading_service()
        .exchange_capability_matrix_for_product(&plan.venue, FeeProduct::Spot)
        .map_err(|error| conflict("ONCHAIN_CEX_CAPABILITY_FAILED", error.to_string()))?;
    if capability.order(OrderType::Market).is_none() || !capability.finality.has_confirmed_path() {
        return Err(conflict(
            "ONCHAIN_CEX_CAPABILITY_MISSING",
            "CEX Spot 缺少 Market 下单或可确认终态路径",
        ));
    }
    let intent = cex_order_intent(config, plan, now_ms);
    let context = cex_submission_context(plan);
    state
        .trading_service()
        .preflight_order_with_context(&intent, &context)
        .await
        .map_err(|error| {
            conflict(
                "ONCHAIN_CEX_ORDER_PREFLIGHT_REJECTED",
                format!("CEX Spot 远程下单预检未通过：{error}"),
            )
        })
}

pub(super) fn cex_order_intent(
    config: &shared_types::OnchainComparisonConfig,
    plan: &OnchainCexOrderPlan,
    now_ms: i64,
) -> OrderIntent {
    let client_order_id_policy =
        exchange::client_order_id_policy(&plan.venue, &plan.client_order_id);
    OrderIntent {
        id: format!("order-{}", uuid::Uuid::new_v4()),
        source: OrderSource::Strategy,
        strategy: Some(StrategyKind::OnchainDepeg),
        mode: shared_types::ExecutionMode::Live,
        exchange: plan.venue.clone(),
        symbol: plan.native_symbol.clone(),
        side: plan.side,
        order_type: OrderType::Market,
        quantity: plan.sizing_plan.rounded_contracts,
        price: None,
        slippage_tolerance_bps: Some(config.slippage_bps),
        reduce_only: false,
        time_in_force: TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: plan.client_order_id.clone(),
        client_order_id_policy: Some(client_order_id_policy),
        created_at_ms: now_ms,
    }
}

pub(super) fn cex_submission_context(
    plan: &OnchainCexOrderPlan,
) -> shared_types::OrderSubmissionContext {
    shared_types::OrderSubmissionContext {
        product: FeeProduct::Spot,
        instrument_spec: Some(plan.instrument_spec.clone()),
        sizing_plan: Some(plan.sizing_plan),
        ..shared_types::OrderSubmissionContext::default()
    }
}

pub(super) struct RevalidatedExecutionPlans {
    pub(super) primary: OnchainCexOrderPlan,
    pub(super) quote_conversion: Option<shared_types::OnchainQuoteConversionOrderPlan>,
}

pub(super) async fn revalidate_for_submit(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    build: &OnchainExecutionBuildResponse,
) -> Result<RevalidatedExecutionPlans, AppError> {
    let requested_at_ms = common::time::now_ms();
    if build.valid_until_ms <= requested_at_ms {
        return Err(conflict(
            "ONCHAIN_BUILD_EXPIRED",
            "交易计划已过期，请重新构建",
        ));
    }
    let instrument = state
        .instrument_registry()
        .resolve_hedge_instrument_for_product(
            &config.cex_venue,
            &config.cex_symbol,
            FeeProduct::Spot,
        )
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_CEX_INSTRUMENT_STALE",
                "提交前 CEX Spot 官方规格已失效",
            )
        })?;
    let read = state
        .market_data()
        .refresh_spot_orderbook_from_ws(
            state.aggregator(),
            &config.cex_venue,
            &config.cex_symbol,
            EXECUTION_BOOK_DEPTH,
            requested_at_ms,
        )
        .await;
    let now_ms = common::time::now_ms();
    if build.valid_until_ms <= now_ms {
        return Err(conflict(
            "ONCHAIN_BUILD_EXPIRED",
            "读取最新 CEX 深度期间交易计划已过期，请重新构建",
        ));
    }
    let book = read.value.as_ref().ok_or_else(|| {
        conflict(
            "ONCHAIN_CEX_DEPTH_MISSING",
            format!(
                "提交前没有新鲜 CEX Spot 完整盘口：{}",
                read.last_error
                    .as_deref()
                    .unwrap_or("官方 WS 深度尚未返回首帧")
            ),
        )
    })?;
    if read.freshness_ms.is_none_or(|age| age > config.max_age_ms) {
        return Err(conflict(
            "ONCHAIN_CEX_DEPTH_STALE",
            "提交前 CEX Spot 盘口已过期",
        ));
    }
    let quote = onchain_monitor::ProviderQuote {
        input_address: build.input_token.clone(),
        output_address: build.output_token.clone(),
        input_amount_raw: build.input_amount_raw.clone(),
        output_amount_raw: build.output_amount_raw.clone(),
        router: None,
    };
    let plan = cex_order_plan(
        config,
        build.direction,
        &quote,
        &instrument,
        book,
        build.cex_order.client_order_id.clone(),
    )
    .map_err(|problem| conflict("ONCHAIN_CEX_ORDER_RECHECK_REJECTED", problem))?;
    validate_execution_target(state, config, &plan)?;
    verify_cex_preflight(state, config, &plan, now_ms).await?;
    let quote_conversion =
        revalidate_quote_conversion_for_submit(state, config, build, &plan, requested_at_ms)
            .await?;
    let valuation = super::usd_valuation::quote_evidence(state, config, common::time::now_ms())
        .map_err(|problem| conflict("ONCHAIN_USD_VALUATION_MISSING", problem))?;
    let economics = executable_economics(
        config,
        build.direction,
        &quote,
        &plan,
        quote_conversion.as_ref(),
        valuation.usd_bid,
    )
    .map_err(|problem| conflict("ONCHAIN_NET_PROFIT_RECHECK_FAILED", problem))?;
    validate_executable_profit(config, &economics).map_err(|_| {
        conflict(
            "ONCHAIN_NET_PROFIT_RECHECK_FAILED",
            "提交瞬间按主交易对和 Quote 换汇最新 WS 深度重算后已不再盈利",
        )
    })?;
    Ok(RevalidatedExecutionPlans {
        primary: plan,
        quote_conversion,
    })
}

async fn revalidate_quote_conversion_for_submit(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    build: &OnchainExecutionBuildResponse,
    primary: &OnchainCexOrderPlan,
    requested_at_ms: i64,
) -> Result<Option<shared_types::OnchainQuoteConversionOrderPlan>, AppError> {
    let Some(original) = build.quote_conversion_order.as_ref() else {
        return Ok(None);
    };
    let instrument = state
        .instrument_registry()
        .resolve_hedge_instrument_for_product(
            &original.order.venue,
            &original.order.instrument_spec.display_symbol,
            FeeProduct::Spot,
        )
        .ok_or_else(|| {
            conflict(
                "ONCHAIN_QUOTE_CONVERSION_INSTRUMENT_STALE",
                "提交前 Quote 换汇官方规格已失效",
            )
        })?;
    let read = state
        .market_data()
        .refresh_spot_orderbook_from_ws(
            state.aggregator(),
            &original.order.venue,
            &instrument.display_symbol,
            EXECUTION_BOOK_DEPTH,
            requested_at_ms,
        )
        .await;
    let now_ms = common::time::now_ms();
    if build.valid_until_ms <= now_ms {
        return Err(conflict(
            "ONCHAIN_BUILD_EXPIRED",
            "读取最新 Quote 换汇深度期间交易计划已过期，请重新构建",
        ));
    }
    let book = read.value.as_ref().ok_or_else(|| {
        conflict(
            "ONCHAIN_QUOTE_CONVERSION_DEPTH_MISSING",
            format!(
                "提交前没有取得 Quote 换汇最新 WS 盘口：{}",
                read.last_error.as_deref().unwrap_or("尚未返回首帧")
            ),
        )
    })?;
    if read.freshness_ms.is_none_or(|age| age > config.max_age_ms) {
        return Err(conflict(
            "ONCHAIN_QUOTE_CONVERSION_DEPTH_STALE",
            "提交前 Quote 换汇 WS 盘口已过期",
        ));
    }
    let replanned = quote_conversion_order::replan(
        config,
        build.direction,
        original,
        &instrument,
        book,
        primary.estimated_quote_amount,
    )
    .map_err(|problem| conflict("ONCHAIN_QUOTE_CONVERSION_RECHECK_REJECTED", problem))?;
    validate_execution_target(state, config, &replanned.order)?;
    verify_cex_preflight(state, config, &replanned.order, now_ms).await?;
    Ok(Some(replanned))
}

pub(super) fn quote_conversion_progress(
    original: &shared_types::OnchainQuoteConversionOrderPlan,
    filled_from_amount: f64,
    filled_to_amount: f64,
) -> Result<bool, String> {
    quote_conversion_order::progress_complete(original, filled_from_amount, filled_to_amount)
}

pub(super) async fn replan_quote_conversion_residual(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    build: &OnchainExecutionBuildResponse,
    original: &shared_types::OnchainQuoteConversionOrderPlan,
    remaining_input: f64,
    remaining_output: f64,
) -> Result<Option<shared_types::OnchainQuoteConversionOrderPlan>, String> {
    let requested_at_ms = common::time::now_ms();
    let instrument = state
        .instrument_registry()
        .resolve_hedge_instrument_for_product(
            &original.order.venue,
            &original.order.instrument_spec.display_symbol,
            FeeProduct::Spot,
        )
        .ok_or_else(|| "Quote 换汇重试时官方规格已失效".to_owned())?;
    let read = state
        .market_data()
        .refresh_spot_orderbook_from_ws(
            state.aggregator(),
            &original.order.venue,
            &instrument.display_symbol,
            EXECUTION_BOOK_DEPTH,
            requested_at_ms,
        )
        .await;
    let book = read.value.as_ref().ok_or_else(|| {
        format!(
            "Quote 换汇重试没有取得最新 WS 盘口：{}",
            read.last_error.as_deref().unwrap_or("尚未返回首帧")
        )
    })?;
    if read.freshness_ms.is_none_or(|age| age > config.max_age_ms) {
        return Err("Quote 换汇重试盘口已过期".to_owned());
    }
    let client_order_id = format!("xl{}", &uuid::Uuid::new_v4().simple().to_string()[..18]);
    // These limits already deduct actual fees and reserve the next order's fees.
    let mut remaining = original.clone();
    remaining.planned_from_amount = remaining_input;
    remaining.planned_to_amount = remaining_output;
    let replanned = quote_conversion_order::replan_after_fill(
        quote_conversion_order::ReplanAfterFillRequest {
            config,
            direction: build.direction,
            original: &remaining,
            instrument: &instrument,
            book,
            filled_from_amount: 0.0,
            filled_to_amount: 0.0,
            client_order_id,
        },
    )?;
    if let Some(plan) = replanned.as_ref() {
        verify_cex_preflight(state, config, &plan.order, common::time::now_ms())
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(replanned)
}

pub(super) fn consume_book(
    levels: &[[f64; 2]],
    quantity: f64,
    best_price: f64,
    slippage_bps: f64,
    side: OrderSide,
) -> Result<f64, String> {
    if !quantity.is_finite() || quantity <= 0.0 {
        return Err("CEX Spot 下单数量非法".to_owned());
    }
    let tolerance = (slippage_bps / 10_000.0).clamp(0.0, 1.0);
    let limit = match side {
        OrderSide::Buy => best_price * (1.0 + tolerance),
        OrderSide::Sell => best_price * (1.0 - tolerance),
    };
    let mut remaining = quantity;
    let mut quote_total = 0.0;
    for [price, available] in levels.iter().copied() {
        let valid = price.is_finite() && available.is_finite() && price > 0.0 && available > 0.0;
        let within_limit = match side {
            OrderSide::Buy => price <= limit,
            OrderSide::Sell => price >= limit,
        };
        if !valid || !within_limit {
            continue;
        }
        let take = remaining.min(available);
        quote_total += take * price;
        remaining -= take;
        if remaining <= quantity * 1e-9 {
            return Ok(quote_total);
        }
    }
    Err(format!(
        "CEX Spot 在 {:.4}% 价格保护内只有 {:.8}，不足 {:.8}",
        slippage_bps / 100.0,
        quantity - remaining,
        quantity
    ))
}

struct ExecutableEconomics {
    capital_usd: f64,
    net_profit_usd: f64,
    net_spread_bps: f64,
}

fn executable_economics(
    config: &shared_types::OnchainComparisonConfig,
    direction: OnchainComparisonDirection,
    quote: &onchain_monitor::ProviderQuote,
    cex_order: &OnchainCexOrderPlan,
    quote_conversion_order: Option<&shared_types::OnchainQuoteConversionOrderPlan>,
    usd_per_quote: f64,
) -> Result<ExecutableEconomics, String> {
    if !usd_per_quote.is_finite()
        || usd_per_quote <= 0.0
        || !config.cex_taker_fee_bps.is_finite()
        || !(0.0..10_000.0).contains(&config.cex_taker_fee_bps)
        || !config.slippage_bps.is_finite()
        || !(0.0..=10_000.0).contains(&config.slippage_bps)
        || !config.gas_usd.is_finite()
        || config.gas_usd < 0.0
        || !cex_order.estimated_quote_amount.is_finite()
        || cex_order.estimated_quote_amount <= 0.0
    {
        return Err("交易金额或费用配置无效，无法核算净收益".to_owned());
    }
    let chain_base = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            super::quote::raw_units(&quote.output_amount_raw, config.base_decimals)
        }
        OnchainComparisonDirection::BuyCexSellOnchain => {
            super::quote::raw_units(&quote.input_amount_raw, config.base_decimals)
        }
    }
    .ok_or_else(|| "firm quote 基础币数量非法".to_owned())?;
    if direction == OnchainComparisonDirection::BuyCexSellOnchain
        && (chain_base - cex_order.base_quantity).abs() > chain_base.max(1.0) * 1e-9
    {
        return Err("CEX 买入数量与链上卖出数量不一致".to_owned());
    }
    let chain_quote = match direction {
        OnchainComparisonDirection::BuyOnchainSellCex => {
            super::quote::raw_units(&quote.input_amount_raw, config.quote_decimals)
        }
        OnchainComparisonDirection::BuyCexSellOnchain => {
            super::quote::raw_units(&quote.output_amount_raw, config.quote_decimals)
        }
    }
    .ok_or_else(|| "firm quote 报价币数量非法".to_owned())?;
    let cex_fee = cex_order.estimated_quote_amount * config.cex_taker_fee_bps / 10_000.0;
    let conversion_fee_rate = config.cex_taker_fee_bps / 10_000.0;
    let conversion_rate = match quote_conversion_order {
        Some(conversion)
            if conversion.planned_from_amount.is_finite()
                && conversion.planned_from_amount > 0.0
                && conversion.planned_to_amount.is_finite()
                && conversion.planned_to_amount > 0.0 =>
        {
            match direction {
                OnchainComparisonDirection::BuyOnchainSellCex => {
                    conversion.planned_to_amount / conversion.planned_from_amount
                }
                OnchainComparisonDirection::BuyCexSellOnchain => {
                    conversion.planned_from_amount / conversion.planned_to_amount
                }
            }
        }
        Some(_) => return Err("Quote 换算计划金额无效，无法核算净收益".to_owned()),
        None => 1.0,
    };
    // Reserve and proceeds must use the same quote units after the explicit conversion.
    let movement_reserve =
        cex_order.estimated_quote_amount * conversion_rate * config.slippage_bps / 10_000.0;
    let (spent, received, explicit_cex_fee) = match (direction, quote_conversion_order) {
        (OnchainComparisonDirection::BuyOnchainSellCex, Some(conversion)) => {
            let net_primary_quote = cex_order.estimated_quote_amount - cex_fee;
            let retained_quote = net_primary_quote - conversion.planned_from_amount;
            if retained_quote < -f64::EPSILON * 64.0 * net_primary_quote.abs().max(1.0) {
                return Err("Quote 换汇计划超过主单扣费后预计收入".into());
            }
            // An unspent fee reserve is still an asset, valued at the explicit conversion rate.
            // Input-fee and output-fee payment produce the same total marked asset value.
            (
                chain_quote,
                conversion.planned_to_amount * (1.0 - conversion_fee_rate)
                    + retained_quote.max(0.0) * conversion_rate,
                0.0,
            )
        }
        (OnchainComparisonDirection::BuyCexSellOnchain, Some(conversion)) => {
            (conversion.planned_from_amount, chain_quote, 0.0)
        }
        (OnchainComparisonDirection::BuyOnchainSellCex, None) => {
            (chain_quote, cex_order.estimated_quote_amount, cex_fee)
        }
        (OnchainComparisonDirection::BuyCexSellOnchain, None) => {
            (cex_order.estimated_quote_amount, chain_quote, cex_fee)
        }
    };
    let net_profit_usd =
        (received - spent - explicit_cex_fee - movement_reserve) * usd_per_quote - config.gas_usd;
    let capital = spent * usd_per_quote;
    let net_spread_bps = net_profit_usd / capital * 10_000.0;
    if !net_profit_usd.is_finite() || !net_spread_bps.is_finite() {
        return Err("净收益计算溢出，已拒绝构建订单".to_owned());
    }
    Ok(ExecutableEconomics {
        capital_usd: capital,
        net_profit_usd,
        net_spread_bps,
    })
}

fn build_valid_until(
    transaction: &shared_types::OnchainUnsignedTransaction,
    quote_observed_at_ms: i64,
    max_age_ms: i64,
) -> i64 {
    let local_expiry =
        quote_observed_at_ms.saturating_add(BUILD_VALIDITY_MS.min(max_age_ms).max(1));
    match transaction {
        shared_types::OnchainUnsignedTransaction::SolanaVersioned {
            expire_at_ms: Some(provider_expiry),
            ..
        } if *provider_expiry > 0 => local_expiry.min(*provider_expiry),
        _ => local_expiry,
    }
}

fn conflict(code: &'static str, message: impl Into<String>) -> AppError {
    AppError::domain(StatusCode::CONFLICT, code, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_consumption_is_side_aware_and_fail_closed() {
        let asks = [[100.0, 0.5], [100.1, 0.5], [101.0, 10.0]];
        let bids = [[100.0, 0.5], [99.9, 0.5], [99.0, 10.0]];
        assert_eq!(
            consume_book(&asks, 1.0, 100.0, 20.0, OrderSide::Buy),
            Ok(100.05)
        );
        assert_eq!(
            consume_book(&bids, 1.0, 100.0, 20.0, OrderSide::Sell),
            Ok(99.95)
        );
        assert!(consume_book(&asks, 2.0, 100.0, 20.0, OrderSide::Buy).is_err());
    }

    #[test]
    fn full_depth_build_still_enforces_the_configured_execution_target() {
        assert_eq!(execution_target_problem(100.0, 100.0), None);
        let problem = execution_target_problem(99.99, 100.0).expect("below target");
        assert!(problem.contains("99.99"));
        assert!(problem.contains("100.00"));
        assert!(problem.contains("100 档"));
    }

    #[test]
    fn firm_build_reuses_the_exact_monitored_direction_amount() {
        let mut snapshot = shared_types::OnchainComparisonSnapshot::default();
        snapshot.config.chain = "solana".to_owned();
        snapshot.config.base_mint = "BaseMint".to_owned();
        snapshot.config.quote_mint = "QuoteMint".to_owned();
        snapshot.quote_observed_at_ms = Some(1_000);
        snapshot.quote_evidence = vec![
            quote_evidence("BaseMint", "QuoteMint", "975000000", 1_000),
            quote_evidence("QuoteMint", "BaseMint", "100000000", 1_000),
        ];

        assert_eq!(
            monitored_input_amount(&snapshot, OnchainComparisonDirection::BuyOnchainSellCex)
                .expect("reverse monitored amount"),
            "100000000"
        );
        assert_eq!(
            monitored_input_amount(&snapshot, OnchainComparisonDirection::BuyCexSellOnchain)
                .expect("forward monitored amount"),
            "975000000"
        );
    }

    #[test]
    fn firm_build_validity_starts_when_the_provider_quote_arrives() {
        let evm = shared_types::OnchainUnsignedTransaction::EvmCall {
            chain_id: 1,
            from: "0x1111111111111111111111111111111111111111".to_owned(),
            to: "0x2222222222222222222222222222222222222222".to_owned(),
            data: "0x1234".to_owned(),
            value: "0".to_owned(),
            gas: "100000".to_owned(),
            gas_price: Some("1".to_owned()),
            max_priority_fee_per_gas: None,
            allowance_spender: None,
        };
        assert_eq!(build_valid_until(&evm, 1_000, 3_000), 4_000);

        let solana = shared_types::OnchainUnsignedTransaction::SolanaVersioned {
            transaction_base64: "AQID".to_owned(),
            request_id: "request".to_owned(),
            router: "iris".to_owned(),
            mode: "ultra".to_owned(),
            last_valid_block_height: Some(10),
            expire_at_ms: Some(3_500),
        };
        assert_eq!(build_valid_until(&solana, 1_000, 5_000), 3_500);
    }

    fn quote_evidence(
        input_mint: &str,
        output_mint: &str,
        input_amount_raw: &str,
        observed_at_ms: i64,
    ) -> shared_types::OnchainQuoteEvidence {
        shared_types::OnchainQuoteEvidence {
            provider: "jupiter_swap_v2".to_owned(),
            endpoint: "https://api.jup.ag/swap/v2/order".to_owned(),
            official_docs_url: "https://developers.jup.ag/docs/swap/order-and-execute".to_owned(),
            input_mint: input_mint.to_owned(),
            output_mint: output_mint.to_owned(),
            input_amount_raw: input_amount_raw.to_owned(),
            output_amount_raw: "1".to_owned(),
            router: Some("iris".to_owned()),
            transaction_requested: false,
            observed_at_ms,
        }
    }
}
