//! Instrument registry 启动期/周期灌库任务（PR-BM/PR-AL 深度接线）。
//!
//! 启动时立即拉取一次各 venue 官方 instrument 规格（Binance `exchangeInfo`、
//! OKX `instruments`、Gate.io `futures/usdt/contracts`），之后每 15 分钟刷新，灌入 [`InstrumentRegistry`]。
//! **fail-closed**：拉取失败仅告警、保留上次条目供诊断（绝不清库），但下游
//! scanner/hedge 预检必须等该 venue 下一次 fresh successful probe 才恢复执行。
//!
//! 多 venue 各自独立刷新——某一 venue 失败不影响其它 venue 既有规格（错误隔离），
//! 仅当全部 venue 刷新失败时整体任务才记为失败。充提网络不做全场轮询；只有
//! `SpotCross`、`SpotPerp` 或 `CrossSpotPerp` 先出现正收益候选时，才按候选涉及的
//! venue 异步读取一次官方钱包元数据。

use super::tasks::{delay_or_shutdown, tick_or_shutdown, BackgroundTasks, ShutdownToken};
use crate::services::instrument_registry::{InstrumentRegistry, SUPPORTED_VENUES};
use crate::state::AppState;
use futures::{stream, StreamExt};
use shared_types::ArbitrageOpportunityDto;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use tracing::{info, warn};

/// 新挂牌会先进入 ticker/funding feed；15 分钟周期避免 registry 落后数小时。
const UPDATER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15 * 60);
const COLD_START_DELAY: std::time::Duration = std::time::Duration::from_secs(4);
const REFRESH_CONCURRENCY: usize = 4;
const KRAKEN_TRANSFER_CURRENCY_BATCH_LIMIT: usize = 4;
/// 启动期灌库 + 周期刷新各 venue instrument 规格。
pub(super) fn spawn_updater(state: &AppState, tasks: &mut BackgroundTasks) {
    let interval = UPDATER_INTERVAL;
    let instrument_state = state.clone();
    let instrument_shutdown = tasks.shutdown_token();
    tasks.supervise("instruments", interval.as_millis() as i64, move || {
        let state = instrument_state.clone();
        let shutdown = instrument_shutdown.clone();
        async move { run_updater(state, shutdown).await }
    });

    info!(
        period_secs = interval.as_secs(),
        venues = ?SUPPORTED_VENUES,
        "instrument registry updater started"
    );
}

pub(super) fn invalidate_transfer_probe_after_exchange_change(state: &AppState, venue: &str) {
    state.instrument_registry().invalidate_transfer_probe(venue);
}

pub(super) fn request_transfer_networks_for_opportunities(
    state: &AppState,
    opportunities: &[ArbitrageOpportunityDto],
) {
    let registry = Arc::clone(state.instrument_registry());
    let now_ms = common::time::now_ms();
    let mut requested = BTreeMap::<String, Vec<String>>::new();
    for row in opportunities
        .iter()
        .filter(|row| registry.should_refresh_transfer_for_candidate(row, now_ms))
    {
        let currencies = transfer_currencies_for_symbol(&row.symbol);
        for venue in [&row.long_exchange, &row.short_exchange] {
            let venue = shared_types::venue_family(venue).to_ascii_lowercase();
            let batch = requested.entry(venue.clone()).or_default();
            for currency in &currencies {
                if batch.contains(currency)
                    || (venue == "kraken" && batch.len() >= KRAKEN_TRANSFER_CURRENCY_BATCH_LIMIT)
                {
                    continue;
                }
                batch.push(currency.clone());
            }
        }
    }
    let requests = requested
        .into_iter()
        .filter(|(venue, currencies)| {
            registry.begin_transfer_refresh_for(venue, currencies, now_ms)
        })
        .collect::<Vec<_>>();
    if requests.is_empty() {
        return;
    }
    let aggregator = Arc::clone(state.aggregator_handle());
    tokio::spawn(async move {
        let results = stream::iter(requests)
            .map(|(venue, currencies)| {
                let aggregator = Arc::clone(&aggregator);
                let registry = Arc::clone(&registry);
                async move {
                    let result =
                        refresh_transfer_venue(&venue, &currencies, &aggregator, &registry).await;
                    (venue, result)
                }
            })
            .buffer_unordered(REFRESH_CONCURRENCY)
            .collect::<Vec<_>>()
            .await;
        for (venue, result) in results {
            if let Err(error) = result {
                warn!(venue, error = %error, "candidate transfer network refresh failed");
            }
        }
    });
}

pub(super) fn request_transfer_networks_for_onchain(
    state: &AppState,
    snapshot: &shared_types::OnchainComparisonSnapshot,
) {
    let Some(venue) = onchain_transfer_refresh_venue(snapshot) else {
        return;
    };
    let registry = Arc::clone(state.instrument_registry());
    let now_ms = common::time::now_ms();
    let currencies = onchain_transfer_currencies(snapshot);
    if !registry.begin_transfer_refresh_for(&venue, &currencies, now_ms) {
        return;
    }
    let aggregator = Arc::clone(state.aggregator_handle());
    tokio::spawn(async move {
        if let Err(error) =
            refresh_transfer_venue(&venue, &currencies, &aggregator, &registry).await
        {
            warn!(venue, error = %error, "on-chain candidate transfer network refresh failed");
        }
    });
}

pub(super) async fn refresh_transfer_networks_for_onchain(
    state: &AppState,
    snapshot: &shared_types::OnchainComparisonSnapshot,
) -> Result<bool, String> {
    let Some(venue) = onchain_transfer_refresh_venue_on_demand(snapshot) else {
        return Ok(false);
    };
    let currencies = onchain_transfer_currencies_on_demand(snapshot);
    if currencies.is_empty() {
        return Ok(false);
    }
    let registry = Arc::clone(state.instrument_registry());
    if !registry.begin_transfer_refresh_for(&venue, &currencies, common::time::now_ms()) {
        return Ok(false);
    }
    refresh_transfer_venue(&venue, &currencies, state.aggregator_handle(), &registry).await?;
    Ok(true)
}

fn transfer_currencies_for_symbol(symbol: &str) -> Vec<String> {
    if let Some((base, quote)) = crate::services::spot::split_spot_pair(symbol) {
        return [base, quote]
            .into_iter()
            .map(|currency| currency.trim().to_ascii_uppercase())
            .filter(|currency| !currency.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
    }
    let currency = symbol.trim().to_ascii_uppercase();
    (!currency.is_empty())
        .then_some(currency)
        .into_iter()
        .collect()
}

fn onchain_transfer_currencies(snapshot: &shared_types::OnchainComparisonSnapshot) -> Vec<String> {
    let missing_inventory = snapshot
        .execution_readiness
        .directions
        .iter()
        .flat_map(|direction| &direction.inventory)
        .filter(|row| row.status == shared_types::OnchainInventoryStatus::Insufficient)
        .map(|row| row.asset.trim().to_ascii_uppercase())
        .filter(|currency| !currency.is_empty())
        .collect::<BTreeSet<_>>();
    if !missing_inventory.is_empty() {
        return missing_inventory.into_iter().collect();
    }
    let config = &snapshot.config;
    [
        Some(config.base_token.as_str()),
        Some(config.quote_token.as_str()),
        shared_types::onchain_cex_base_token(&config.cex_symbol),
        shared_types::onchain_cex_quote_token(&config.cex_symbol),
    ]
    .into_iter()
    .flatten()
    .map(|currency| currency.trim().to_ascii_uppercase())
    .filter(|currency| !currency.is_empty())
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect()
}

fn onchain_transfer_currencies_on_demand(
    snapshot: &shared_types::OnchainComparisonSnapshot,
) -> Vec<String> {
    snapshot
        .execution_readiness
        .directions
        .iter()
        .flat_map(|direction| &direction.inventory)
        .filter(|row| row.status != shared_types::OnchainInventoryStatus::Ready)
        .map(|row| row.asset.trim().to_ascii_uppercase())
        .filter(|currency| !currency.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn onchain_transfer_refresh_venue(
    snapshot: &shared_types::OnchainComparisonSnapshot,
) -> Option<String> {
    let profitable = snapshot.comparisons.iter().any(|row| {
        row.net_spread_bps > 0.0
            && row.net_spread_bps >= snapshot.config.spread_alert.min_net_spread_bps.max(0.0)
    });
    let inventory_missing = snapshot
        .execution_readiness
        .directions
        .iter()
        .flat_map(|row| &row.inventory)
        .any(|row| row.status == shared_types::OnchainInventoryStatus::Insufficient);
    if !profitable || !inventory_missing {
        return None;
    }
    let venue = onchain_transfer_refresh_venue_on_demand(snapshot)?;
    Some(venue)
}

fn onchain_transfer_refresh_venue_on_demand(
    snapshot: &shared_types::OnchainComparisonSnapshot,
) -> Option<String> {
    let inventory_unready = snapshot
        .execution_readiness
        .directions
        .iter()
        .flat_map(|row| &row.inventory)
        .any(|row| row.status != shared_types::OnchainInventoryStatus::Ready);
    if !snapshot.config.enabled || !inventory_unready {
        return None;
    }
    Some(shared_types::venue_family(&snapshot.config.cex_venue).to_ascii_lowercase())
}

async fn run_updater(state: AppState, shutdown: ShutdownToken) {
    if !delay_or_shutdown(COLD_START_DELAY, &shutdown).await {
        return;
    }
    let aggregator = Arc::clone(state.aggregator_handle());
    let registry = Arc::clone(state.instrument_registry());
    let task_registry = state.task_registry().clone();
    let interval = UPDATER_INTERVAL;
    let started_at_ms = common::time::now_ms();
    let outcome = refresh_all(&aggregator, &registry).await;
    task_registry.record_result_timed("instruments", started_at_ms, outcome);

    let mut tick = tokio::time::interval(interval);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    tick.tick().await;
    while tick_or_shutdown(&mut tick, &shutdown).await {
        let started_at_ms = common::time::now_ms();
        let outcome = refresh_all(&aggregator, &registry).await;
        task_registry.record_result_timed("instruments", started_at_ms, outcome);
    }
}

/// 逐 venue 独立刷新；错误隔离——单 venue 失败仅告警并保留其既有条目，其它 venue
/// 照常刷新。仅当**全部** venue 失败时整体记为失败（health 可见），任一成功即 Ok。
async fn refresh_all(
    aggregator: &exchange::Aggregator,
    registry: &InstrumentRegistry,
) -> Result<(), String> {
    let venues = SUPPORTED_VENUES
        .iter()
        .map(|venue| (*venue).to_owned())
        .collect::<Vec<_>>();
    let results = stream::iter(venues)
        .map(|venue| async move {
            let result = refresh_venue(&venue, aggregator, registry).await;
            (venue, result)
        })
        .buffer_unordered(REFRESH_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;
    let (any_ok, errors) = summarize_refresh_results(results);
    persist_refreshed_registry(any_ok, registry).await;
    instrument_refresh_outcome(any_ok, &errors)
}

fn summarize_refresh_results(results: Vec<(String, Result<(), String>)>) -> (bool, Vec<String>) {
    let mut errors: Vec<String> = Vec::new();
    let mut any_ok = false;
    for (venue, result) in results {
        match result {
            Ok(()) => any_ok = true,
            Err(err) => {
                warn!(venue, error = %err, "instrument refresh failed");
                errors.push(err);
            }
        }
    }
    (any_ok, errors)
}

async fn persist_refreshed_registry(any_ok: bool, registry: &InstrumentRegistry) {
    if any_ok {
        if let Err(error) = registry.persist_checkpoint().await {
            warn!(%error, "instrument registry checkpoint persist failed");
        }
    }
}

fn instrument_refresh_outcome(any_ok: bool, errors: &[String]) -> Result<(), String> {
    if errors.is_empty() || any_ok {
        return Ok(());
    }
    Err(errors.join("; "))
}

async fn refresh_transfer_venue(
    venue: &str,
    currencies: &[String],
    aggregator: &exchange::Aggregator,
    registry: &InstrumentRegistry,
) -> Result<(), String> {
    let Some(adapter) = aggregator.get(venue) else {
        let error = format!("{venue} adapter not registered");
        registry.record_transfer_unsupported(venue, &error);
        return Err(error);
    };
    let rows = match adapter.fetch_transfer_networks_for(currencies).await {
        Ok(rows) => rows,
        Err(error) => {
            let message = format!("{venue} fetch_transfer_networks failed: {error}");
            if matches!(
                &error,
                exchange::ExchangeError::UnsupportedCapability(_)
                    | exchange::ExchangeError::NotImplemented(_)
            ) {
                registry.record_transfer_unsupported(venue, &message);
            } else {
                registry.record_transfer_failure(venue, &message);
            }
            return Err(message);
        }
    };
    let fetched = rows.len();
    if fetched == 0 {
        let detail = if currencies.is_empty() {
            format!("{venue} official transfer endpoint returned no networks")
        } else {
            format!(
                "{venue} official transfer endpoint returned no networks for {}",
                currencies.join(",")
            )
        };
        registry.record_transfer_unavailable(venue, currencies, &detail);
        info!(venue, requested = ?currencies, "currency transfer networks unavailable for candidate");
        return Ok(());
    }
    let accepted = registry.replace_transfer_venue_scope(venue, currencies, rows);
    if accepted == 0 {
        let error = format!("{venue} transfer refresh accepted 0 of {fetched} rows");
        registry.record_transfer_failure(venue, &error);
        return Err(error);
    }
    info!(
        venue,
        fetched,
        accepted,
        total = registry.transfer_len(),
        "currency transfer networks refreshed"
    );
    Ok(())
}

/// 拉取单个 venue 官方 instrument 规格并原子替换 registry 中该 venue 的条目。
///
/// fail-closed：adapter 缺失/拉取失败 → 返回 `Err` 且**不动**既有条目（保留上次
/// 成功规格供诊断，执行授权由 probe state 立即撤销，直到下一次成功刷新）。
pub(super) async fn refresh_venue(
    venue: &str,
    aggregator: &exchange::Aggregator,
    registry: &InstrumentRegistry,
) -> Result<(), String> {
    if !registry.begin_instrument_refresh(venue) {
        return Ok(());
    }
    let result = refresh_venue_inner(venue, aggregator, registry).await;
    registry.finish_instrument_refresh(venue);
    result
}

async fn refresh_venue_inner(
    venue: &str,
    aggregator: &exchange::Aggregator,
    registry: &InstrumentRegistry,
) -> Result<(), String> {
    let Some(adapter) = aggregator.get(venue) else {
        let error = format!("{venue} adapter not registered");
        registry.record_unsupported(venue, &error);
        return Err(error);
    };
    let instruments = match adapter.fetch_instruments().await {
        Ok(instruments) => instruments,
        Err(error) => {
            let error = format!("{venue} fetch_instruments failed: {error}");
            registry.record_refresh_failure(venue, &error);
            return Err(error);
        }
    };
    let fetched = instruments.len();
    let accepted = registry.replace_venue(venue, instruments);
    if accepted == 0 {
        let error = format!("{venue} refresh accepted 0 of {fetched} instruments");
        registry.record_refresh_failure(venue, &error);
        warn!(
            venue,
            fetched, "instrument refresh yielded zero usable specs"
        );
        return Err(error);
    }
    info!(
        venue,
        fetched,
        accepted,
        total = registry.len(),
        "instrument registry refreshed"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        instrument_refresh_outcome, onchain_transfer_currencies,
        onchain_transfer_currencies_on_demand, onchain_transfer_refresh_venue,
        onchain_transfer_refresh_venue_on_demand, transfer_currencies_for_symbol,
    };
    use shared_types::{
        OnchainCexComparison, OnchainCexInstrumentEvidence, OnchainComparisonDirection,
        OnchainDirectionReadiness, OnchainInventoryEvidence, OnchainInventoryLocation,
        OnchainInventoryStatus, OnchainPathReadiness,
    };

    #[test]
    fn partial_refresh_keeps_supervised_task_healthy() {
        let errors = vec!["optional venue unavailable".to_owned()];

        assert!(instrument_refresh_outcome(true, &errors).is_ok());
    }

    #[test]
    fn total_refresh_failure_remains_visible_to_task_health() {
        let errors = vec![
            "binance unavailable".to_owned(),
            "okx unavailable".to_owned(),
        ];

        assert_eq!(
            instrument_refresh_outcome(false, &errors),
            Err("binance unavailable; okx unavailable".to_owned())
        );
    }

    #[test]
    fn onchain_transfer_refresh_is_candidate_and_inventory_triggered() {
        let mut snapshot = shared_types::OnchainComparisonSnapshot::default();
        snapshot.config.enabled = true;
        snapshot.config.cex_venue = "kraken:spot".to_owned();
        snapshot.config.spread_alert.min_net_spread_bps = 20.0;
        snapshot.comparisons.push(OnchainCexComparison {
            direction: OnchainComparisonDirection::BuyOnchainSellCex,
            onchain_price: 1.0,
            cex_price: 1.01,
            gross_spread_bps: 100.0,
            cex_fee_bps: 10.0,
            quote_conversion_fee_bps: 0.0,
            slippage_bps: 5.0,
            gas_usd: 0.01,
            gas_bps: 1.0,
            total_cost_bps: 16.0,
            net_spread_bps: 84.0,
            observable_notional_usd: 100.0,
            executable: false,
        });
        snapshot
            .execution_readiness
            .directions
            .push(OnchainDirectionReadiness {
                direction: OnchainComparisonDirection::BuyOnchainSellCex,
                path: OnchainPathReadiness::default(),
                inventory: vec![OnchainInventoryEvidence {
                    location: OnchainInventoryLocation::Cex,
                    scope: "kraken:spot".to_owned(),
                    asset: "PUPS".to_owned(),
                    required: 10.0,
                    available: Some(0.0),
                    status: OnchainInventoryStatus::Insufficient,
                    source: "account_balance".to_owned(),
                    observed_at_ms: Some(1),
                    problem: None,
                }],
                cex_instrument: OnchainCexInstrumentEvidence::default(),
                build_ready: false,
                submit_ready: false,
                blockers: Vec::new(),
            });

        assert_eq!(
            onchain_transfer_refresh_venue(&snapshot).as_deref(),
            Some("kraken")
        );
        snapshot.config.base_token = "PUPS".to_owned();
        snapshot.config.quote_token = "USDC".to_owned();
        snapshot.config.cex_symbol = "PUPS/USD".to_owned();
        assert_eq!(
            onchain_transfer_currencies(&snapshot),
            vec!["PUPS".to_owned()]
        );

        snapshot.comparisons[0].net_spread_bps = 19.99;
        assert_eq!(onchain_transfer_refresh_venue(&snapshot), None);
        assert_eq!(
            onchain_transfer_refresh_venue_on_demand(&snapshot).as_deref(),
            Some("kraken")
        );

        snapshot.comparisons[0].net_spread_bps = 84.0;
        snapshot.execution_readiness.directions[0].inventory[0].status =
            OnchainInventoryStatus::Unknown;
        assert_eq!(onchain_transfer_refresh_venue(&snapshot), None);
        assert_eq!(
            onchain_transfer_refresh_venue_on_demand(&snapshot).as_deref(),
            Some("kraken")
        );
        assert_eq!(
            onchain_transfer_currencies_on_demand(&snapshot),
            vec!["PUPS".to_owned()]
        );
    }

    #[test]
    fn candidate_transfer_scope_keeps_only_involved_assets() {
        assert_eq!(
            transfer_currencies_for_symbol("PUPS/USDC"),
            vec!["PUPS".to_owned(), "USDC".to_owned()]
        );
        assert_eq!(
            transfer_currencies_for_symbol("BTC"),
            vec!["BTC".to_owned()]
        );
    }
}
