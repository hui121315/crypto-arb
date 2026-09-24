use arc_swap::ArcSwap;
use exchange::{Aggregator, PublicWsSnapshot};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use shared_types::{
    ApiProblem, ApiRecoveryAction, GateCrossExMode, GateCrossExModeConfig,
    GateCrossExModeConfigPatch, GateCrossExModeSnapshot, GateCrossExProduct,
    GateCrossExRouteCatalogResponse, GateCrossExRouteCatalogRow, GateCrossExRouteQuote,
    GateCrossExRuntimeState, GateCrossExSpreadCandidate, InstrumentListingStatus, VenueInstrument,
    GATE_CROSSEX_SELECTED_ROUTE_LIMIT,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

use super::instrument_registry::InstrumentRegistry;

const CHECKPOINT_VERSION: u32 = 1;
const SOURCE: &str = "gate_crossex_mode";
const ROUTE_CATALOG_LIMIT: usize = 80;

#[derive(Debug)]
pub(crate) struct GateCrossExModeService {
    path: Option<PathBuf>,
    config: ArcSwap<GateCrossExModeConfig>,
    snapshot: ArcSwap<GateCrossExModeSnapshot>,
    publication_lock: Mutex<()>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Checkpoint {
    version: u32,
    config: GateCrossExModeConfig,
}

#[derive(Debug, Error)]
pub(crate) enum GateCrossExModeError {
    #[error("Gate CrossEx 配置存储失败: {0}")]
    Io(#[from] std::io::Error),
    #[error("Gate CrossEx 配置序列化失败: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("{0}")]
    Validation(String),
}

impl GateCrossExModeService {
    pub(crate) fn load(path: impl Into<Option<PathBuf>>) -> Self {
        let path = path.into();
        let config = load_config(path.as_deref());
        let snapshot = GateCrossExModeSnapshot {
            config: config.clone(),
            ..Default::default()
        };
        Self {
            path,
            config: ArcSwap::from_pointee(config),
            snapshot: ArcSwap::from_pointee(snapshot),
            publication_lock: Mutex::new(()),
        }
    }

    pub(crate) fn snapshot(&self) -> Arc<GateCrossExModeSnapshot> {
        self.snapshot.load_full()
    }

    pub(crate) fn catalog(
        &self,
        registry: &InstrumentRegistry,
        search: Option<&str>,
        product: Option<GateCrossExProduct>,
        underlying_venue: Option<&str>,
        requested_limit: Option<usize>,
    ) -> GateCrossExRouteCatalogResponse {
        let query = search.map(normalized_search).unwrap_or_default();
        let venue = underlying_venue.map(normalized_search).unwrap_or_default();
        let all = catalog_rows(registry);
        let total = all
            .iter()
            .filter(|row| catalog_matches(row, &query, product, &venue))
            .count();
        let limit = requested_limit
            .unwrap_or(ROUTE_CATALOG_LIMIT)
            .clamp(1, ROUTE_CATALOG_LIMIT);
        let routes = all
            .into_iter()
            .filter(|row| catalog_matches(row, &query, product, &venue))
            .take(limit)
            .collect();
        GateCrossExRouteCatalogResponse {
            routes,
            total,
            generated_at_ms: common::time::now_ms(),
        }
    }

    pub(crate) fn update(
        &self,
        patch: GateCrossExModeConfigPatch,
        registry: &InstrumentRegistry,
    ) -> Result<GateCrossExModeConfig, GateCrossExModeError> {
        let _guard = self.publication_lock.lock();
        let mut next = (*self.config.load_full()).clone();
        if let Some(mode) = patch.mode {
            next.mode = mode;
        }
        if let Some(routes) = patch.selected_routes {
            next.selected_routes = validate_routes(routes, registry)?;
        }
        if let Some(minimum) = patch.min_gross_spread_pct {
            if !minimum.is_finite() || !(0.0..=100.0).contains(&minimum) {
                return Err(GateCrossExModeError::Validation(
                    "最小毛价差必须是 0% 到 100% 之间的有限数值".to_owned(),
                ));
            }
            next.min_gross_spread_pct = minimum;
        }
        persist(self.path.as_deref(), &next)?;
        self.config.store(Arc::new(next.clone()));
        let runtime_state = if next.mode == GateCrossExMode::Disabled {
            GateCrossExRuntimeState::Disabled
        } else {
            GateCrossExRuntimeState::Warming
        };
        let previous = self.snapshot.load_full();
        let snapshot = if next.mode == GateCrossExMode::Monitor
            && previous.config.mode == next.mode
            && previous.config.selected_routes == next.selected_routes
        {
            let mut snapshot = (*previous).clone();
            snapshot.config = next.clone();
            snapshot.candidates = spread_candidates(&snapshot.routes, next.min_gross_spread_pct);
            snapshot
        } else {
            base_snapshot(
                next.clone(),
                runtime_state,
                catalog_rows(registry).len(),
                common::time::now_ms(),
            )
        };
        self.snapshot.store(Arc::new(snapshot));
        Ok(next)
    }

    pub(crate) async fn refresh(
        &self,
        aggregator: &Aggregator,
        registry: &InstrumentRegistry,
        now_ms: i64,
    ) {
        let revision = self.config.load_full();
        let config = (*revision).clone();
        if config.mode == GateCrossExMode::Disabled {
            let current = self.snapshot.load();
            if current.runtime_state == GateCrossExRuntimeState::Disabled
                && current.config == config
                && current.observed_at_ms > 0
            {
                return;
            }
            let catalog_count = catalog_rows(registry).len();
            self.publish(
                &revision,
                base_snapshot(
                    config,
                    GateCrossExRuntimeState::Disabled,
                    catalog_count,
                    now_ms,
                ),
            );
            return;
        }
        let catalog_count = catalog_rows(registry).len();
        if catalog_count == 0 {
            let mut snapshot = base_snapshot(
                config,
                GateCrossExRuntimeState::WaitingForRegistry,
                0,
                now_ms,
            );
            snapshot.problem = Some(problem(
                "GATE_CROSSEX_REGISTRY_WARMING",
                "正在等待 Gate CrossEx 官方 instrument registry",
            ));
            self.publish(&revision, snapshot);
            return;
        }
        if config.selected_routes.is_empty() {
            self.publish(
                &revision,
                base_snapshot(
                    config,
                    GateCrossExRuntimeState::Warming,
                    catalog_count,
                    now_ms,
                ),
            );
            return;
        }
        let Some(adapter) = aggregator.get("gate_crossex") else {
            let mut snapshot = base_snapshot(
                config,
                GateCrossExRuntimeState::Degraded,
                catalog_count,
                now_ms,
            );
            snapshot.problem = Some(problem(
                "GATE_CROSSEX_ADAPTER_MISSING",
                "Gate CrossEx adapter 尚未注册",
            ));
            self.publish(&revision, snapshot);
            return;
        };
        let selected = config.selected_routes.clone();
        match adapter.public_ws_route_quote_snapshot(&selected).await {
            Ok(PublicWsSnapshot::Ready(routes)) if !routes.is_empty() => {
                self.publish(
                    &revision,
                    ready_snapshot(config, catalog_count, routes, now_ms),
                );
            }
            Ok(PublicWsSnapshot::Ready(_) | PublicWsSnapshot::Pending) => {
                self.publish(
                    &revision,
                    base_snapshot(
                        config,
                        GateCrossExRuntimeState::Warming,
                        catalog_count,
                        now_ms,
                    ),
                );
            }
            Ok(PublicWsSnapshot::Unsupported) => {
                let mut snapshot = base_snapshot(
                    config,
                    GateCrossExRuntimeState::Degraded,
                    catalog_count,
                    now_ms,
                );
                snapshot.problem = Some(problem(
                    "GATE_CROSSEX_WS_UNSUPPORTED",
                    "Gate CrossEx native route WebSocket 行情未接线",
                ));
                self.publish(&revision, snapshot);
            }
            Err(error) => {
                let mut snapshot = base_snapshot(
                    config,
                    GateCrossExRuntimeState::Degraded,
                    catalog_count,
                    now_ms,
                );
                snapshot.problem = Some(problem(
                    "GATE_CROSSEX_WS_FAILED",
                    format!("Gate CrossEx WebSocket 行情失败: {error}"),
                ));
                self.publish(&revision, snapshot);
            }
        }
    }

    fn publish(&self, revision: &Arc<GateCrossExModeConfig>, snapshot: GateCrossExModeSnapshot) {
        // A quote read may finish after a config write, including disable/enable.
        // Compare the allocation, not values, so an A -> B -> A edit is protected.
        let _guard = self.publication_lock.lock();
        if !Arc::ptr_eq(revision, &self.config.load_full()) {
            return;
        }
        self.snapshot.store(Arc::new(snapshot));
    }
}

fn load_config(path: Option<&Path>) -> GateCrossExModeConfig {
    match path.map_or(Ok(None), read_checkpoint) {
        Ok(Some(checkpoint)) => config_from_checkpoint(checkpoint),
        Ok(None) => GateCrossExModeConfig::default(),
        Err(error) => {
            tracing::warn!(error = %error, "Gate CrossEx checkpoint replay failed; using safe defaults");
            GateCrossExModeConfig::default()
        }
    }
}

fn config_from_checkpoint(checkpoint: Checkpoint) -> GateCrossExModeConfig {
    if checkpoint.version == CHECKPOINT_VERSION {
        return checkpoint.config;
    }
    tracing::warn!(
        version = checkpoint.version,
        "Gate CrossEx checkpoint version is unsupported; using safe defaults"
    );
    GateCrossExModeConfig::default()
}

fn base_snapshot(
    config: GateCrossExModeConfig,
    runtime_state: GateCrossExRuntimeState,
    catalog_count: usize,
    now_ms: i64,
) -> GateCrossExModeSnapshot {
    GateCrossExModeSnapshot {
        selected_count: config.selected_routes.len(),
        config,
        runtime_state,
        catalog_count,
        live_count: 0,
        routes: Vec::new(),
        candidates: Vec::new(),
        observed_at_ms: now_ms,
        problem: None,
    }
}

fn ready_snapshot(
    config: GateCrossExModeConfig,
    catalog_count: usize,
    mut routes: Vec<GateCrossExRouteQuote>,
    now_ms: i64,
) -> GateCrossExModeSnapshot {
    routes.sort_by(|left, right| left.native_symbol.cmp(&right.native_symbol));
    let live_count = routes.len();
    let candidates = spread_candidates(&routes, config.min_gross_spread_pct);
    let selected_count = config.selected_routes.len();
    let runtime_state = if live_count == selected_count {
        GateCrossExRuntimeState::Live
    } else {
        GateCrossExRuntimeState::Degraded
    };
    let problem = (live_count != selected_count).then(|| {
        problem(
            "GATE_CROSSEX_QUOTES_PARTIAL",
            format!("仅收到 {live_count}/{selected_count} 条新鲜 native route 行情"),
        )
    });
    GateCrossExModeSnapshot {
        config,
        runtime_state,
        catalog_count,
        selected_count,
        live_count,
        routes,
        candidates,
        observed_at_ms: now_ms,
        problem,
    }
}

fn spread_candidates(
    routes: &[GateCrossExRouteQuote],
    minimum_pct: f64,
) -> Vec<GateCrossExSpreadCandidate> {
    let mut candidates = Vec::new();
    for (index, left) in routes.iter().enumerate() {
        for right in routes.iter().skip(index + 1) {
            if !same_market(left, right) || left.underlying_venue == right.underlying_venue {
                continue;
            }
            push_candidate(&mut candidates, left, right, minimum_pct);
            push_candidate(&mut candidates, right, left, minimum_pct);
        }
    }
    candidates.sort_by(|left, right| {
        right
            .gross_spread_pct
            .total_cmp(&left.gross_spread_pct)
            .then_with(|| left.long_route.cmp(&right.long_route))
            .then_with(|| left.short_route.cmp(&right.short_route))
    });
    candidates
}

fn push_candidate(
    candidates: &mut Vec<GateCrossExSpreadCandidate>,
    long: &GateCrossExRouteQuote,
    short: &GateCrossExRouteQuote,
    minimum_pct: f64,
) {
    if !long.ask.is_finite() || !short.bid.is_finite() || long.ask <= 0.0 || short.bid <= long.ask {
        return;
    }
    let gross_spread_pct = (short.bid - long.ask) / long.ask * 100.0;
    if gross_spread_pct < minimum_pct {
        return;
    }
    candidates.push(GateCrossExSpreadCandidate {
        product: long.product,
        base_asset: long.base_asset.clone(),
        quote_asset: long.quote_asset.clone(),
        long_route: long.native_symbol.clone(),
        short_route: short.native_symbol.clone(),
        long_ask: long.ask,
        short_bid: short.bid,
        gross_spread_pct,
        synchronized_at_ms: long.observed_at_ms.min(short.observed_at_ms),
    });
}

fn same_market(left: &GateCrossExRouteQuote, right: &GateCrossExRouteQuote) -> bool {
    left.product == right.product
        && left.base_asset == right.base_asset
        && left.quote_asset == right.quote_asset
}

fn validate_routes(
    routes: Vec<String>,
    registry: &InstrumentRegistry,
) -> Result<Vec<String>, GateCrossExModeError> {
    let selected = routes
        .into_iter()
        .map(|route| route.trim().to_ascii_uppercase())
        .filter(|route| !route.is_empty())
        .collect::<BTreeSet<_>>();
    if selected.len() > GATE_CROSSEX_SELECTED_ROUTE_LIMIT {
        return Err(GateCrossExModeError::Validation(format!(
            "最多选择 {GATE_CROSSEX_SELECTED_ROUTE_LIMIT} 条 Gate CrossEx native route"
        )));
    }
    let catalog = catalog_rows(registry)
        .into_iter()
        .map(|row| (row.native_symbol.clone(), row))
        .collect::<BTreeMap<_, _>>();
    for route in &selected {
        let Some(row) = catalog.get(route) else {
            return Err(GateCrossExModeError::Validation(format!(
                "{route} 不在 Gate CrossEx 官方 instrument registry 中"
            )));
        };
        if row.listing_status != InstrumentListingStatus::Trading {
            return Err(GateCrossExModeError::Validation(format!(
                "{route} 当前不是可交易状态"
            )));
        }
    }
    Ok(selected.into_iter().collect())
}

fn catalog_rows(registry: &InstrumentRegistry) -> Vec<GateCrossExRouteCatalogRow> {
    registry
        .venue_instruments("gate_crossex")
        .into_iter()
        .filter_map(catalog_row)
        .collect()
}

fn catalog_row(instrument: VenueInstrument) -> Option<GateCrossExRouteCatalogRow> {
    let product = match instrument.product_type.as_deref() {
        Some("spot") => GateCrossExProduct::Spot,
        Some("perp") => GateCrossExProduct::Future,
        _ => return None,
    };
    let underlying_venue = instrument
        .venue
        .split_once(':')
        .map(|(_, route)| route.to_owned())?;
    Some(GateCrossExRouteCatalogRow {
        native_symbol: instrument.native_symbol,
        underlying_venue,
        product,
        base_asset: instrument.canonical_symbol,
        quote_asset: instrument.quote_asset?,
        display_symbol: instrument.display_symbol,
        execution_supported: instrument.execution_supported,
        listing_status: instrument.listing_status,
        source_url: instrument.source_url,
    })
}

fn catalog_matches(
    row: &GateCrossExRouteCatalogRow,
    query: &str,
    product: Option<GateCrossExProduct>,
    venue: &str,
) -> bool {
    product.is_none_or(|product| row.product == product)
        && (venue.is_empty() || row.underlying_venue.eq_ignore_ascii_case(venue))
        && (query.is_empty()
            || row.native_symbol.to_ascii_uppercase().contains(query)
            || row.base_asset.to_ascii_uppercase().contains(query)
            || row.quote_asset.to_ascii_uppercase().contains(query)
            || row.underlying_venue.to_ascii_uppercase().contains(query))
}

fn normalized_search(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn problem(code: &str, message: impl Into<String>) -> ApiProblem {
    ApiProblem::new(code, message)
        .with_source(SOURCE)
        .with_recovery_action(ApiRecoveryAction::CheckRuntimeHealth)
}

fn read_checkpoint(path: &Path) -> Result<Option<Checkpoint>, GateCrossExModeError> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn persist(
    path: Option<&Path>,
    config: &GateCrossExModeConfig,
) -> Result<(), GateCrossExModeError> {
    let Some(path) = path else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let checkpoint = Checkpoint {
        version: CHECKPOINT_VERSION,
        config: config.clone(),
    };
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temp, serde_json::to_vec_pretty(&checkpoint)?)?;
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quote(route: &str, venue: &str, bid: f64, ask: f64) -> GateCrossExRouteQuote {
        GateCrossExRouteQuote {
            native_symbol: route.to_owned(),
            underlying_venue: venue.to_owned(),
            product: GateCrossExProduct::Future,
            base_asset: "BTC".to_owned(),
            quote_asset: "USDT".to_owned(),
            bid,
            ask,
            last: (bid + ask) / 2.0,
            observed_at_ms: 1_700_000_000_000,
        }
    }

    #[test]
    fn spread_uses_executable_ask_to_bid_direction_and_percent_units() {
        let rows = [
            quote("BINANCE_FUTURE_BTC_USDT", "binance", 100.0, 100.1),
            quote("OKX_FUTURE_BTC_USDT", "okx", 100.5, 100.6),
        ];

        let candidates = spread_candidates(&rows, 0.30);

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].long_route, "BINANCE_FUTURE_BTC_USDT");
        assert_eq!(candidates[0].short_route, "OKX_FUTURE_BTC_USDT");
        assert!((candidates[0].gross_spread_pct - 0.3996003996).abs() < 1e-9);
    }

    #[test]
    fn spread_never_compares_different_quote_assets() {
        let left = quote("KRAKEN_FUTURE_BTC_USD", "kraken", 100.0, 100.1);
        let mut right = quote("OKX_FUTURE_BTC_USDT", "okx", 101.0, 101.1);
        right.quote_asset = "USDT".to_owned();
        let mut left = left;
        left.quote_asset = "USD".to_owned();

        assert!(spread_candidates(&[left, right], 0.0).is_empty());
    }

    #[tokio::test]
    async fn disabled_refresh_keeps_the_initial_snapshot_stable() {
        let service = GateCrossExModeService::load(None);
        let aggregator = Aggregator::new();
        let registry = InstrumentRegistry::default();

        service.refresh(&aggregator, &registry, 1_000).await;
        service.refresh(&aggregator, &registry, 2_000).await;

        let snapshot = service.snapshot();
        assert_eq!(snapshot.runtime_state, GateCrossExRuntimeState::Disabled);
        assert_eq!(snapshot.observed_at_ms, 1_000);
    }

    #[test]
    fn delayed_refresh_cannot_restore_old_config_even_after_an_aba_edit() {
        let service = GateCrossExModeService::load(None);
        let registry = InstrumentRegistry::default();
        let revision = service.config.load_full();
        let old = base_snapshot((*revision).clone(), GateCrossExRuntimeState::Live, 2, 1_000);
        for mode in [GateCrossExMode::Monitor, GateCrossExMode::Disabled] {
            service
                .update(
                    GateCrossExModeConfigPatch {
                        mode: Some(mode),
                        ..Default::default()
                    },
                    &registry,
                )
                .unwrap();
        }
        service.publish(&revision, old);
        assert_eq!(
            service.snapshot().runtime_state,
            GateCrossExRuntimeState::Disabled
        );
        let current = service.config.load_full();
        service.publish(
            &current,
            base_snapshot(
                (*current).clone(),
                GateCrossExRuntimeState::Disabled,
                5,
                2_000,
            ),
        );
        assert_eq!(service.snapshot().catalog_count, 5);
    }

    #[test]
    fn threshold_edit_reuses_quotes_without_refreshing_their_evidence_time() {
        let service = GateCrossExModeService::load(None);
        let registry = InstrumentRegistry::default();
        service
            .update(
                GateCrossExModeConfigPatch {
                    mode: Some(GateCrossExMode::Monitor),
                    ..Default::default()
                },
                &registry,
            )
            .unwrap();
        let revision = service.config.load_full();
        let quotes = vec![
            quote("BINANCE_FUTURE_BTC_USDT", "binance", 100.0, 100.1),
            quote("OKX_FUTURE_BTC_USDT", "okx", 100.5, 100.6),
        ];
        service.publish(
            &revision,
            ready_snapshot((*revision).clone(), 2, quotes, 5_000),
        );
        service
            .update(
                GateCrossExModeConfigPatch {
                    min_gross_spread_pct: Some(1.0),
                    ..Default::default()
                },
                &registry,
            )
            .unwrap();
        let snapshot = service.snapshot();
        assert_eq!(snapshot.routes.len(), 2);
        assert!(snapshot.candidates.is_empty());
        assert_eq!(snapshot.observed_at_ms, 5_000);
        assert_eq!(snapshot.routes[0].observed_at_ms, 1_700_000_000_000);
    }
}
