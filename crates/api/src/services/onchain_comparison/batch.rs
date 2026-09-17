use super::projection::{
    batch_item_snapshot, degraded_without_quotes, project_snapshot_with_conversion,
    publish_snapshot,
};
use super::provider_runtime::provider_runtime;
use super::quote::fetch_pair;
use crate::state::AppState;
use common::AppError;
use onchain_monitor::{batch_item_id, OnchainMonitor};
use shared_types::{
    OnchainBatchSnapshot, OnchainComparisonConfig, OnchainComparisonConfigPatch,
    OnchainComparisonQuality, OnchainComparisonSnapshot, OnchainRpcMode, ONCHAIN_BATCH_MAX_ITEMS,
};
use std::sync::Arc;

pub(crate) fn snapshot(state: &AppState) -> Arc<OnchainBatchSnapshot> {
    let active = state.onchain_monitor().snapshot();
    let runtime = provider_runtime(&active.config.provider);
    Arc::new(
        state
            .onchain_monitor()
            .batch()
            .snapshot_with_active(&active.config, runtime.quote_interval_ms),
    )
}

pub(crate) async fn add_config(
    state: &AppState,
    patch: &OnchainComparisonConfigPatch,
    now_ms: i64,
) -> Result<Arc<OnchainBatchSnapshot>, AppError> {
    let _mutation = state.onchain_config_mutation_lock().lock().await;
    let active_config = state.onchain_monitor().snapshot().config.clone();
    let resolved = super::config::resolve_patch(state, &active_config, patch).await?;
    let mut config = state
        .onchain_monitor()
        .preview_config(&resolved)
        .map_err(|error| AppError::BadRequest(error.to_string()))?;
    // The submitted RPC can prove token identity during this request, but batch
    // monitoring itself only consumes public DEX quotes and CEX WS BBO data.
    config.rpc.mode = OnchainRpcMode::ProviderManaged;
    config.enabled = true;
    let runtime = provider_runtime(&config.provider);
    let item_id = batch_item_id(&config);
    let mut batch_configs = current_batch_configs(state);
    if let Some(existing) = batch_configs
        .iter_mut()
        .find(|existing| batch_item_id(existing) == item_id)
    {
        *existing = config.clone();
    } else {
        if batch_configs.len() >= ONCHAIN_BATCH_MAX_ITEMS {
            return Err(AppError::BadRequest(format!(
                "批量监控最多支持 {ONCHAIN_BATCH_MAX_ITEMS} 个组合"
            )));
        }
        batch_configs.push(config.clone());
    }
    persist_checkpoint(state, active_config, batch_configs).await?;
    state
        .onchain_monitor()
        .batch()
        .upsert(config, runtime.quote_interval_ms, now_ms)
        .map_err(AppError::BadRequest)?;
    sync_active_snapshot(state, &state.onchain_monitor().snapshot(), now_ms, true);
    publish_batch(state, now_ms, true);
    Ok(snapshot(state))
}

pub(crate) async fn remove(
    state: &AppState,
    item_id: &str,
    now_ms: i64,
) -> Result<Arc<OnchainBatchSnapshot>, AppError> {
    let _mutation = state.onchain_config_mutation_lock().lock().await;
    let active_config = state.onchain_monitor().snapshot().config.clone();
    let mut batch_configs = current_batch_configs(state);
    let previous_len = batch_configs.len();
    batch_configs.retain(|config| batch_item_id(config) != item_id);
    if batch_configs.len() == previous_len {
        return Err(AppError::NotFound(format!(
            "on-chain batch item: {item_id}"
        )));
    }
    persist_checkpoint(state, active_config, batch_configs).await?;
    if !state.onchain_monitor().batch().remove(item_id, now_ms) {
        return Err(AppError::Config(format!(
            "persisted batch removal was not found in runtime: {item_id}"
        )));
    }
    publish_batch(state, now_ms, true);
    Ok(snapshot(state))
}

pub(crate) fn restore_configs(
    monitor: &OnchainMonitor,
    configs: Vec<OnchainComparisonConfig>,
    now_ms: i64,
) {
    for config in configs {
        if let Err(problem) = restore_config(monitor, config, now_ms) {
            tracing::warn!(%problem, "restored on-chain batch config was rejected; skipping item");
        }
    }
}

fn restore_config(
    monitor: &OnchainMonitor,
    mut config: OnchainComparisonConfig,
    now_ms: i64,
) -> Result<(), String> {
    config.enabled = true;
    let mut validation_config = config.clone();
    validation_config.rpc.mode = OnchainRpcMode::ProviderManaged;
    OnchainMonitor::from_config(validation_config, now_ms).map_err(|error| error.to_string())?;
    let runtime = provider_runtime(&config.provider);
    monitor
        .batch()
        .upsert(config, runtime.quote_interval_ms, now_ms)
        .map(|_| ())
}

fn current_batch_configs(state: &AppState) -> Vec<OnchainComparisonConfig> {
    state
        .onchain_monitor()
        .batch()
        .configs()
        .into_iter()
        .map(|(_, config)| config)
        .collect()
}

async fn persist_checkpoint(
    state: &AppState,
    active_config: OnchainComparisonConfig,
    batch_configs: Vec<OnchainComparisonConfig>,
) -> Result<(), AppError> {
    let store = Arc::clone(state.onchain_config_store());
    tokio::task::spawn_blocking(move || store.persist(&active_config, &batch_configs))
        .await
        .map_err(anyhow::Error::new)?
        .map_err(anyhow::Error::new)?;
    Ok(())
}

pub(super) async fn refresh_target(
    state: &AppState,
    item_id: String,
    config: OnchainComparisonConfig,
    now_ms: i64,
) {
    let runtime = provider_runtime(&config.provider);
    let result = if runtime.configured {
        fetch_pair(&config, now_ms).await
    } else {
        Err(runtime
            .problem
            .clone()
            .unwrap_or_else(|| "quote provider is unavailable".to_owned()))
    };
    match result {
        Ok(quotes) => {
            let derived_primary = quotes.clone();
            state
                .onchain_monitor()
                .batch()
                .record_provider_success(&config.provider);
            state
                .onchain_monitor()
                .batch()
                .publish_quote_pair(&item_id, quotes);
            let completed_at_ms = common::time::now_ms();
            if let Some(projected) =
                project_item(state, &item_id, &config, completed_at_ms, true, None)
            {
                super::alerts::emit_if_due(state, &projected, common::time::now_ms()).await;
            }
            if config.dex_comparison.enabled
                && state
                    .onchain_monitor()
                    .batch()
                    .try_begin_dex_cross_refresh(&item_id)
            {
                let state = state.clone();
                let item_id = item_id.clone();
                let config = config.clone();
                let dex_primary = derived_primary.clone();
                tokio::spawn(async move {
                    let result = super::dex_cross::fetch(&config, &dex_primary, now_ms).await;
                    match result {
                        Ok(quotes) => state
                            .onchain_monitor()
                            .batch()
                            .publish_dex_cross_quotes(&item_id, quotes),
                        Err(problem) => state
                            .onchain_monitor()
                            .batch()
                            .publish_dex_cross_problem(&item_id, problem),
                    }
                    if let Some(projected) = project_item(
                        &state,
                        &item_id,
                        &config,
                        common::time::now_ms(),
                        true,
                        None,
                    ) {
                        super::alerts::emit_dex_if_due(&state, &projected, common::time::now_ms())
                            .await;
                    }
                    state
                        .onchain_monitor()
                        .batch()
                        .finish_dex_cross_refresh(&item_id);
                    publish_batch(&state, common::time::now_ms(), true);
                });
            }
            if config.cross_chain.enabled {
                if let Some((peer_item_id, peer)) = super::cross_chain::peer_config(state, &config)
                {
                    if state
                        .onchain_monitor()
                        .batch()
                        .try_begin_cross_chain_refresh(
                            &item_id,
                            now_ms,
                            super::cross_chain::REFRESH_INTERVAL_MS,
                        )
                    {
                        let state = state.clone();
                        let item_id = item_id.clone();
                        let source_config = config.clone();
                        let source_primary = derived_primary;
                        tokio::spawn(async move {
                            let result = super::cross_chain::fetch(
                                &source_config,
                                &peer_item_id,
                                &peer,
                                &source_primary,
                                now_ms,
                            )
                            .await;
                            let source_unchanged = state
                                .onchain_monitor()
                                .batch()
                                .configs()
                                .into_iter()
                                .any(|(id, config)| id == item_id && config == source_config);
                            let peer_unchanged =
                                super::cross_chain::peer_config(&state, &source_config)
                                    .is_some_and(|(id, config)| {
                                        id == peer_item_id && config == peer
                                    });
                            if source_unchanged && peer_unchanged {
                                match result {
                                    Ok(quotes) => state
                                        .onchain_monitor()
                                        .batch()
                                        .publish_cross_chain_quotes(&item_id, quotes),
                                    Err(problem) => state
                                        .onchain_monitor()
                                        .batch()
                                        .publish_cross_chain_problem(&item_id, problem),
                                }
                                if let Some(projected) = project_item(
                                    &state,
                                    &item_id,
                                    &source_config,
                                    common::time::now_ms(),
                                    true,
                                    None,
                                ) {
                                    super::alerts::emit_cross_chain_if_due(
                                        &state,
                                        &projected,
                                        common::time::now_ms(),
                                    )
                                    .await;
                                }
                            }
                            state
                                .onchain_monitor()
                                .batch()
                                .finish_cross_chain_refresh(&item_id);
                            publish_batch(&state, common::time::now_ms(), true);
                        });
                    }
                }
            }
        }
        Err(reason) => {
            state
                .onchain_monitor()
                .batch()
                .record_provider_problem(&config.provider, &reason);
            let retry_after_ms = super::schedule_quote_retry(
                state,
                &config.provider,
                &reason,
                runtime.quote_interval_ms,
            );
            tracing::warn!(
                item_id = %item_id,
                provider = %config.provider,
                ?retry_after_ms,
                error = %reason,
                "batch on-chain quote refresh failed"
            );
            if state
                .onchain_monitor()
                .batch()
                .quote_pair(&item_id)
                .is_some_and(|quotes| quotes.matches_config(&config))
            {
                project_item(state, &item_id, &config, common::time::now_ms(), true, None);
            } else {
                let degraded = degraded_without_quotes(
                    config,
                    OnchainComparisonQuality::UpstreamUnavailable,
                    &reason,
                    now_ms,
                );
                let item = batch_item_with_retry(state, item_id, &degraded, now_ms);
                state
                    .onchain_monitor()
                    .batch()
                    .update_item(item, now_ms, true);
            }
        }
    }
    publish_batch(state, common::time::now_ms(), true);
}

pub(crate) async fn project_all_from_ws(state: &AppState, now_ms: i64) {
    let active = state.onchain_monitor().snapshot();
    let active_id = active.config.enabled.then(|| batch_item_id(&active.config));
    let previous_items = state.onchain_monitor().batch().snapshot();
    let configs = state.onchain_monitor().batch().configs();
    let mut changed = false;
    for (item_id, config) in configs {
        if active_id.as_deref() == Some(item_id.as_str()) {
            changed |= sync_active_snapshot(state, &active, now_ms, false);
            continue;
        }
        let cex_problem = super::refresh_cex_bbo_from_ws(state, &config).await.err();
        let projected = project_item(
            state,
            &item_id,
            &config,
            common::time::now_ms(),
            false,
            cex_problem,
        );
        let Some(projected) = projected else {
            continue;
        };
        changed = true;
        let previous = previous_items
            .items
            .iter()
            .find(|item| item.item_id == item_id);
        if super::alerts::batch_crossed_alert_threshold(previous, &projected) {
            super::alerts::emit_if_due(state, &projected, common::time::now_ms()).await;
        }
    }
    if changed {
        publish_batch(state, common::time::now_ms(), false);
    }
}

pub(crate) async fn recover_cex(state: &AppState, _now_ms: i64) {
    let active = state.onchain_monitor().snapshot();
    let active_id = active.config.enabled.then(|| batch_item_id(&active.config));
    for (item_id, config) in state.onchain_monitor().batch().configs() {
        if active_id.as_deref() == Some(item_id.as_str()) {
            continue;
        }
        let _ = super::refresh_cex_bbo_from_ws(state, &config).await;
    }
}

pub(super) fn sync_active_snapshot(
    state: &AppState,
    snapshot: &OnchainComparisonSnapshot,
    now_ms: i64,
    force: bool,
) -> bool {
    if !snapshot.config.enabled {
        return false;
    }
    if state
        .onchain_monitor()
        .quote_pair()
        .is_some_and(|quotes| !quotes.matches_config(&snapshot.config))
    {
        return false;
    }
    let item_id = batch_item_id(&snapshot.config);
    let item = batch_item_with_retry(state, item_id, snapshot, now_ms);
    state
        .onchain_monitor()
        .batch()
        .update_item(item, now_ms, force)
}

pub(super) fn refresh_runtime(state: &AppState, now_ms: i64) {
    let configs = state.onchain_monitor().batch().configs();
    for (item_id, config) in configs {
        let Some(quotes) = state.onchain_monitor().batch().quote_pair(&item_id) else {
            let runtime = provider_runtime(&config.provider);
            let mut item = shared_types::OnchainBatchItemSnapshot::pending(
                item_id,
                config,
                runtime.quote_interval_ms,
                now_ms,
            );
            item.provider_configured = runtime.configured;
            item.provider_problem = runtime.problem.or_else(|| {
                state
                    .onchain_monitor()
                    .batch()
                    .provider_problem(&item.config.provider)
            });
            item.provider_retry_after_ms = state
                .onchain_monitor()
                .batch()
                .provider_retry_after_ms(&item.config.provider, now_ms);
            state
                .onchain_monitor()
                .batch()
                .update_item(item, now_ms, true);
            continue;
        };
        let cex = super::selected_cex_bbo(state, &config, now_ms);
        let conversion = super::quote_conversion::evidence(state, &config, now_ms);
        let valuation = super::usd_valuation::quote_evidence(state, &config, now_ms);
        let mut projected =
            project_snapshot_with_conversion(config, &quotes, &cex, conversion, valuation, now_ms);
        super::dex_cross::attach_batch(state, &item_id, &mut projected, now_ms);
        super::cross_chain::attach_batch(state, &item_id, &mut projected, now_ms);
        state.onchain_monitor().batch().update_item(
            batch_item_with_retry(state, item_id, &projected, now_ms),
            now_ms,
            true,
        );
    }
}

fn project_item(
    state: &AppState,
    item_id: &str,
    config: &OnchainComparisonConfig,
    now_ms: i64,
    force: bool,
    cex_problem: Option<super::CexWsRefreshProblem>,
) -> Option<OnchainComparisonSnapshot> {
    let quotes = state.onchain_monitor().batch().quote_pair(item_id)?;
    let mut projected = if quotes.matches_config(config) {
        let mut cex = super::selected_cex_bbo(state, config, now_ms);
        if cex.quality != crate::services::market_data::MarketQuality::Unsupported {
            if let Some(problem) = cex_problem {
                super::apply_cex_refresh_problem(&mut cex, problem);
            }
        }
        let conversion = super::quote_conversion::evidence(state, config, now_ms);
        let valuation = super::usd_valuation::quote_evidence(state, config, now_ms);
        project_snapshot_with_conversion(
            config.clone(),
            &quotes,
            &cex,
            conversion,
            valuation,
            now_ms,
        )
    } else {
        degraded_without_quotes(
            config.clone(),
            OnchainComparisonQuality::MappingInvalid,
            "cached batch quote identity does not match the configured pair",
            now_ms,
        )
    };
    super::dex_cross::attach_batch(state, item_id, &mut projected, now_ms);
    super::cross_chain::attach_batch(state, item_id, &mut projected, now_ms);
    let changed = state.onchain_monitor().batch().update_item(
        batch_item_with_retry(state, item_id.to_owned(), &projected, now_ms),
        now_ms,
        force,
    );
    changed.then_some(projected)
}

fn batch_item_with_retry(
    state: &AppState,
    item_id: String,
    snapshot: &OnchainComparisonSnapshot,
    now_ms: i64,
) -> shared_types::OnchainBatchItemSnapshot {
    let mut item = batch_item_snapshot(item_id, snapshot);
    if item.provider_configured {
        item.provider_problem = state
            .onchain_monitor()
            .batch()
            .provider_problem(&item.config.provider);
    }
    item.provider_retry_after_ms = state
        .onchain_monitor()
        .batch()
        .provider_retry_after_ms(&item.config.provider, now_ms);
    item
}

fn publish_batch(state: &AppState, now_ms: i64, force: bool) {
    let mut next = (*state.onchain_monitor().snapshot()).clone();
    next.observed_at_ms = now_ms;
    publish_snapshot(state, &next, force);
}
