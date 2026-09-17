use super::super::{
    is_default_spot_contract, is_default_usdt_contract, normalized_native_symbol,
    snapshot::InstrumentLookupSnapshot,
};
use super::{
    executable_venue, listing_blocker, normalize_canonical_symbol, InstrumentRegistry,
    LISTING_BLOCKER_PREFIX, SUPPORTED_VENUES,
};
use shared_types::instrument_coverage::InstrumentCoverageEntry;
use shared_types::instrument_registry::{VenueInstrument, INSTRUMENT_SPEC_FRESHNESS_MS};
use shared_types::{
    p0_hedge_leg_product, ArbitrageOpportunityDto, FeeProduct, HedgeLegRole, StrategyKind,
};
use std::sync::Arc;

mod identity;

use identity::economic_identity_blocker;

#[derive(Clone, Copy, PartialEq, Eq)]
enum RequiredProduct {
    Perp,
    Spot,
}

struct ExecutionGateIndex {
    instruments: Arc<InstrumentLookupSnapshot>,
}

impl ExecutionGateIndex {
    fn snapshot(registry: &InstrumentRegistry) -> Self {
        Self {
            instruments: registry.instrument_lookup_snapshot(),
        }
    }

    fn resolve(
        &self,
        venue: &str,
        canonical_symbol: &str,
        product: RequiredProduct,
        market_symbol: Option<&str>,
        now_ms: i64,
    ) -> Option<&VenueInstrument> {
        let matches = self
            .instruments
            .rows(venue, canonical_symbol)
            .iter()
            .filter(|instrument| {
                product.matches(instrument.product_type.as_deref())
                    && instrument.is_hedge_constructible_at(now_ms, INSTRUMENT_SPEC_FRESHNESS_MS)
            })
            .collect::<Vec<_>>();
        if product == RequiredProduct::Spot {
            if let Some(market_symbol) = market_symbol {
                let requested = normalized_native_symbol(market_symbol);
                let mut exact = matches.iter().copied().filter(|instrument| {
                    !requested.is_empty()
                        && normalized_native_symbol(&instrument.native_symbol) == requested
                });
                let instrument = exact.next()?;
                return exact.next().is_none().then_some(instrument);
            }
        }
        if matches.len() == 1 {
            return matches.first().copied();
        }
        let mut preferred = matches.into_iter().filter(|instrument| match product {
            RequiredProduct::Perp => is_default_usdt_contract(instrument),
            RequiredProduct::Spot => is_default_spot_contract(instrument),
        });
        let instrument = preferred.next()?;
        preferred.next().is_none().then_some(instrument)
    }

    fn coverage(
        &self,
        registry: &InstrumentRegistry,
        canonical_symbol: &str,
        now_ms: i64,
    ) -> InstrumentCoverageEntry {
        let canonical_symbol = normalize_canonical_symbol(canonical_symbol);
        let venues = SUPPORTED_VENUES
            .iter()
            .map(|venue| {
                let candidates = self.instruments.rows(venue, &canonical_symbol);
                registry.venue_coverage_with_candidates(
                    venue,
                    &canonical_symbol,
                    now_ms,
                    Some(candidates),
                )
            })
            .collect();
        InstrumentCoverageEntry {
            canonical_symbol,
            venues,
        }
    }
}

impl RequiredProduct {
    const fn label(self) -> &'static str {
        match self {
            Self::Perp => "永续",
            Self::Spot => "现货",
        }
    }

    fn matches(self, product_type: Option<&str>) -> bool {
        let product_type = product_type.unwrap_or_default().trim();
        match self {
            Self::Perp => {
                product_type.eq_ignore_ascii_case("perp")
                    || product_type.eq_ignore_ascii_case("perpetual")
            }
            Self::Spot => product_type.eq_ignore_ascii_case("spot"),
        }
    }
}

pub(super) fn apply(
    registry: &InstrumentRegistry,
    opportunities: &mut [ArbitrageOpportunityDto],
    now_ms: i64,
) {
    let index = ExecutionGateIndex::snapshot(registry);
    for opportunity in opportunities {
        if let Some(blocker) = opportunity_gate_blocker(registry, &index, opportunity, now_ms) {
            opportunity.execution_eligible = false;
            if !opportunity.execution_blockers.contains(&blocker) {
                opportunity.execution_blockers.insert(0, blocker);
            }
        }
    }
}

fn opportunity_gate_blocker(
    registry: &InstrumentRegistry,
    index: &ExecutionGateIndex,
    opportunity: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> Option<String> {
    let Some((long_product, short_product)) = required_leg_products(opportunity) else {
        if matches!(
            opportunity.strategy_kind,
            Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp)
        ) {
            return Some(format!(
                "{LISTING_BLOCKER_PREFIX}现货腿方向证据缺失，无法确认双腿执行产品，仅观察"
            ));
        }
        let coverage = index.coverage(registry, &opportunity.symbol, now_ms);
        return legacy_listing_blocker(&coverage, opportunity, now_ms);
    };
    let long = execution_instrument(
        registry,
        index,
        &opportunity.long_exchange,
        long_product,
        (
            &opportunity.symbol,
            opportunity
                .long_leg_market_evidence
                .as_ref()
                .map(|evidence| evidence.symbol.as_str()),
        ),
        now_ms,
    );
    let short = execution_instrument(
        registry,
        index,
        &opportunity.short_exchange,
        short_product,
        (
            &opportunity.symbol,
            opportunity
                .short_leg_market_evidence
                .as_ref()
                .map(|evidence| evidence.symbol.as_str()),
        ),
        now_ms,
    );
    match (long.as_ref(), short.as_ref()) {
        (Some(long), Some(short)) => economic_identity_blocker(opportunity, long, short),
        _ if long_product == RequiredProduct::Spot || short_product == RequiredProduct::Spot => {
            Some(missing_product_blocker(
                opportunity,
                long_product,
                long.is_none(),
                short_product,
                short.is_none(),
            ))
        }
        _ => {
            let coverage = index.coverage(registry, &opportunity.symbol, now_ms);
            legacy_listing_blocker(&coverage, opportunity, now_ms)
        }
    }
}

fn execution_instrument<'a>(
    registry: &InstrumentRegistry,
    index: &'a ExecutionGateIndex,
    venue: &str,
    product: RequiredProduct,
    symbols: (&str, Option<&str>),
    now_ms: i64,
) -> Option<&'a VenueInstrument> {
    if !registry.probe_allows_execution(venue, now_ms) {
        return None;
    }
    index.resolve(venue, symbols.0, product, symbols.1, now_ms)
}

fn required_leg_products(
    opportunity: &ArbitrageOpportunityDto,
) -> Option<(RequiredProduct, RequiredProduct)> {
    Some((
        required_product(p0_hedge_leg_product(
            opportunity.strategy_kind,
            opportunity.spot_leg_mode,
            HedgeLegRole::Long,
        )?)?,
        required_product(p0_hedge_leg_product(
            opportunity.strategy_kind,
            opportunity.spot_leg_mode,
            HedgeLegRole::Short,
        )?)?,
    ))
}

const fn required_product(product: FeeProduct) -> Option<RequiredProduct> {
    match product {
        FeeProduct::Perp => Some(RequiredProduct::Perp),
        FeeProduct::Spot => Some(RequiredProduct::Spot),
        FeeProduct::Margin | FeeProduct::Unknown => None,
    }
}

fn legacy_listing_blocker(
    coverage: &InstrumentCoverageEntry,
    opportunity: &ArbitrageOpportunityDto,
    now_ms: i64,
) -> Option<String> {
    let long_ready = executable_venue(coverage, &opportunity.long_exchange, now_ms);
    let short_ready = executable_venue(coverage, &opportunity.short_exchange, now_ms);
    (!(long_ready && short_ready)).then(|| {
        listing_blocker(
            coverage,
            &opportunity.long_exchange,
            &opportunity.short_exchange,
            now_ms,
        )
    })
}

fn missing_product_blocker(
    opportunity: &ArbitrageOpportunityDto,
    long_product: RequiredProduct,
    long_missing: bool,
    short_product: RequiredProduct,
    short_missing: bool,
) -> String {
    let mut missing = Vec::with_capacity(2);
    if long_missing {
        missing.push(format!(
            "{} 缺少官方{}执行规格",
            opportunity.long_exchange.to_ascii_uppercase(),
            long_product.label()
        ));
    }
    if short_missing {
        missing.push(format!(
            "{} 缺少官方{}执行规格",
            opportunity.short_exchange.to_ascii_uppercase(),
            short_product.label()
        ));
    }
    format!("{LISTING_BLOCKER_PREFIX}{}，仅观察", missing.join("；"))
}
