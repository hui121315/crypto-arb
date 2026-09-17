use super::{
    normalized_native_symbol, InstrumentRegistry, VenueInstrument, VenueProbeState,
    INSTRUMENT_SPEC_FRESHNESS_MS,
};
#[cfg(test)]
use shared_types::execution_sizing::{plan_leg_sizing, ExecutionSizingPlan, SizingBlock};
use shared_types::instruments::InstrumentListingStatus;
use shared_types::{ApiProblem, FeeProduct};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SpotRegistryState {
    Ready {
        checked_at_ms: i64,
    },
    Syncing,
    Stale {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SpotInstrumentResolutionStatus {
    Ready,
    Syncing,
    Unlisted,
    Incomplete,
    Stale,
    Unavailable,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SpotInstrumentResolution {
    pub status: SpotInstrumentResolutionStatus,
    pub instrument: Option<VenueInstrument>,
    pub checked_at_ms: Option<i64>,
    pub problem: Option<ApiProblem>,
}

impl InstrumentRegistry {
    /// Exact public lookup without granting trade readiness or scanning every venue row.
    pub(crate) fn public_native_instrument(&self, venue: &str, native_symbol: &str, product: FeeProduct) -> Option<VenueInstrument> {
        let key=Self::lookup_key(venue,native_symbol,product)?;
        let row=self.by_key.get(&key)?;
        (row.venue==venue && row.native_symbol==native_symbol && instrument_matches_product(&row,product)).then(||row.clone())
    }
    pub(crate) fn spot_registry_state(&self, venue: &str, now_ms: i64) -> SpotRegistryState {
        match self.spot_probe_state(venue) {
            Some(VenueProbeState::Restored { checked_at_ms }) => {
                SpotRegistryState::Stale { checked_at_ms }
            }
            Some(VenueProbeState::Success { checked_at_ms })
                if now_ms >= checked_at_ms
                    && now_ms.saturating_sub(checked_at_ms) < INSTRUMENT_SPEC_FRESHNESS_MS =>
            {
                SpotRegistryState::Ready { checked_at_ms }
            }
            Some(VenueProbeState::Success { checked_at_ms }) => {
                SpotRegistryState::Stale { checked_at_ms }
            }
            Some(VenueProbeState::Failed {
                checked_at_ms,
                problem,
            }) => SpotRegistryState::Failed {
                checked_at_ms,
                problem,
            },
            Some(VenueProbeState::Unsupported {
                checked_at_ms,
                problem,
            }) => SpotRegistryState::Unsupported {
                checked_at_ms,
                problem,
            },
            None => SpotRegistryState::Syncing,
        }
    }

    pub(crate) fn resolve_spot_instrument_evidence(
        &self,
        venue: &str,
        base: &str,
        quote: &str,
        now_ms: i64,
    ) -> SpotInstrumentResolution {
        let instrument = self.exact_spot_instrument(venue, base, quote);
        match self.spot_registry_state(venue, now_ms) {
            SpotRegistryState::Ready { checked_at_ms } => instrument.map_or(
                SpotInstrumentResolution {
                    status: SpotInstrumentResolutionStatus::Unlisted,
                    instrument: None,
                    checked_at_ms: Some(checked_at_ms),
                    problem: None,
                },
                |instrument| {
                    let status = if instrument
                        .is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS)
                    {
                        SpotInstrumentResolutionStatus::Ready
                    } else if !instrument.is_fresh_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS) {
                        SpotInstrumentResolutionStatus::Stale
                    } else {
                        SpotInstrumentResolutionStatus::Incomplete
                    };
                    SpotInstrumentResolution {
                        status,
                        checked_at_ms: Some(instrument.checked_at_ms),
                        instrument: Some(instrument),
                        problem: None,
                    }
                },
            ),
            SpotRegistryState::Syncing => SpotInstrumentResolution {
                status: SpotInstrumentResolutionStatus::Syncing,
                instrument,
                checked_at_ms: None,
                problem: None,
            },
            SpotRegistryState::Stale { checked_at_ms } => SpotInstrumentResolution {
                status: SpotInstrumentResolutionStatus::Stale,
                instrument,
                checked_at_ms: Some(checked_at_ms),
                problem: None,
            },
            SpotRegistryState::Failed {
                checked_at_ms,
                problem,
            } => SpotInstrumentResolution {
                status: SpotInstrumentResolutionStatus::Unavailable,
                instrument,
                checked_at_ms: Some(checked_at_ms),
                problem: Some(problem),
            },
            SpotRegistryState::Unsupported {
                checked_at_ms,
                problem,
            } => SpotInstrumentResolution {
                status: SpotInstrumentResolutionStatus::Unsupported,
                instrument,
                checked_at_ms: Some(checked_at_ms),
                problem: Some(problem),
            },
        }
    }

    fn exact_spot_instrument(
        &self,
        venue: &str,
        base: &str,
        quote: &str,
    ) -> Option<VenueInstrument> {
        let snapshot = self.instrument_lookup_snapshot();
        snapshot
            .rows(venue, base)
            .iter()
            .filter(|instrument| {
                instrument_matches_product(instrument, FeeProduct::Spot)
                    && instrument
                        .quote_asset
                        .as_deref()
                        .is_some_and(|asset| asset.eq_ignore_ascii_case(quote))
            })
            .max_by_key(|instrument| {
                (
                    instrument.has_official_provenance(),
                    instrument.listing_status == InstrumentListingStatus::Trading,
                    instrument.execution_supported,
                    instrument.checked_at_ms,
                )
            })
            .cloned()
    }

    /// `None` means the venue registry is not fresh enough to decide. `Some(false)`
    /// means a fresh official Spot snapshot exists but does not list this exact pair.
    pub(crate) fn exact_spot_listing_evidence(
        &self,
        venue: &str,
        base: &str,
        quote: &str,
        now_ms: i64,
    ) -> Option<bool> {
        if !self.spot_probe_allows_execution(venue, now_ms) {
            return None;
        }
        let snapshot = self.instrument_lookup_snapshot();
        Some(snapshot.rows(venue, base).iter().any(|instrument| {
            instrument_matches_product(instrument, FeeProduct::Spot)
                && instrument.listing_status == InstrumentListingStatus::Trading
                && instrument.has_official_provenance()
                && instrument.is_fresh_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS)
                && instrument
                    .quote_asset
                    .as_deref()
                    .is_some_and(|asset| asset.eq_ignore_ascii_case(quote))
        }))
    }

    /// Resolve an exact native symbol inside one product. Spot and perpetual
    /// instruments may share a native symbol, so the product is never inferred.
    pub(super) fn hedge_instrument_at_for_product(
        &self,
        venue: &str,
        native_symbol: &str,
        product: FeeProduct,
        now_ms: i64,
    ) -> Option<VenueInstrument> {
        if !self.probe_allows_product_execution(venue, product, now_ms) {
            return None;
        }
        let key = Self::lookup_key(venue, native_symbol, product)?;
        let entry = self.by_key.get(&key)?;
        if instrument_matches_product(&entry, product)
            && entry.is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS)
        {
            Some(entry.clone())
        } else {
            None
        }
    }

    /// Resolve an executable instrument by exact native symbol first. A bare
    /// canonical symbol uses the sole constructible contract, or the sole
    /// USDT-quoted/settled contract when a venue also lists another quote.
    pub(crate) fn resolve_hedge_instrument_for_product(
        &self,
        venue: &str,
        requested_symbol: &str,
        product: FeeProduct,
    ) -> Option<VenueInstrument> {
        let now_ms = common::time::now_ms();
        if let Some(instrument) =
            self.hedge_instrument_at_for_product(venue, requested_symbol, product, now_ms)
        {
            return Some(instrument);
        }
        if !self.probe_allows_product_execution(venue, product, now_ms) {
            return None;
        }
        let venue = venue.trim();
        let requested_symbol = requested_symbol.trim();
        let requested_native = normalized_native_symbol(requested_symbol);
        if !requested_native.is_empty() {
            let exact = self.by_key.iter().filter_map(|entry| {
                let instrument = entry.value();
                (instrument.venue.eq_ignore_ascii_case(venue)
                    && instrument_matches_product(instrument, product)
                    && normalized_native_symbol(&instrument.native_symbol) == requested_native
                    && instrument.is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS))
                .then(|| instrument.clone())
            });
            if let Some(instrument) = select_unique_instrument(exact.collect()) {
                return Some(instrument);
            }
        }
        let matches = self.by_key.iter().filter_map(|entry| {
            let instrument = entry.value();
            (instrument.venue.eq_ignore_ascii_case(venue)
                && instrument
                    .canonical_symbol
                    .eq_ignore_ascii_case(requested_symbol)
                && instrument_matches_product(instrument, product)
                && instrument.is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS))
            .then(|| instrument.clone())
        });
        select_preferred_instrument(matches.collect(), product)
    }

    /// Fail-closed sizing helper for registry contract tests.
    #[cfg(test)]
    pub(super) fn plan_leg_sizing_for_product(
        &self,
        venue: &str,
        native_symbol: &str,
        product: FeeProduct,
        target_notional_usd: f64,
        reference_price: f64,
    ) -> Result<ExecutionSizingPlan, SizingBlock> {
        let Some(instrument) =
            self.resolve_hedge_instrument_for_product(venue, native_symbol, product)
        else {
            return Err(SizingBlock::SpecMissing);
        };
        plan_leg_sizing(target_notional_usd, &instrument, reference_price)
    }
}

fn instrument_matches_product(instrument: &VenueInstrument, product: FeeProduct) -> bool {
    let product_type = instrument
        .product_type
        .as_deref()
        .unwrap_or_default()
        .trim();
    match product {
        FeeProduct::Spot => product_type.eq_ignore_ascii_case("spot"),
        FeeProduct::Perp => {
            product_type.eq_ignore_ascii_case("perp")
                || product_type.eq_ignore_ascii_case("perpetual")
        }
        FeeProduct::Margin | FeeProduct::Unknown => false,
    }
}

fn select_unique_instrument(mut matches: Vec<VenueInstrument>) -> Option<VenueInstrument> {
    if matches.len() == 1 {
        return matches.pop();
    }
    None
}

fn select_preferred_instrument(
    matches: Vec<VenueInstrument>,
    product: FeeProduct,
) -> Option<VenueInstrument> {
    if let Some(instrument) = select_unique_instrument(matches.clone()) {
        return Some(instrument);
    }
    let mut preferred = matches.into_iter().filter(|instrument| match product {
        FeeProduct::Perp => is_default_usdt_contract(instrument),
        FeeProduct::Spot => is_default_spot_contract(instrument),
        FeeProduct::Margin | FeeProduct::Unknown => false,
    });
    let instrument = preferred.next()?;
    preferred.next().is_none().then_some(instrument)
}

pub(super) fn is_default_spot_contract(instrument: &VenueInstrument) -> bool {
    instrument
        .quote_asset
        .as_deref()
        .is_some_and(|quote| quote.eq_ignore_ascii_case("USDT"))
}

pub(super) fn is_default_usdt_contract(instrument: &VenueInstrument) -> bool {
    instrument
        .quote_asset
        .as_deref()
        .is_some_and(|quote| quote.eq_ignore_ascii_case("USDT"))
        && instrument
            .settle_asset
            .as_deref()
            .is_some_and(|settle| settle.eq_ignore_ascii_case("USDT"))
}
