use super::identity;
use super::provider_runtime::provider_runtime;
use super::provider_types::ProviderRuntime;
use super::quote::{quote_price, raw_units};
use crate::services::market_data::{MarketQuality, MarketRead, MarketSource};
use crate::state::AppState;
use onchain_monitor::{classify_quality, compare_quotes, OnchainQuotePair, QuoteInputs};
use shared_types::{
    onchain_cex_base_token, onchain_cex_pair_matches, onchain_cex_quote_token,
    onchain_quotes_match, OnchainBatchItemSnapshot, OnchainComparisonConfig,
    OnchainComparisonQuality, OnchainComparisonSnapshot, OnchainQuoteConversionEvidence,
    OnchainSpreadAlertMode,
};

const PROJECTION_INTERVAL_MS: i64 = 100;
const PROJECTION_HEARTBEAT_MS: i64 = 1_000;

pub(super) fn project_snapshot_with_conversion(
    config: OnchainComparisonConfig,
    quotes: &OnchainQuotePair,
    cex: &MarketRead<shared_types::OrderBookInfo>,
    conversion: Result<Option<OnchainQuoteConversionEvidence>, String>,
    valuation: Result<shared_types::OnchainUsdValuation, String>,
    now_ms: i64,
) -> OnchainComparisonSnapshot {
    let runtime = provider_runtime(&config.provider);
    let mut snapshot = base_snapshot(config, now_ms, &runtime, Some(quotes));
    if cex.value.is_none() {
        return cex_orderbook_unavailable(snapshot, cex);
    }
    let valuation = match valuation {
        Ok(value) => value,
        Err(problem) => {
            attach_cex_telemetry(&mut snapshot, cex, now_ms);
            return with_problem(
                snapshot,
                OnchainComparisonQuality::ValuationPending,
                &problem,
            );
        }
    };
    let Some(usd_rate) = super::usd_valuation::rate(
        Some(&valuation),
        &snapshot.config.quote_token,
        snapshot.config.max_age_ms,
        now_ms,
    ) else {
        attach_cex_telemetry(&mut snapshot, cex, now_ms);
        return with_problem(
            snapshot,
            OnchainComparisonQuality::ValuationPending,
            "Quote/USD 估值汇率已过期或资产不一致",
        );
    };
    snapshot.quote_usd_valuation = Some(valuation);
    let (conversion, conversion_problem) = match conversion {
        Ok(evidence) => (evidence, None),
        Err(problem) => (None, Some(problem)),
    };
    let comparisons =
        match comparison_rows(&snapshot.config, quotes, cex, conversion.as_ref(), usd_rate) {
            Ok(comparisons) => comparisons,
            Err(reason) => {
                return with_problem(snapshot, OnchainComparisonQuality::MappingInvalid, &reason);
            }
        };
    let cex_age_ms = cex.freshness_ms.unwrap_or(i64::MAX);
    let onchain_age_ms = snapshot.onchain_freshness_ms.unwrap_or(i64::MAX);
    let conversion_age_ms = conversion
        .as_ref()
        .map_or(0, |evidence| evidence.freshness_ms);
    let source_age_ms = cex_age_ms.max(onchain_age_ms).max(conversion_age_ms);
    let source_quality = classify_quality(
        source_age_ms,
        snapshot.config.max_age_ms,
        snapshot.config.min_liquidity_usd,
        snapshot.config.spread_alert.min_net_spread_bps,
        &comparisons,
    );
    let quality = if source_quality == OnchainComparisonQuality::Stale
        || onchain_cex_pair_matches(&snapshot.config)
    {
        source_quality
    } else if onchain_quotes_match(&snapshot.config) {
        OnchainComparisonQuality::RawCustomPair
    } else if conversion.is_some() && snapshot.config.base_identity_resolved {
        source_quality
    } else {
        OnchainComparisonQuality::RawCrossQuote
    };
    snapshot.quality = quality;
    snapshot.quote_conversion = conversion;
    attach_cex_telemetry(&mut snapshot, cex, now_ms);
    snapshot.degradation_reasons = quality_reason(
        quality,
        &comparisons,
        onchain_age_ms,
        cex_age_ms,
        &snapshot.config,
        conversion_problem.as_deref(),
    );
    snapshot.comparisons = comparisons;
    snapshot
}

#[cfg(test)]
pub(super) fn project_snapshot(
    config: OnchainComparisonConfig,
    quotes: &OnchainQuotePair,
    cex: &MarketRead<shared_types::OrderBookInfo>,
    now_ms: i64,
) -> OnchainComparisonSnapshot {
    let valuation = super::usd_valuation::fixture(&config.quote_token, 1.0, now_ms);
    project_snapshot_with_conversion(config, quotes, cex, Ok(None), Ok(valuation), now_ms)
}

pub(super) fn retain_last_official_cex_projection(
    current: &OnchainComparisonSnapshot,
    config: &OnchainComparisonConfig,
    cex: &MarketRead<shared_types::OrderBookInfo>,
    now_ms: i64,
) -> Option<OnchainComparisonSnapshot> {
    let transient_gap = cex.value.is_none()
        && matches!(
            cex.quality,
            MarketQuality::Warming
                | MarketQuality::Missing
                | MarketQuality::StaleAllowed
                | MarketQuality::RateLimited
                | MarketQuality::CircuitOpen
        );
    let prior_decision = matches!(
        current.quality,
        OnchainComparisonQuality::Fresh
            | OnchainComparisonQuality::RawCrossQuote
            | OnchainComparisonQuality::RawCustomPair
            | OnchainComparisonQuality::LowLiquidity
            | OnchainComparisonQuality::NoNetProfit
            | OnchainComparisonQuality::Stale
    );
    if !transient_gap
        || !prior_decision
        || &current.config != config
        || current.cex_source != "ws_push"
        || current.comparisons.is_empty()
    {
        return None;
    }

    let cex_observed_at_ms = current.cex_observed_at_ms?;
    let quote_observed_at_ms = current.quote_observed_at_ms?;
    let cex_age_ms = now_ms.saturating_sub(cex_observed_at_ms).max(0);
    let quote_age_ms = now_ms.saturating_sub(quote_observed_at_ms).max(0);
    if cex_age_ms > crate::services::market_data::cache::TICKER_STALE_MS
        || quote_age_ms > crate::services::market_data::cache::TICKER_STALE_MS
    {
        return None;
    }

    let degradation_reason = format!(
        "{} {} 最后一次官方 WS 最优价距今 {}ms，已超过 {}ms 时效门槛；仅保留上次结果用于观察，禁止 Webhook 与交易构建",
        config.cex_venue.to_uppercase(),
        config.cex_symbol,
        cex_age_ms,
        config.max_age_ms,
    );
    let market_blocker = format!(
        "{} {} 官方 WS 最优价已过期；等待新鲜行情后才能提醒、构建或提交",
        config.cex_venue.to_uppercase(),
        config.cex_symbol,
    );
    let mut next = current.clone();
    next.quality = OnchainComparisonQuality::Stale;
    next.onchain_freshness_ms = Some(quote_age_ms);
    next.cex_freshness_ms = Some(cex_age_ms);
    next.cex_problem = cex
        .last_error
        .clone()
        .or_else(|| Some("正在等待新的官方 CEX WS 最优价".to_owned()));
    next.cex_retry_after_ms = cex.retry_after_ms;
    next.degradation_reasons = vec![degradation_reason];
    next.observed_at_ms = now_ms;
    next.read_only = true;
    for comparison in &mut next.comparisons {
        comparison.executable = false;
    }
    if !next
        .execution_readiness
        .global_blockers
        .iter()
        .any(|problem| problem == &market_blocker)
    {
        next.execution_readiness
            .global_blockers
            .push(market_blocker.clone());
    }
    for direction in &mut next.execution_readiness.directions {
        direction.build_ready = false;
        direction.submit_ready = false;
        if !direction
            .blockers
            .iter()
            .any(|problem| problem == &market_blocker)
        {
            direction.blockers.push(market_blocker.clone());
        }
    }
    next.execution_readiness.observed_at_ms = now_ms;
    Some(next)
}

pub(super) fn attach_cex_telemetry(
    snapshot: &mut OnchainComparisonSnapshot,
    cex: &MarketRead<shared_types::OrderBookInfo>,
    now_ms: i64,
) {
    snapshot.cex_source = if cex.value.is_none() && cex.source == MarketSource::LocalCache {
        "ws_pending".to_owned()
    } else {
        cex.source.as_str().to_owned()
    };
    snapshot.cex_freshness_ms = cex.freshness_ms;
    snapshot.cex_observed_at_ms = cex
        .freshness_ms
        .map(|age_ms| now_ms.saturating_sub(age_ms.max(0)));
    snapshot.cex_problem.clone_from(&cex.last_error);
    snapshot.cex_retry_after_ms = cex.retry_after_ms;
}

pub(super) fn cex_unavailable_reason(
    config: &OnchainComparisonConfig,
    cex: &MarketRead<shared_types::OrderBookInfo>,
) -> String {
    let detail = cex
        .last_error
        .clone()
        .unwrap_or_else(|| "正在等待首个 CEX WS 最优买卖价帧".to_owned());
    let retry = cex
        .retry_after_ms
        .map(|delay| format!("；预计 {delay}ms 后重试"))
        .unwrap_or_default();
    format!(
        "{} {} 现货 WS 最优买卖价暂不可用：{detail}{retry}",
        config.cex_venue, config.cex_symbol
    )
}

fn cex_orderbook_unavailable(
    mut snapshot: OnchainComparisonSnapshot,
    cex: &MarketRead<shared_types::OrderBookInfo>,
) -> OnchainComparisonSnapshot {
    let observed_at_ms = snapshot.observed_at_ms;
    attach_cex_telemetry(&mut snapshot, cex, observed_at_ms);
    let quality = match cex.quality {
        MarketQuality::Fresh | MarketQuality::Warming | MarketQuality::Missing => {
            OnchainComparisonQuality::Pending
        }
        MarketQuality::Unsupported => OnchainComparisonQuality::MappingInvalid,
        MarketQuality::StaleAllowed | MarketQuality::RateLimited | MarketQuality::CircuitOpen => {
            OnchainComparisonQuality::UpstreamUnavailable
        }
    };
    let reason = cex_unavailable_reason(&snapshot.config, cex);
    with_problem(snapshot, quality, &reason)
}

fn comparison_rows(
    config: &OnchainComparisonConfig,
    quotes: &OnchainQuotePair,
    cex: &MarketRead<shared_types::OrderBookInfo>,
    conversion: Option<&OnchainQuoteConversionEvidence>,
    usd_rate: f64,
) -> Result<Vec<shared_types::OnchainCexComparison>, String> {
    identity::validate_quote_identity(config, &quotes.forward, &quotes.reverse)?;
    let book = cex
        .value
        .as_ref()
        .ok_or_else(|| "所选 CEX 现货交易对尚无 WS 最优买卖价".to_owned())?;
    identity::validate_book_identity(config, book)?;
    let (Some(bid), Some(ask)) = (book.best_bid(), book.best_ask()) else {
        return Err("所选 CEX 现货交易对缺少有效的 WS 最优买价或卖价".to_owned());
    };
    let Some(onchain_sell_price) =
        quote_price(&quotes.forward, config.base_decimals, config.quote_decimals)
    else {
        return Err("链上正向报价返回的输入或输出数量无效".to_owned());
    };
    let Some(reverse_base) = raw_units(&quotes.reverse.output_amount_raw, config.base_decimals)
    else {
        return Err("链上反向报价返回的 Base 数量无效".to_owned());
    };
    let Some(reverse_quote) = raw_units(&quotes.reverse.input_amount_raw, config.quote_decimals)
    else {
        return Err("链上反向报价返回的 Quote 数量无效".to_owned());
    };
    let Some(forward_base) = raw_units(&quotes.forward.input_amount_raw, config.base_decimals)
    else {
        return Err("链上正向报价返回的 Base 数量无效".to_owned());
    };
    let onchain_buy_price = reverse_quote / reverse_base;
    let normalized_bid = conversion.map_or(bid, |row| bid * row.cex_to_onchain_bid);
    let normalized_ask = conversion.map_or(ask, |row| ask * row.cex_to_onchain_ask);
    let bid_depth_usd = cex_depth_usd(&book.bids, bid, config.slippage_bps, BookSide::Bid)
        * conversion.map_or(1.0, |row| row.cex_to_onchain_bid);
    let ask_depth_usd = cex_depth_usd(&book.asks, ask, config.slippage_bps, BookSide::Ask)
        * conversion.map_or(1.0, |row| row.cex_to_onchain_ask);
    let cex_buy_notional = forward_base * normalized_ask;
    let buy_onchain_sell_cex_cost_notional_usd = reverse_quote;
    let buy_cex_sell_onchain_cost_notional_usd = cex_buy_notional;
    let bid_depth_usd = conversion.map_or(bid_depth_usd, |row| {
        bid_depth_usd.min(row.cex_to_onchain_capacity)
    });
    let ask_depth_usd = conversion.map_or(ask_depth_usd, |row| {
        ask_depth_usd.min(row.onchain_to_cex_capacity)
    });
    compare_quotes(QuoteInputs {
        onchain_sell_price,
        onchain_buy_price,
        cex_bid: normalized_bid,
        cex_ask: normalized_ask,
        cex_fee_bps: config.cex_taker_fee_bps,
        quote_conversion_fee_bps: conversion.map_or(0.0, |_| config.cex_taker_fee_bps),
        slippage_bps: config.slippage_bps,
        gas_usd: config.gas_usd,
        buy_onchain_sell_cex_cost_notional_usd: buy_onchain_sell_cex_cost_notional_usd * usd_rate,
        buy_onchain_sell_cex_observable_notional_usd: buy_onchain_sell_cex_cost_notional_usd
            .min(bid_depth_usd / normalized_bid * onchain_buy_price)
            * usd_rate,
        buy_cex_sell_onchain_cost_notional_usd: buy_cex_sell_onchain_cost_notional_usd * usd_rate,
        buy_cex_sell_onchain_observable_notional_usd: buy_cex_sell_onchain_cost_notional_usd
            .min(ask_depth_usd)
            * usd_rate,
    })
    .ok_or_else(|| "报价价格、成本或统一比较金额无效，暂不能计算净收益".to_owned())
}

pub(super) fn disabled_snapshot(
    config: OnchainComparisonConfig,
    now_ms: i64,
) -> OnchainComparisonSnapshot {
    let runtime = provider_runtime(&config.provider);
    base_snapshot(config, now_ms, &runtime, None)
}

pub(super) fn degraded_without_quotes(
    config: OnchainComparisonConfig,
    quality: OnchainComparisonQuality,
    reason: &str,
    now_ms: i64,
) -> OnchainComparisonSnapshot {
    let runtime = provider_runtime(&config.provider);
    with_problem(
        base_snapshot(config, now_ms, &runtime, None),
        quality,
        reason,
    )
}

pub(super) fn pending_snapshot(
    config: OnchainComparisonConfig,
    reason: &str,
    now_ms: i64,
) -> OnchainComparisonSnapshot {
    let runtime = provider_runtime(&config.provider);
    with_problem(
        base_snapshot(config, now_ms, &runtime, None),
        OnchainComparisonQuality::Pending,
        reason,
    )
}

fn base_snapshot(
    config: OnchainComparisonConfig,
    now_ms: i64,
    runtime: &ProviderRuntime,
    quotes: Option<&OnchainQuotePair>,
) -> OnchainComparisonSnapshot {
    let mut snapshot = OnchainComparisonSnapshot {
        config,
        quality: OnchainComparisonQuality::Disabled,
        comparisons: Vec::new(),
        quote_evidence: quotes.map_or_else(Vec::new, OnchainQuotePair::evidence),
        quote_observed_at_ms: quotes.map(|pair| pair.observed_at_ms),
        onchain_freshness_ms: quotes.map(|pair| now_ms.saturating_sub(pair.observed_at_ms)),
        onchain_latency_ms: quotes.map(|pair| pair.request_latency_ms),
        quote_interval_ms: runtime.quote_interval_ms,
        cex_source: "not_started".to_owned(),
        cex_freshness_ms: None,
        cex_observed_at_ms: None,
        quote_conversion: None,
        quote_usd_valuation: None,
        cex_problem: None,
        cex_retry_after_ms: None,
        projection_interval_ms: PROJECTION_INTERVAL_MS,
        provider_configured: runtime.configured,
        provider_problem: runtime.problem.clone(),
        provider_retry_after_ms: None,
        rpc_status: shared_types::OnchainRpcStatus::default(),
        cex_symbol_source: "adapter_spot_identity".to_owned(),
        degradation_reasons: Vec::new(),
        read_only: true,
        execution_readiness: shared_types::OnchainExecutionReadiness::default(),
        dex_comparison: shared_types::OnchainDexComparisonSnapshot::default(),
        cross_chain: shared_types::OnchainCrossChainSnapshot::default(),
        observed_at_ms: now_ms,
        batch: shared_types::OnchainBatchSnapshot::default(),
    };
    if !snapshot.config.enabled {
        snapshot.degradation_reasons = vec!["on-chain comparison is disabled".to_owned()];
    }
    apply_runtime_telemetry(&mut snapshot, runtime);
    snapshot
}

pub(super) fn apply_runtime_telemetry(
    snapshot: &mut OnchainComparisonSnapshot,
    runtime: &ProviderRuntime,
) {
    snapshot.quote_interval_ms = runtime.quote_interval_ms;
    snapshot.projection_interval_ms = PROJECTION_INTERVAL_MS;
    snapshot.provider_configured = runtime.configured;
    snapshot.provider_problem.clone_from(&runtime.problem);
    snapshot.cex_symbol_source = "adapter_spot_identity".to_owned();
}

fn with_problem(
    mut snapshot: OnchainComparisonSnapshot,
    quality: OnchainComparisonQuality,
    reason: &str,
) -> OnchainComparisonSnapshot {
    snapshot.quality = quality;
    snapshot.degradation_reasons = vec![reason.to_owned()];
    snapshot
}

fn quality_reason(
    quality: OnchainComparisonQuality,
    comparisons: &[shared_types::OnchainCexComparison],
    onchain_age_ms: i64,
    cex_age_ms: i64,
    config: &OnchainComparisonConfig,
    conversion_problem: Option<&str>,
) -> Vec<String> {
    match quality {
        OnchainComparisonQuality::Stale => vec![format!(
            "source age exceeds {}ms: on-chain {onchain_age_ms}ms, CEX {cex_age_ms}ms",
            config.max_age_ms
        )],
        OnchainComparisonQuality::LowLiquidity => {
            let best = comparisons
                .iter()
                .filter(|row| row.net_spread_bps > 0.0)
                .map(|row| row.observable_notional_usd)
                .filter(|value| value.is_finite())
                .fold(0.0_f64, f64::max);
            vec![format!(
                "best profitable direction previews ${best:.2} across the on-chain quote and CEX WS best-price size; this preview does not block a manual build; 100-level depth is checked when building; target is ${:.2}",
                config.min_liquidity_usd
            )]
        }
        OnchainComparisonQuality::NoNetProfit => {
            let best = comparisons
                .iter()
                .map(|row| row.net_spread_bps)
                .filter(|value| value.is_finite())
                .fold(f64::NEG_INFINITY, f64::max);
            vec![if best.is_finite() {
                format!(
                    "最佳费后净差 {:.4}% 低于配置门槛 {:.4}%",
                    best / 100.0,
                    config.spread_alert.min_net_spread_bps.max(0.0) / 100.0
                )
            } else {
                "没有可用的费后净差证据".to_owned()
            }]
        }
        OnchainComparisonQuality::RawCrossQuote => {
            vec![if let Some(problem) = conversion_problem {
                format!("Quote 换算尚未就绪：{problem}；当前只展示未换算原始价差")
            } else if !config.quote_identity_resolved {
                format!(
                "链上 Quote {} 已取得合约与精度，但符号身份尚未核验；当前只展示原始价格，不判断净收益、不发送确定性套利提醒，也不允许构建交易计划",
                config.quote_token,
            )
            } else {
                format!(
                "链上 Quote 为 {}，CEX Quote 为 {}；当前只展示未换算的原始价差，不判断净收益、不发送确定性套利提醒，也不允许构建交易计划",
                config.quote_token,
                onchain_cex_quote_token(&config.cex_symbol).unwrap_or("未知")
            )
            }]
        }
        OnchainComparisonQuality::RawCustomPair => {
            vec![if !config.base_identity_resolved {
                format!(
                "链上 Base {} 已取得合约与精度，但符号身份尚未核验；当前只展示原始价格，不判断净收益、不发送确定性套利提醒，也不允许构建交易计划",
                config.base_token,
            )
            } else {
                format!(
                "链上 Base 为 {}，CEX Base 为 {}；这是不同 Base 资产，当前只展示两个独立市场的原始价格，不判断净收益、不发送确定性套利提醒，也不允许构建交易计划",
                config.base_token,
                onchain_cex_base_token(&config.cex_symbol).unwrap_or("未知")
            )
            }]
        }
        _ => Vec::new(),
    }
}

#[derive(Debug, Clone, Copy)]
enum BookSide {
    Bid,
    Ask,
}

fn cex_depth_usd(levels: &[[f64; 2]], best_price: f64, slippage_bps: f64, side: BookSide) -> f64 {
    let slippage = (slippage_bps / 10_000.0).clamp(0.0, 1.0);
    let limit = match side {
        BookSide::Bid => best_price * (1.0 - slippage),
        BookSide::Ask => best_price * (1.0 + slippage),
    };
    levels
        .iter()
        .filter_map(|level| {
            let [price, quantity] = *level;
            let valid = price.is_finite() && quantity.is_finite() && price > 0.0 && quantity > 0.0;
            let within_limit = match side {
                BookSide::Bid => price >= limit,
                BookSide::Ask => price <= limit,
            };
            (valid && within_limit).then_some(price * quantity)
        })
        .sum()
}

pub(super) fn publish_snapshot(state: &AppState, next: &OnchainComparisonSnapshot, force: bool) {
    let mut next = next.clone();
    if next.config.enabled && next.provider_configured {
        next.provider_problem = state
            .onchain_monitor()
            .batch()
            .provider_problem(&next.config.provider);
        if let Some(problem) = next.provider_problem.as_ref() {
            if !next
                .degradation_reasons
                .iter()
                .any(|reason| reason == problem)
            {
                next.degradation_reasons.push(problem.clone());
            }
        }
    }
    next.provider_retry_after_ms = state
        .onchain_monitor()
        .batch()
        .provider_retry_after_ms(&next.config.provider, common::time::now_ms());
    next.batch = state
        .onchain_monitor()
        .batch()
        .snapshot_with_active(&next.config, next.quote_interval_ms);
    let current = state.onchain_monitor().snapshot();
    let source_changed = current.quote_observed_at_ms != next.quote_observed_at_ms
        || current.cex_observed_at_ms != next.cex_observed_at_ms
        || current.quote_usd_valuation != next.quote_usd_valuation
        || current.quality != next.quality
        || current.config != next.config
        || current.provider_problem != next.provider_problem
        || current.rpc_status != next.rpc_status
        || current.dex_comparison != next.dex_comparison
        || current.cross_chain != next.cross_chain
        || current.batch != next.batch;
    let heartbeat_due =
        next.observed_at_ms.saturating_sub(current.observed_at_ms) >= PROJECTION_HEARTBEAT_MS;
    if !(force || source_changed || heartbeat_due) {
        return;
    }
    state.onchain_monitor().publish(next.clone());
    if let Err(error) = crate::services::ws_publish::publish_onchain_comparison(state, &next) {
        tracing::warn!(error = %error, "failed to publish on-chain comparison snapshot");
    }
}

pub(super) fn batch_item_snapshot(
    item_id: String,
    snapshot: &OnchainComparisonSnapshot,
) -> OnchainBatchItemSnapshot {
    let raw_observation = snapshot.config.spread_alert.mode
        == OnchainSpreadAlertMode::RawObservation
        || matches!(
            snapshot.quality,
            OnchainComparisonQuality::RawCrossQuote | OnchainComparisonQuality::RawCustomPair
        );
    let best = snapshot.comparisons.iter().max_by(|left, right| {
        if raw_observation {
            left.gross_spread_bps.total_cmp(&right.gross_spread_bps)
        } else {
            left.net_spread_bps.total_cmp(&right.net_spread_bps)
        }
    });
    let best_dex = snapshot
        .dex_comparison
        .routes
        .iter()
        .filter(|route| route.net_return_bps.is_some_and(f64::is_finite))
        .max_by(|left, right| {
            left.net_return_bps
                .unwrap_or(f64::NEG_INFINITY)
                .total_cmp(&right.net_return_bps.unwrap_or(f64::NEG_INFINITY))
        });
    OnchainBatchItemSnapshot {
        item_id,
        config: snapshot.config.clone(),
        quality: snapshot.quality,
        best_direction: best.map(|row| row.direction),
        best_gross_spread_bps: best.map(|row| row.gross_spread_bps),
        best_net_spread_bps: best.map(|row| row.net_spread_bps),
        observable_notional_usd: best.map(|row| row.observable_notional_usd),
        quote_observed_at_ms: snapshot.quote_observed_at_ms,
        onchain_freshness_ms: snapshot.onchain_freshness_ms,
        onchain_latency_ms: snapshot.onchain_latency_ms,
        quote_interval_ms: snapshot.quote_interval_ms,
        cex_source: snapshot.cex_source.clone(),
        cex_freshness_ms: snapshot.cex_freshness_ms,
        cex_observed_at_ms: snapshot.cex_observed_at_ms,
        quote_conversion: snapshot.quote_conversion.clone(),
        quote_usd_valuation: snapshot.quote_usd_valuation.clone(),
        dex_quality: snapshot.dex_comparison.quality,
        best_dex_direction: best_dex.map(|route| route.direction),
        best_dex_net_return_bps: best_dex.and_then(|route| route.net_return_bps),
        dex_problem: snapshot.dex_comparison.problem.clone(),
        cross_chain_quality: snapshot.cross_chain.quality,
        best_cross_chain_net_return_bps: snapshot.cross_chain.net_return_bps,
        cross_chain_problem: snapshot.cross_chain.problem.clone(),
        cex_problem: snapshot.cex_problem.clone(),
        cex_retry_after_ms: snapshot.cex_retry_after_ms,
        provider_configured: snapshot.provider_configured,
        provider_problem: snapshot.provider_problem.clone(),
        provider_retry_after_ms: snapshot.provider_retry_after_ms,
        degradation_reasons: snapshot.degradation_reasons.clone(),
        observed_at_ms: snapshot.observed_at_ms,
    }
}
