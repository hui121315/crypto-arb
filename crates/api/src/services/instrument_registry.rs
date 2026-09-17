//! 进程内 instrument 注册表（PR-BM/PR-AL 深度接线）：把 [`VenueInstrument`] 事实源
//! 接进后端——lifecycle 启动期从官方 endpoint 灌库，hedge 预检据此 **fail-closed** 算
//! 合法张数。按 `(venue, product, native_symbol)` 索引（venue/product 小写、symbol
//! 大写归一）精确命中，避免同名现货与永续规格互相覆盖。

use arc_swap::ArcSwap;
use dashmap::DashMap;
use shared_types::instrument_registry::{VenueInstrument, INSTRUMENT_SPEC_FRESHNESS_MS};
use shared_types::instruments::InstrumentListingStatus;
use shared_types::{ApiProblem, VenueOperationStatus};
use std::collections::{BTreeMap, BTreeSet};

mod coverage;
mod metadata;
mod persistence;
mod resolution;
mod snapshot;
mod spot_refresh;
mod transfer_loop;
pub(crate) use metadata::instrument_metadata_evidence;
use metadata::{
    fee_product_key, metadata_evidence_matches, normalized_native_symbol, normalized_product_key,
    venue_in_scope,
};
use resolution::{is_default_spot_contract, is_default_usdt_contract};
pub(crate) use resolution::{
    SpotInstrumentResolution, SpotInstrumentResolutionStatus, SpotRegistryState,
};
pub(crate) use spot_refresh::{refresh_spot_venue, request_spot_refresh};
pub(crate) use transfer_loop::CandidateTransferStatus;

pub(crate) const SUPPORTED_VENUES: &[&str] = &[
    "binance",
    "okx",
    "gate",
    "kucoin",
    "bybit",
    "bitget",
    "kraken",
    "gate_crossex",
    exchange::HyperliquidMarket::Core.venue(),
    exchange::HyperliquidMarket::XYZ.venue(),
];

pub(crate) const TRANSFER_SUPPORTED_VENUES: &[&str] = &[
    "binance", "okx", "bybit", "bitget", "gate", "kucoin", "kraken",
];

#[derive(Clone, Debug)]
enum VenueProbeState {
    Restored {
        checked_at_ms: i64,
    },
    Success {
        checked_at_ms: i64,
    },
    Failed {
        checked_at_ms: i64,
        problem: ApiProblem,
    },
    Unsupported {
        checked_at_ms: i64,
        problem: ApiProblem,
    },
}

#[derive(Clone, Debug)]
enum TransferProbeState {
    Refreshing {
        checked_at_ms: i64,
    },
    Success {
        checked_at_ms: i64,
    },
    Unavailable {
        checked_at_ms: i64,
        currencies: BTreeSet<String>,
        problem: ApiProblem,
    },
    Failed {
        checked_at_ms: i64,
        problem: ApiProblem,
    },
    Unsupported {
        checked_at_ms: i64,
        problem: ApiProblem,
    },
}

/// 按 `(venue, product, native_symbol)` 索引的内存 instrument 注册表。
#[derive(Debug)]
pub(crate) struct InstrumentRegistry {
    by_key: DashMap<(String, String, String), VenueInstrument>,
    probe_by_venue: DashMap<String, VenueProbeState>,
    spot_probe_by_venue: DashMap<String, VenueProbeState>,
    instrument_refresh_in_flight: DashMap<String, ()>,
    spot_instrument_refresh_in_flight: DashMap<String, ()>,
    instrument_lookup: ArcSwap<snapshot::InstrumentLookupSnapshot>,
    checkpoint_path: Option<std::path::PathBuf>,
    checkpoint_write_lock: tokio::sync::Mutex<()>,
    transfer_snapshot: ArcSwap<transfer_loop::TransferNetworkSnapshot>,
    transfer_probe_by_venue: DashMap<String, TransferProbeState>,
}

impl Default for InstrumentRegistry {
    fn default() -> Self {
        Self {
            by_key: DashMap::new(),
            probe_by_venue: DashMap::new(),
            spot_probe_by_venue: DashMap::new(),
            instrument_refresh_in_flight: DashMap::new(),
            spot_instrument_refresh_in_flight: DashMap::new(),
            instrument_lookup: ArcSwap::from_pointee(snapshot::InstrumentLookupSnapshot::default()),
            checkpoint_path: None,
            checkpoint_write_lock: tokio::sync::Mutex::new(()),
            transfer_snapshot: ArcSwap::from_pointee(
                transfer_loop::TransferNetworkSnapshot::default(),
            ),
            transfer_probe_by_venue: DashMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct InstrumentRegistryRuntimeHealth {
    pub venue: String,
    pub status: VenueOperationStatus,
    pub rows: usize,
    pub execution_ready_rows: usize,
    pub checked_at_ms: i64,
    pub freshness_ms: Option<i64>,
    pub schema_versions: Vec<String>,
    pub source_urls: Vec<String>,
    pub message: String,
    pub problem: Option<ApiProblem>,
}

impl InstrumentRegistry {
    fn key(instrument: &VenueInstrument) -> (String, String, String) {
        (
            instrument.venue.trim().to_ascii_lowercase(),
            normalized_product_key(instrument.product_type.as_deref()),
            instrument.native_symbol.trim().to_ascii_uppercase(),
        )
    }

    fn lookup_key(
        venue: &str,
        native_symbol: &str,
        product: shared_types::FeeProduct,
    ) -> Option<(String, String, String)> {
        Some((
            venue.trim().to_ascii_lowercase(),
            fee_product_key(product)?.to_owned(),
            native_symbol.trim().to_ascii_uppercase(),
        ))
    }

    /// fail-closed 写入：仅纳入结构有效条目；非法丢弃并返回 `Err`（不污染注册表）。
    #[cfg(test)]
    pub(crate) fn upsert(&self, instrument: VenueInstrument) -> Result<(), String> {
        if !instrument.is_structurally_valid() {
            return Err(format!(
                "rejected structurally-invalid instrument: {}/{}",
                instrument.venue, instrument.native_symbol
            ));
        }
        let venue = instrument.venue.clone();
        let checked_at_ms = instrument.checked_at_ms;
        let is_spot = normalized_product_key(instrument.product_type.as_deref()) == "spot";
        let key = Self::key(&instrument);
        self.by_key.insert(key, instrument);
        self.rebuild_instrument_lookup();
        self.record_refresh_success(&venue, checked_at_ms);
        if is_spot {
            self.record_spot_refresh_success(&venue, checked_at_ms);
        }
        Ok(())
    }

    /// 批次替换某 venue 全部有效条目；空/非法响应保留旧快照并返回 0。
    pub(crate) fn replace_venue(&self, venue: &str, instruments: Vec<VenueInstrument>) -> usize {
        let venue_key = venue.trim().to_ascii_lowercase();
        let evidence = instrument_metadata_evidence(&venue_key);
        let accepted = instruments
            .into_iter()
            .filter(|instrument| {
                venue_in_scope(&instrument.venue, &venue_key)
                    && instrument.is_structurally_valid()
                    && (evidence.is_some_and(|row| metadata_evidence_matches(instrument, row))
                        || exchange::official_instrument_evidence_matches(instrument))
            })
            .collect::<Vec<_>>();
        if accepted.is_empty() {
            return 0;
        }
        let checked_at_ms = accepted
            .iter()
            .map(|instrument| instrument.checked_at_ms)
            .max()
            .unwrap_or_else(common::time::now_ms);
        let has_spot = accepted
            .iter()
            .any(|instrument| normalized_product_key(instrument.product_type.as_deref()) == "spot");
        self.by_key
            .retain(|(v, _, _), _| !venue_in_scope(v, &venue_key));
        let accepted_count = accepted.len();
        for instrument in accepted {
            let key = Self::key(&instrument);
            self.by_key.insert(key, instrument);
        }
        self.rebuild_instrument_lookup();
        self.record_refresh_success(&venue_key, checked_at_ms);
        if has_spot {
            self.record_spot_refresh_success(&venue_key, checked_at_ms);
        }
        accepted_count
    }

    /// Replace only one venue's official Spot rows. Perpetual rows and their
    /// probe state stay untouched when on-chain comparison recovers Spot
    /// metadata independently.
    pub(crate) fn replace_spot_venue(
        &self,
        venue: &str,
        instruments: Vec<VenueInstrument>,
    ) -> usize {
        let venue_key = venue.trim().to_ascii_lowercase();
        let evidence = instrument_metadata_evidence(&venue_key);
        let accepted = instruments
            .into_iter()
            .filter(|instrument| {
                venue_in_scope(&instrument.venue, &venue_key)
                    && normalized_product_key(instrument.product_type.as_deref()) == "spot"
                    && instrument.is_structurally_valid()
                    && (evidence.is_some_and(|row| metadata_evidence_matches(instrument, row))
                        || exchange::official_instrument_evidence_matches(instrument))
            })
            .collect::<Vec<_>>();
        if accepted.is_empty() {
            return 0;
        }
        let checked_at_ms = accepted
            .iter()
            .map(|instrument| instrument.checked_at_ms)
            .max()
            .unwrap_or_else(common::time::now_ms);
        self.by_key.retain(|(stored_venue, product, _), _| {
            !venue_in_scope(stored_venue, &venue_key) || product != "spot"
        });
        let accepted_count = accepted.len();
        for instrument in accepted {
            self.by_key.insert(Self::key(&instrument), instrument);
        }
        self.rebuild_instrument_lookup();
        self.record_spot_refresh_success(&venue_key, checked_at_ms);
        accepted_count
    }

    /// 注册表条目总数（启动日志/诊断用）。
    pub(crate) fn len(&self) -> usize {
        self.by_key.len()
    }

    /// Return one venue family's official rows without exposing the mutable
    /// registry internals. Routed venues such as `gate_crossex:*` are included
    /// when their family name is requested.
    pub(crate) fn venue_instruments(&self, venue: &str) -> Vec<VenueInstrument> {
        let venue_key = venue.trim().to_ascii_lowercase();
        let mut rows = self
            .by_key
            .iter()
            .filter(|entry| venue_in_scope(&entry.key().0, &venue_key))
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        rows.sort_by(|left, right| left.native_symbol.cmp(&right.native_symbol));
        rows
    }

    /// Full public Spot WS universe, grouped by venue. This uses official
    /// instrument rows only as the symbol source; order-book depth remains an
    /// on-demand execution concern.
    pub(crate) fn spot_ws_requests_by_venue(&self) -> BTreeMap<String, Vec<String>> {
        let mut requests = BTreeMap::<String, Vec<String>>::new();
        for entry in &self.by_key {
            let instrument = entry.value();
            if normalized_product_key(instrument.product_type.as_deref()) != "spot"
                || instrument.listing_status != InstrumentListingStatus::Trading
            {
                continue;
            }
            let venue = instrument.venue.trim().to_ascii_lowercase();
            let symbol = instrument.native_symbol.trim().to_ascii_uppercase();
            // Routed/synthetic venues own a separate upstream protocol and do
            // not accept native exchange Spot symbols through the generic
            // venue adapter. Feeding them here creates thousands of requests
            // that can never produce a public WS row.
            if venue.is_empty() || venue.contains(':') || symbol.is_empty() {
                continue;
            }
            requests.entry(venue).or_default().push(symbol);
        }
        for symbols in requests.values_mut() {
            symbols.sort_unstable();
            symbols.dedup();
        }
        requests
    }

    /// 该 venue 是否有任何 instrument 覆盖：区分「未接线 venue」与「有 feed 缺该 symbol」。
    #[cfg(test)]
    pub(crate) fn has_venue(&self, venue: &str) -> bool {
        let venue_key = venue.trim().to_ascii_lowercase();
        self.by_key
            .iter()
            .any(|entry| venue_in_scope(&entry.key().0, &venue_key))
    }

    /// Whether the product has a first-class official instrument feed for the
    /// venue. This is intentionally independent from currently loaded rows so
    /// an empty/failed startup refresh cannot turn the Live sizing gate off.
    pub(crate) fn supports_venue(&self, venue: &str) -> bool {
        let venue = venue.trim();
        SUPPORTED_VENUES.iter().any(|candidate| {
            candidate.eq_ignore_ascii_case(venue)
                || (!candidate.contains(':')
                    && candidate.eq_ignore_ascii_case(shared_types::venue_family(venue)))
        })
    }

    fn probe_allows_execution(&self, venue: &str, now_ms: i64) -> bool {
        self.probe_state(venue).is_some_and(|state| {
            matches!(
                state,
                VenueProbeState::Success { checked_at_ms }
                    if now_ms >= checked_at_ms
                        && now_ms.saturating_sub(checked_at_ms)
                            < INSTRUMENT_SPEC_FRESHNESS_MS
            )
        })
    }

    fn probe_allows_product_execution(
        &self,
        venue: &str,
        product: shared_types::FeeProduct,
        now_ms: i64,
    ) -> bool {
        if product == shared_types::FeeProduct::Spot {
            self.spot_probe_allows_execution(venue, now_ms)
        } else {
            self.probe_allows_execution(venue, now_ms)
        }
    }

    fn spot_probe_allows_execution(&self, venue: &str, now_ms: i64) -> bool {
        self.spot_probe_state(venue).is_some_and(|state| {
            matches!(
                state,
                VenueProbeState::Success { checked_at_ms }
                    if now_ms >= checked_at_ms
                        && now_ms.saturating_sub(checked_at_ms)
                            < INSTRUMENT_SPEC_FRESHNESS_MS
            )
        })
    }

    fn probe_state(&self, venue: &str) -> Option<VenueProbeState> {
        let exact = venue.trim().to_ascii_lowercase();
        self.probe_by_venue
            .get(&exact)
            .map(|state| state.clone())
            .or_else(|| {
                let family = shared_types::venue_family(&exact);
                (family != exact && SUPPORTED_VENUES.contains(&family))
                    .then(|| self.probe_by_venue.get(family).map(|state| state.clone()))
                    .flatten()
            })
    }

    fn spot_probe_state(&self, venue: &str) -> Option<VenueProbeState> {
        let exact = venue.trim().to_ascii_lowercase();
        self.spot_probe_by_venue
            .get(&exact)
            .map(|state| state.clone())
            .or_else(|| {
                let family = shared_types::venue_family(&exact);
                (family != exact && SUPPORTED_VENUES.contains(&family))
                    .then(|| {
                        self.spot_probe_by_venue
                            .get(family)
                            .map(|state| state.clone())
                    })
                    .flatten()
            })
    }
}

#[cfg(test)]
mod tests;
