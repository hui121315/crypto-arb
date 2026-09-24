use crate::services::market_data::{MarketQuality, MarketRead, MarketSource};
use crate::state::AppState;
use exchange::PublicWsSnapshot;
use onchain_monitor::OnchainQuoteTarget;
use shared_types::{OnchainComparisonQuality, OnchainComparisonSnapshot};
use std::sync::Arc;

pub(crate) mod stock_quotes;
pub(crate) mod stock_inventory;
pub(crate) mod stock_costs;

#[derive(Debug)]
pub(super) struct CexWsRefreshProblem {
    message: String,
    quality: MarketQuality,
    retry_after_ms: Option<i64>,
}

impl CexWsRefreshProblem {
    fn pending(message: String) -> Self {
        let quality = if ws_connection_problem(&message) {
            MarketQuality::CircuitOpen
        } else {
            MarketQuality::Warming
        };
        Self {
            message,
            quality,
            retry_after_ms: None,
        }
    }

    fn unsupported(message: String) -> Self {
        Self {
            message,
            quality: MarketQuality::Unsupported,
            retry_after_ms: None,
        }
    }

    fn exchange(error: &exchange::ExchangeError) -> Self {
        let quality = match error {
            exchange::ExchangeError::RateLimited { .. } => MarketQuality::RateLimited,
            exchange::ExchangeError::UnsupportedSymbol(_)
            | exchange::ExchangeError::UnsupportedCapability(_)
            | exchange::ExchangeError::NotImplemented(_) => MarketQuality::Unsupported,
            exchange::ExchangeError::Http { status: 429, .. } => MarketQuality::RateLimited,
            exchange::ExchangeError::Network(_)
            | exchange::ExchangeError::Timeout { .. }
            | exchange::ExchangeError::WsClosed(_)
            | exchange::ExchangeError::CircuitBreaker { .. }
            | exchange::ExchangeError::Auth(_)
            | exchange::ExchangeError::Http { .. }
            | exchange::ExchangeError::Parse(_)
            | exchange::ExchangeError::Api { .. } => MarketQuality::CircuitOpen,
        };
        Self {
            retry_after_ms: error
                .retry_after_ms()
                .map(|delay| i64::try_from(delay).unwrap_or(i64::MAX)),
            message: error.to_string(),
            quality,
        }
    }
}

fn ws_connection_problem(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "handshake timed out",
        "inbound idle timeout",
        "connection failed",
        "connect error",
        "connection refused",
        "websocket is disconnected",
        "ws closed",
        "circuit breaker",
        "dns",
        "tls",
        "连接失败",
        "ws 已断开",
        "连续重连失败",
        "入站心跳已过期",
    ]
    .iter()
    .any(|marker| message.contains(marker))
}

pub(super) fn apply_cex_refresh_problem(
    cex: &mut MarketRead<shared_types::OrderBookInfo>,
    problem: CexWsRefreshProblem,
) {
    if cex.value.is_none() && cex.quality != MarketQuality::Unsupported {
        cex.quality = problem.quality;
    }
    cex.retry_after_ms = problem.retry_after_ms.or(cex.retry_after_ms);
    cex.last_error = Some(problem.message);
}

mod alerts;
mod allowance;
mod batch;
mod cex_pairs;
mod config;
mod cow;
mod cross_chain;
mod cross_chain_reconcile;
mod cross_chain_submit;
mod cross_chain_recovery_preview;
mod dex_cross;
mod evm_token_identity;
mod execution_build;
mod execution_readiness;
mod execution_submit;
mod identity;
mod jupiter_quota;
pub(crate) mod lifi;
mod okx;
mod projection;
mod provider_runtime;
mod provider_types;
mod quote;
mod quote_conversion;
mod replenishment;
pub(crate) mod replenishment_allocation;
pub(crate) mod approval_allocation;
pub(crate) mod cross_chain_costs;
mod replenishment_chain_transfer;
pub(crate) mod stock_funding_transfer;
pub(crate) mod replenishment_costs;
mod replenishment_credit;
pub(crate) use replenishment_credit::stock_funding::read as read_stock_funding_receipt;
#[cfg(test)]
pub(crate) use replenishment_credit::stock_funding::read_with as read_stock_funding_receipt_with;
mod replenishment_submit;
mod rpc;
mod rpc_target;
mod solana_token_precision;
mod three_leg_execution;
mod token_approval;
mod token_identity_service;
mod token_registry;
mod usd_valuation;
mod cross_chain_accounting;
mod wallet_inventory;

pub(crate) use batch::restore_configs as restore_batch_configs;
pub(crate) use batch::snapshot as batch_snapshot;
pub(crate) use batch::{add_config as add_batch_config, remove as remove_batch_item};
pub(crate) use cex_pairs::catalog as cex_pair_catalog;
pub(crate) use config::update_config;
pub(crate) use cross_chain::authorize as authorize_cross_chain;
pub(crate) use cross_chain::build_preview as build_cross_chain_preview;
pub(crate) use cross_chain_recovery_preview::preview as preview_cross_chain_recovery;
pub(crate) use cross_chain::recent_runs as cross_chain_runs;
pub(crate) use cross_chain_reconcile::reconcile_pending as reconcile_cross_chain;
pub(crate) use cross_chain_accounting::refresh_pending as refresh_cross_chain_accounting;
pub(crate) use cross_chain_reconcile::request_recheck as recheck_cross_chain;
pub(crate) use cross_chain_submit::submit as submit_cross_chain;
pub(crate) use execution_submit::accounting::value_flows as value_execution_flows;
pub(crate) use execution_build::build as build_execution;
pub(crate) use execution_submit::recent_runs as execution_runs;
pub(crate) use execution_submit::refresh_accounting as refresh_execution_accounting;
pub(crate) use execution_submit::restore_reconciliations as restore_execution_reconciliations;
pub(crate) use execution_submit::submit as submit_execution;
pub(crate) use replenishment::authorize as authorize_replenishment;
pub(crate) use replenishment::build as build_replenishment;
pub(crate) use replenishment::recent_plans as replenishment_plans;
pub(crate) use replenishment::recent_runs as replenishment_runs;
pub(crate) use replenishment_submit::reconcile_pending as reconcile_replenishment;
pub(crate) use replenishment_submit::submit as submit_replenishment;
pub(crate) use replenishment_submit::recheck as recheck_replenishment;
pub(crate) use token_approval::build as build_token_approval;
pub(crate) use token_approval::recent_runs as token_approval_runs;
pub(crate) use token_approval::submit as submit_token_approval;
pub(crate) use token_approval::refresh_receipts as refresh_token_approval_receipts;
pub(crate) use token_identity_service::resolve as resolve_token_identity;

#[cfg(test)]
use projection::project_snapshot;
use projection::{
    attach_cex_telemetry, cex_unavailable_reason, degraded_without_quotes, disabled_snapshot,
    pending_snapshot, project_snapshot_with_conversion, publish_snapshot,
    retain_last_official_cex_projection,
};
use provider_runtime::{provider_runtime, quote_retry_backoff};
use quote::fetch_pair;

#[cfg(test)]
use config::resolve_changed_token_identity;
#[cfg(test)]
use onchain_monitor::OnchainQuotePair;
#[cfg(test)]
use provider_types::{
    JupiterOrderBuild, JupiterOrderQuote, OkxCredentials, ZeroExFirmQuote, ZeroExPrice,
};
#[cfg(test)]
use quote::{quote_price, raw_units, JUPITER_ORDER_DOCS, ZEROEX_PRICE_DOCS};
#[cfg(test)]
use shared_types::{OnchainComparisonConfig, OnchainComparisonConfigPatch};

pub(crate) fn snapshot(state: &AppState) -> Arc<OnchainComparisonSnapshot> {
    state.onchain_monitor().snapshot()
}

fn schedule_quote_retry(
    state: &AppState,
    provider: &str,
    reason: &str,
    quote_interval_ms: i64,
) -> Option<i64> {
    let scheduled_at_ms = common::time::now_ms();
    let normalized_reason = reason.to_ascii_lowercase();
    if normalized_reason.contains("http 429") || normalized_reason.contains("rate_limit_wait") {
        let keyed = match provider {
            "jupiter_swap_v2" => Some(false),
            "jupiter_swap_v2_keyed" => Some(true),
            _ => None,
        };
        if let Some(retry_after_ms) =
            keyed.and_then(|keyed| jupiter_quota::rate_limit_retry_after_ms(keyed, scheduled_at_ms))
        {
            return Some(state.onchain_monitor().batch().record_provider_retry_after(
                provider,
                scheduled_at_ms,
                retry_after_ms,
            ));
        }
    }
    let backoff = quote_retry_backoff(reason, quote_interval_ms)?;
    Some(state.onchain_monitor().batch().record_provider_failure(
        provider,
        scheduled_at_ms,
        backoff.base_delay_ms,
        backoff.max_delay_ms,
    ))
}

pub(crate) async fn refresh_if_due(state: &AppState, now_ms: i64) {
    let context = state.onchain_monitor().read_context();
    let active_config = context.snapshot.config.clone();
    let runtime = provider_runtime(&active_config.provider);
    let target = state.onchain_monitor().batch().next_due_target(
        &active_config,
        runtime.quote_interval_ms,
        now_ms,
    );
    let rpc_due = rpc::probe_due(
        &active_config,
        &state.onchain_monitor().rpc_status(),
        now_ms,
    );
    match target {
        Some(OnchainQuoteTarget::Active { config }) => {
            if config == active_config {
                refresh_sources(state, context, now_ms, true, rpc_due).await;
            }
        }
        Some(OnchainQuoteTarget::Batch {
            item_id,
            config: batch_config,
        }) => {
            let quote_refresh = batch::refresh_target(state, item_id, batch_config, now_ms);
            let rpc_refresh = async {
                if rpc_due {
                    refresh_rpc_status(state, &context, now_ms).await;
                }
            };
            tokio::join!(quote_refresh, rpc_refresh);
        }
        None if rpc_due => refresh_rpc_status(state, &context, now_ms).await,
        None => {}
    }
}

pub(crate) async fn refresh(state: &AppState, now_ms: i64) {
    let context = state.onchain_monitor().read_context();
    let config = context.snapshot.config.clone();
    if !config.enabled {
        refresh_rpc_status(state, &context, now_ms).await;
        let mut next = disabled_snapshot(config, common::time::now_ms());
        attach_rpc_status(state, &mut next);
        publish_snapshot(state, &context, &next, true);
        return;
    }
    let runtime = provider_runtime(&config.provider);
    let quote_due = state.onchain_monitor().batch().try_begin_active(
        &config,
        runtime.quote_interval_ms,
        now_ms,
    );
    refresh_sources(state, context, now_ms, quote_due, true).await;
}

pub(crate) fn refresh_provider_runtime(state: &AppState, now_ms: i64) {
    let context = state.onchain_monitor().read_context();
    let mut next = (*context.snapshot).clone();
    let runtime = provider_runtime(&next.config.provider);
    next.provider_configured = runtime.configured;
    next.provider_problem = runtime.problem;
    next.quote_interval_ms = runtime.quote_interval_ms;
    next.observed_at_ms = now_ms;
    batch::refresh_runtime(state, now_ms);
    publish_snapshot(state, &context, &next, true);
}

async fn refresh_sources(
    state: &AppState,
    context: onchain_monitor::OnchainReadContext,
    now_ms: i64,
    quote_due: bool,
    rpc_due: bool,
) {
    let config = context.snapshot.config.clone();
    let runtime = provider_runtime(&config.provider);
    let quote_refresh = async {
        if !quote_due {
            return Ok::<(), String>(());
        }
        if !runtime.configured {
            return Err(runtime
                .problem
                .clone()
                .unwrap_or_else(|| "quote provider is unavailable".to_owned()));
        }
        let quotes = fetch_pair(&config, now_ms).await?;
        let derived_primary = quotes.clone();
        if !state.onchain_monitor().publish_quote_pair(&context, quotes) {
            tracing::debug!(
                provider = %config.provider,
                "discarding quote completed for a superseded on-chain configuration"
            );
            return Ok(());
        }
        state
            .onchain_monitor()
            .batch()
            .record_provider_success(&config.provider);
        if config.dex_comparison.enabled && state.onchain_monitor().try_begin_dex_cross_refresh() {
            let state = state.clone();
            let dex_config = config.clone();
            let dex_context = context.clone();
            let dex_primary = derived_primary.clone();
            tokio::spawn(async move {
                let result = dex_cross::fetch(&dex_config, &dex_primary, now_ms).await;
                let accepted = match result {
                    Ok(quotes) => state
                        .onchain_monitor()
                        .publish_dex_cross_quotes(&dex_context, quotes),
                    Err(problem) => state
                        .onchain_monitor()
                        .publish_dex_cross_problem(&dex_context, problem),
                };
                if accepted {
                    project_latest(&state, common::time::now_ms(), true);
                    let snapshot = state.onchain_monitor().snapshot();
                    alerts::emit_dex_if_due(&state, &snapshot, common::time::now_ms()).await;
                }
                state.onchain_monitor().finish_dex_cross_refresh();
            });
        }
        if config.cross_chain.enabled {
            if let Some((peer_item_id, peer)) = cross_chain::peer_config(state, &config) {
                if state
                    .onchain_monitor()
                    .try_begin_cross_chain_refresh(now_ms, cross_chain::REFRESH_INTERVAL_MS)
                {
                    let state = state.clone();
                    let source_config = config.clone();
                    let source_context = context.clone();
                    let source_primary = derived_primary;
                    tokio::spawn(async move {
                        let result = cross_chain::fetch(
                            &source_config,
                            &peer_item_id,
                            &peer,
                            &source_primary,
                            now_ms,
                        )
                        .await;
                        let peer_unchanged = cross_chain::peer_config(&state, &source_config)
                            .is_some_and(|(item_id, config)| {
                                item_id == peer_item_id && config == peer
                            });
                        let accepted = peer_unchanged
                            && match result {
                                Ok(quotes) => state
                                    .onchain_monitor()
                                    .publish_cross_chain_quotes(&source_context, quotes),
                                Err(problem) => state
                                    .onchain_monitor()
                                    .publish_cross_chain_problem(&source_context, problem),
                            };
                        if accepted {
                            project_latest(&state, common::time::now_ms(), true);
                            let snapshot = state.onchain_monitor().snapshot();
                            alerts::emit_cross_chain_if_due(
                                &state,
                                &snapshot,
                                common::time::now_ms(),
                            )
                            .await;
                        }
                        state.onchain_monitor().finish_cross_chain_refresh();
                    });
                }
            }
        }
        Ok(())
    };
    let rpc_refresh = async {
        if rpc_due {
            refresh_rpc_status(state, &context, now_ms).await;
        }
    };
    let (quote_result, ()) = tokio::join!(quote_refresh, rpc_refresh);
    if !state.onchain_monitor().context_is_current(&context) {
        return;
    }
    if let Err(reason) = quote_result {
        state
            .onchain_monitor()
            .batch()
            .record_provider_problem(&config.provider, &reason);
        let retry_after_ms =
            schedule_quote_retry(state, &config.provider, &reason, runtime.quote_interval_ms);
        tracing::warn!(
            provider = %config.provider,
            quote_interval_ms = runtime.quote_interval_ms,
            ?retry_after_ms,
            error = %reason,
            "on-chain quote refresh failed"
        );
        if state
            .onchain_monitor()
            .quote_pair()
            .is_some_and(|quotes| quotes.matches_config(&config))
        {
            project_latest(state, common::time::now_ms(), true);
            return;
        }
        let completed_at_ms = common::time::now_ms();
        let mut next = degraded_without_quotes(
            config,
            OnchainComparisonQuality::UpstreamUnavailable,
            &reason,
            completed_at_ms,
        );
        attach_current_cex_state(state, &mut next, completed_at_ms);
        attach_rpc_status(state, &mut next);
        publish_snapshot(state, &context, &next, true);
        return;
    }
    project_latest(state, common::time::now_ms(), true);
    if quote_due {
        let snapshot = state.onchain_monitor().snapshot();
        alerts::emit_if_due(state, &snapshot, common::time::now_ms()).await;
    }
}

pub(super) async fn refresh_rpc_status(
    state: &AppState,
    context: &onchain_monitor::OnchainReadContext,
    now_ms: i64,
) {
    let status = rpc::probe(
        &context.snapshot.config,
        context.custom_rpc_url.as_deref().map(String::as_str),
        now_ms,
    )
    .await;
    if !state
        .onchain_monitor()
        .publish_rpc_status(context, status.clone())
    {
        return;
    }
    let mut next = (*state.onchain_monitor().snapshot()).clone();
    next.rpc_status = status;
    next.observed_at_ms = common::time::now_ms();
    publish_snapshot(state, context, &next, true);
}

fn attach_rpc_status(state: &AppState, snapshot: &mut OnchainComparisonSnapshot) {
    snapshot.rpc_status = (*state.onchain_monitor().rpc_status()).clone();
    if snapshot.config.rpc.mode == shared_types::OnchainRpcMode::Custom
        && !snapshot.rpc_status.ready
    {
        snapshot.quality = OnchainComparisonQuality::UpstreamUnavailable;
        snapshot.comparisons.clear();
        snapshot.degradation_reasons = vec![snapshot
            .rpc_status
            .problem
            .clone()
            .unwrap_or_else(|| "custom RPC evidence is not ready".to_owned())];
    }
}

pub(crate) fn project_latest(state: &AppState, now_ms: i64, force: bool) {
    project_latest_with_cex_problem(
        state,
        state.onchain_monitor().read_context(),
        now_ms,
        force,
        None,
    );
}

fn project_latest_with_cex_problem(
    state: &AppState,
    context: onchain_monitor::OnchainReadContext,
    now_ms: i64,
    force: bool,
    cex_refresh_problem: Option<CexWsRefreshProblem>,
) {
    let config = context.snapshot.config.clone();
    if !config.enabled {
        return;
    }
    let mut cex = selected_cex_bbo(state, &config, now_ms);
    if let Some(problem) = cex_refresh_problem {
        apply_cex_refresh_problem(&mut cex, problem);
    }
    let Some(quotes) = state.onchain_monitor().quote_pair() else {
        let current = state.onchain_monitor().snapshot();
        let mut next = quote_wait_snapshot(&current, config, now_ms);
        attach_cex_wait_state(&mut next, &cex, now_ms);
        attach_rpc_status(state, &mut next);
        execution_readiness::attach_global(state, &mut next, now_ms);
        publish_snapshot(state, &context, &next, force);
        return;
    };
    if !quotes.matches_config(&config) {
        let mut next = degraded_without_quotes(
            config,
            OnchainComparisonQuality::MappingInvalid,
            "缓存中的链上报价不属于当前合约配置，正在等待新配置报价",
            now_ms,
        );
        attach_cex_wait_state(&mut next, &cex, now_ms);
        attach_rpc_status(state, &mut next);
        execution_readiness::attach_global(state, &mut next, now_ms);
        publish_snapshot(state, &context, &next, force);
        return;
    }
    let current = state.onchain_monitor().snapshot();
    if let Some(mut next) = retain_last_official_cex_projection(&current, &config, &cex, now_ms) {
        attach_rpc_status(state, &mut next);
        publish_snapshot(state, &context, &next, force);
        return;
    }
    let conversion = quote_conversion::evidence(state, &config, now_ms);
    let valuation = usd_valuation::quote_evidence(state, &config, now_ms);
    let mut next =
        project_snapshot_with_conversion(config, &quotes, &cex, conversion, valuation, now_ms);
    attach_rpc_status(state, &mut next);
    execution_readiness::attach(state, &quotes, &mut next, now_ms);
    dex_cross::attach(state, &mut next, now_ms);
    cross_chain::attach(state, &mut next, now_ms);
    crate::lifecycle::request_onchain_transfer_networks(state, &next);
    batch::sync_active_snapshot(state, &context, &next, now_ms, force);
    publish_snapshot(state, &context, &next, force);
}

fn quote_wait_snapshot(
    current: &OnchainComparisonSnapshot,
    config: shared_types::OnchainComparisonConfig,
    now_ms: i64,
) -> OnchainComparisonSnapshot {
    if current.config == config
        && current.quote_observed_at_ms.is_none()
        && current.quality == OnchainComparisonQuality::UpstreamUnavailable
    {
        let mut next = current.clone();
        next.observed_at_ms = now_ms;
        next.comparisons.clear();
        return next;
    }
    pending_snapshot(config, "正在等待首轮链上双向报价", now_ms)
}

fn attach_current_cex_state(
    state: &AppState,
    snapshot: &mut OnchainComparisonSnapshot,
    now_ms: i64,
) {
    let cex = selected_cex_bbo(state, &snapshot.config, now_ms);
    attach_cex_wait_state(snapshot, &cex, now_ms);
}

fn attach_cex_wait_state(
    snapshot: &mut OnchainComparisonSnapshot,
    cex: &MarketRead<shared_types::OrderBookInfo>,
    now_ms: i64,
) {
    attach_cex_telemetry(snapshot, cex, now_ms);
    if cex.value.is_none() {
        let reason_prefix = format!(
            "{} {} 现货 WS 最优买卖价暂不可用：",
            snapshot.config.cex_venue, snapshot.config.cex_symbol
        );
        snapshot
            .degradation_reasons
            .retain(|reason| !reason.starts_with(&reason_prefix));
        let reason = cex_unavailable_reason(&snapshot.config, cex);
        snapshot.degradation_reasons.push(reason);
    }
}

pub(crate) async fn project_latest_from_ws(state: &AppState, _now_ms: i64) {
    let context = state.onchain_monitor().read_context();
    let previous = Arc::clone(&context.snapshot);
    let config = previous.config.clone();
    if !config.enabled {
        return;
    }
    let cex_refresh_problem = refresh_cex_bbo_from_ws(state, &config).await.err();
    if !state.onchain_monitor().context_is_current(&context) {
        return;
    }
    if let Some(problem) = cex_refresh_problem.as_ref() {
        tracing::debug!(
            venue = %config.cex_venue,
            symbol = %config.cex_symbol,
            problem = %problem.message,
            "selected on-chain CEX WS BBO is not ready"
        );
    }
    project_latest_with_cex_problem(
        state,
        context,
        common::time::now_ms(),
        false,
        cex_refresh_problem,
    );
    let next = state.onchain_monitor().snapshot();
    if alerts::crossed_alert_threshold(&previous, &next) {
        alerts::emit_if_due(state, &next, common::time::now_ms()).await;
    }
}

pub(super) fn selected_cex_bbo(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    now_ms: i64,
) -> MarketRead<shared_types::OrderBookInfo> {
    let read = state.market_data().spot_bbo_read(
        &config.cex_venue,
        &config.cex_symbol,
        now_ms,
        config.max_age_ms,
    );
    let mut read = ws_only_spot_bbo(read, &config.cex_venue, &config.cex_symbol);
    if read.value.is_none() && exact_cex_spot_listing(state, config, now_ms) == Some(false) {
        read.quality = MarketQuality::Unsupported;
        read.last_error = Some(format!(
            "{} 最新官方 Spot instrument registry 没有 {} 精确交易对；请改选已挂牌市场或等待 registry 刷新",
            config.cex_venue, config.cex_symbol
        ));
    }
    read
}

pub(super) fn ws_only_spot_bbo(
    mut read: MarketRead<shared_types::OrderBookInfo>,
    venue: &str,
    symbol: &str,
) -> MarketRead<shared_types::OrderBookInfo> {
    if read.value.is_none() || read.source == MarketSource::WsPush {
        return read;
    }
    let source = read.source;
    let previous_problem = read.last_error.take();
    read.value = None;
    read.freshness_ms = None;
    read.source = MarketSource::LocalCache;
    read.quality = if source == MarketSource::LocalCache {
        MarketQuality::StaleAllowed
    } else {
        MarketQuality::Warming
    };
    read.last_error = Some(if source == MarketSource::LocalCache {
        format!(
            "{} {} 缓存中的最优买卖价不是新鲜官方 WS 行情，正在等待 WS 恢复{}",
            venue,
            symbol,
            previous_problem.map_or_else(String::new, |problem| format!("：{problem}"))
        )
    } else {
        format!(
            "{} {} 的 {} 只用于启动预热，不能参与套利收益计算；正在等待首个官方 WS 最优买卖价帧",
            venue,
            symbol,
            source.as_str()
        )
    });
    read
}

#[cfg(test)]
fn ws_only_cex_bbo(
    read: MarketRead<shared_types::OrderBookInfo>,
    config: &shared_types::OnchainComparisonConfig,
) -> MarketRead<shared_types::OrderBookInfo> {
    ws_only_spot_bbo(read, &config.cex_venue, &config.cex_symbol)
}

fn exact_cex_spot_listing(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
    now_ms: i64,
) -> Option<bool> {
    let (base, quote) = crate::services::spot::split_spot_pair(&config.cex_symbol)?;
    state.instrument_registry().exact_spot_listing_evidence(
        &config.cex_venue,
        &base,
        &quote,
        now_ms,
    )
}

pub(super) async fn refresh_cex_bbo_from_ws(
    state: &AppState,
    config: &shared_types::OnchainComparisonConfig,
) -> Result<(), CexWsRefreshProblem> {
    usd_valuation::refresh(state, config).await;
    let adapter = state.aggregator().get(&config.cex_venue).ok_or_else(|| {
        CexWsRefreshProblem::unsupported(format!("{} adapter is not registered", config.cex_venue))
    })?;
    let mut symbols = vec![config.cex_symbol.clone()];
    if let Some(target) = quote_conversion::target(state, config, common::time::now_ms()) {
        if target.symbol != config.cex_symbol {
            symbols.push(target.symbol);
        }
    }
    match adapter.public_ws_spot_snapshot(&symbols).await {
        Ok(PublicWsSnapshot::Ready(rows)) => {
            state
                .market_data()
                .store_spot_ticks(&rows, MarketSource::WsPush);
            Ok(())
        }
        Ok(PublicWsSnapshot::Pending) => Err(CexWsRefreshProblem::pending(
            adapter
                .public_ws_spot_problem(&config.cex_symbol)
                .unwrap_or_else(|| {
                    format!(
                        "{} {} 现货 WS 正在等待首帧最优买卖价",
                        config.cex_venue, config.cex_symbol
                    )
                }),
        )),
        Ok(PublicWsSnapshot::Unsupported) => Err(CexWsRefreshProblem::unsupported(format!(
            "{} 适配器尚未提供现货 WS 最优买卖价快照",
            config.cex_venue
        ))),
        Err(error) => Err(CexWsRefreshProblem::exchange(&error)),
    }
}

pub(crate) async fn project_batch_latest_from_ws(state: &AppState, now_ms: i64) {
    batch::project_all_from_ws(state, now_ms).await;
}

pub(crate) async fn recover_batch_cex(state: &AppState, now_ms: i64) {
    batch::recover_cex(state, now_ms).await;
}

pub(crate) async fn refresh_wallet_inventory_if_due(state: &AppState, now_ms: i64) {
    wallet_inventory::refresh_if_due(state, now_ms).await;
}

#[cfg(test)]
mod tests;
