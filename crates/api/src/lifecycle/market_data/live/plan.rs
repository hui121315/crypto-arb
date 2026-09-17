use super::*;
use shared_types::{ArbitrageOpportunityDto, PositionRow, SpotLegMode, StrategyKind, TickerInfo};

mod capacity;
mod positions;
mod symbols;
#[cfg(test)]
mod test_helpers;

use capacity::{leg_pair_has_capacity, spot_requests_have_capacity, LegMarket};
use positions::{append_position_requests, WS_LIVE_POSITION_MARK_SYMBOLS_PER_VENUE};
use symbols::{
    leg_request_symbol, live_request_venue, opportunity_leg_markets, push_funding_symbol,
    push_leg_with_limit, push_market_symbol_with_limit,
};
#[cfg(test)]
use test_helpers::{push_leg, push_market_symbol};

pub(super) const WS_LIVE_PERP_CROSS_ROWS: usize = 50;
pub(super) const WS_LIVE_OTHER_ROWS: usize = 24;
pub(super) const WS_LIVE_SYMBOLS_PER_VENUE: usize = 16;
const WS_LIVE_OTHER_SYMBOLS_PER_VENUE: usize = 4;
const WS_LIVE_QUOTE_CONVERSION_SYMBOLS_PER_VENUE: usize = 2;
const WS_LIVE_TOTAL_SYMBOLS_PER_VENUE: usize =
    WS_LIVE_SYMBOLS_PER_VENUE + WS_LIVE_OTHER_SYMBOLS_PER_VENUE;
const WS_LIVE_FUNDING_SYMBOLS_PER_VENUE: usize = 32;
const WS_DISCOVERY_BOOTSTRAP_SYMBOLS: [&str; 3] = ["BTC", "ETH", "SOL"];
const WS_LIVE_OTHER_STRATEGIES: [StrategyKind; 4] = [
    StrategyKind::PerpPriceSpread,
    StrategyKind::SpotPerp,
    StrategyKind::CrossSpotPerp,
    StrategyKind::SpotCross,
];

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct LiveRequestPlan {
    pub(super) perp: BTreeMap<String, Vec<String>>,
    pub(super) spot: BTreeMap<String, Vec<String>>,
    pub(super) funding: BTreeMap<String, Vec<String>>,
    pub(super) marks: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct VersionedOpportunityPlan {
    pub(super) version: u64,
    pub(super) requests: LiveRequestPlan,
}

impl LiveRequestPlan {
    pub(super) fn is_empty(&self) -> bool {
        self.perp.is_empty()
            && self.spot.is_empty()
            && self.funding.is_empty()
            && self.marks.is_empty()
    }
}

impl VersionedOpportunityPlan {
    pub(super) fn replace(&mut self, next: Self) -> bool {
        let changed = self.requests != next.requests;
        self.version = next.version;
        if changed {
            self.requests = next.requests;
        }
        changed
    }
}

pub(super) fn live_request_plan(
    state: &AppState,
    opportunity_plan: &LiveRequestPlan,
) -> LiveRequestPlan {
    let mut plan = LiveRequestPlan::default();
    if let Some(entry) = state.portfolio_snapshot().get_arc_now() {
        append_position_requests(&mut plan, &entry.value.positions);
    }
    append_plan(&mut plan, opportunity_plan);
    plan
}

pub(super) fn opportunity_request_plan(state: &AppState) -> VersionedOpportunityPlan {
    let mut requests = LiveRequestPlan::default();
    let version = if let Some(view) = state.opportunity_index().read() {
        append_opportunity_requests(&mut requests, view.rows());
        view.version()
    } else {
        state.opportunity_index().version()
    };
    if requests.is_empty() {
        let market = state.market_data().market_snapshot_cached();
        append_discovery_perp_requests(&mut requests, market.perp_tickers.as_ref());
        if requests.is_empty() {
            append_bootstrap_perp_requests(&mut requests);
        }
    }
    VersionedOpportunityPlan { version, requests }
}

fn append_bootstrap_perp_requests(plan: &mut LiveRequestPlan) {
    for venue in WS_PREWARM_VENUES
        .iter()
        .filter(|venue| **venue != VenueId::GateCrossEx)
    {
        for symbol in WS_DISCOVERY_BOOTSTRAP_SYMBOLS {
            push_market_symbol_with_limit(
                &mut plan.perp,
                venue.as_str(),
                symbol,
                WS_DISCOVERY_BOOTSTRAP_SYMBOLS.len(),
            );
        }
    }
}

fn append_discovery_perp_requests(plan: &mut LiveRequestPlan, rows: &[TickerInfo]) {
    let mut by_symbol = BTreeMap::<String, BTreeMap<String, f64>>::new();
    for row in rows {
        let Some(venue) = live_request_venue(&row.exchange) else {
            continue;
        };
        let symbol = row.symbol.trim().to_ascii_uppercase();
        if symbol.is_empty() {
            continue;
        }
        let volume = if row.volume_24h.is_finite() {
            row.volume_24h.max(0.0)
        } else {
            0.0
        };
        by_symbol
            .entry(symbol)
            .or_default()
            .entry(venue)
            .and_modify(|current| *current = current.max(volume))
            .or_insert(volume);
    }

    let mut candidates = by_symbol
        .into_iter()
        .filter(|(_, venues)| venues.len() >= 2)
        .map(|(symbol, venues)| {
            let venue_count = venues.len();
            let min_volume = venues.values().copied().fold(f64::INFINITY, f64::min);
            (symbol, venues, venue_count, min_volume)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .2
            .cmp(&left.2)
            .then_with(|| right.3.total_cmp(&left.3))
            .then_with(|| left.0.cmp(&right.0))
    });

    for (symbol, venues, _, _) in candidates {
        let eligible = venues
            .keys()
            .filter(|venue| perp_request_has_capacity(plan, venue, &symbol))
            .cloned()
            .collect::<Vec<_>>();
        if eligible.len() < 2 {
            continue;
        }
        for venue in eligible {
            push_leg_with_limit(
                plan,
                LegMarket::Perp,
                &venue,
                &symbol,
                WS_LIVE_SYMBOLS_PER_VENUE,
            );
        }
    }
}

fn perp_request_has_capacity(plan: &LiveRequestPlan, venue: &str, symbol: &str) -> bool {
    plan.perp.get(venue).is_none_or(|symbols| {
        symbols.iter().any(|current| current == symbol) || symbols.len() < WS_LIVE_SYMBOLS_PER_VENUE
    })
}

fn append_plan(plan: &mut LiveRequestPlan, source: &LiveRequestPlan) {
    for (venue, symbols) in &source.perp {
        for symbol in symbols {
            push_market_symbol_with_limit(
                &mut plan.perp,
                venue,
                symbol,
                WS_LIVE_TOTAL_SYMBOLS_PER_VENUE,
            );
        }
    }
    for (venue, symbols) in &source.spot {
        for symbol in symbols {
            push_market_symbol_with_limit(
                &mut plan.spot,
                venue,
                symbol,
                WS_LIVE_TOTAL_SYMBOLS_PER_VENUE,
            );
        }
    }
    for (venue, symbols) in &source.funding {
        for symbol in symbols {
            push_funding_symbol(&mut plan.funding, venue, symbol);
        }
    }
    for (venue, symbols) in &source.marks {
        for symbol in symbols {
            push_market_symbol_with_limit(
                &mut plan.marks,
                venue,
                symbol,
                WS_LIVE_POSITION_MARK_SYMBOLS_PER_VENUE,
            );
        }
    }
}

fn append_opportunity_requests(plan: &mut LiveRequestPlan, rows: &[ArbitrageOpportunityDto]) {
    for row in rows
        .iter()
        .filter(|row| {
            row.strategy_kind == Some(StrategyKind::PerpCross) && is_positive_candidate(row)
        })
        .take(WS_LIVE_PERP_CROSS_ROWS)
    {
        append_opportunity_request(plan, row, WS_LIVE_SYMBOLS_PER_VENUE);
    }
    let mut other_strategy_plan = LiveRequestPlan::default();
    append_other_strategy_requests(&mut other_strategy_plan, rows);
    append_plan(plan, &other_strategy_plan);
    append_funding_prewarm_requests(plan, rows);
}

fn append_funding_prewarm_requests(plan: &mut LiveRequestPlan, rows: &[ArbitrageOpportunityDto]) {
    for row in rows.iter().filter(|row| {
        is_positive_candidate(row)
            && matches!(
                row.strategy_kind,
                Some(
                    StrategyKind::PerpCross
                        | StrategyKind::PerpPriceSpread
                        | StrategyKind::SpotPerp
                        | StrategyKind::CrossSpotPerp
                )
            )
    }) {
        let (long_market, short_market) =
            opportunity_leg_markets(row.strategy_kind, row.spot_leg_mode);
        push_funding_leg(plan, long_market, &row.long_exchange, &row.symbol);
        push_funding_leg(plan, short_market, &row.short_exchange, &row.symbol);
    }
}

fn push_funding_leg(plan: &mut LiveRequestPlan, market: LegMarket, venue: &str, symbol: &str) {
    if market != LegMarket::Perp {
        return;
    }
    let Some(venue) = live_request_venue(venue) else {
        return;
    };
    push_funding_symbol(&mut plan.funding, &venue, symbol);
}

fn append_other_strategy_requests(plan: &mut LiveRequestPlan, rows: &[ArbitrageOpportunityDto]) {
    let mut buckets: [Vec<&ArbitrageOpportunityDto>; 4] = std::array::from_fn(|_| Vec::new());
    for row in rows {
        if !is_positive_candidate(row) {
            continue;
        }
        let Some(index) = row.strategy_kind.and_then(|strategy| {
            WS_LIVE_OTHER_STRATEGIES
                .iter()
                .position(|item| *item == strategy)
        }) else {
            continue;
        };
        if buckets[index].len() < WS_LIVE_OTHER_ROWS {
            buckets[index].push(row);
        }
    }
    let mut considered = 0;
    let mut rank = 0;
    while considered < WS_LIVE_OTHER_ROWS {
        let mut progressed = false;
        for bucket in &buckets {
            let Some(row) = bucket.get(rank) else {
                continue;
            };
            append_opportunity_request(plan, row, WS_LIVE_OTHER_SYMBOLS_PER_VENUE);
            considered += 1;
            progressed = true;
            if considered == WS_LIVE_OTHER_ROWS {
                break;
            }
        }
        if !progressed {
            break;
        }
        rank += 1;
    }
}

fn is_positive_candidate(row: &ArbitrageOpportunityDto) -> bool {
    row.net_single_yield.is_finite() && row.net_single_yield > f64::EPSILON
}

fn append_opportunity_request(
    plan: &mut LiveRequestPlan,
    row: &ArbitrageOpportunityDto,
    symbol_limit: usize,
) {
    let (long_market, short_market) = opportunity_leg_markets(row.strategy_kind, row.spot_leg_mode);
    let Some(long_symbol) = leg_request_symbol(
        long_market,
        &row.symbol,
        row.long_leg_market_evidence
            .as_ref()
            .map(|evidence| evidence.symbol.as_str()),
    ) else {
        return;
    };
    let Some(short_symbol) = leg_request_symbol(
        short_market,
        &row.symbol,
        row.short_leg_market_evidence
            .as_ref()
            .map(|evidence| evidence.symbol.as_str()),
    ) else {
        return;
    };
    let Some(long_venue) = live_request_venue(&row.long_exchange) else {
        return;
    };
    let Some(short_venue) = live_request_venue(&row.short_exchange) else {
        return;
    };
    if long_symbol.trim().is_empty() || short_symbol.trim().is_empty() {
        return;
    }
    if !leg_pair_has_capacity(
        plan,
        (long_market, &long_venue, long_symbol),
        (short_market, &short_venue, short_symbol),
        symbol_limit,
    ) {
        return;
    }
    let conversion_limit = symbol_limit
        .saturating_add(WS_LIVE_QUOTE_CONVERSION_SYMBOLS_PER_VENUE)
        .min(WS_LIVE_TOTAL_SYMBOLS_PER_VENUE);
    let mut spot_requests = Vec::with_capacity(row.quote_conversions.len().saturating_add(2));
    if long_market == LegMarket::Spot {
        spot_requests.push((long_venue.clone(), long_symbol.trim().to_ascii_uppercase()));
    }
    if short_market == LegMarket::Spot {
        spot_requests.push((
            short_venue.clone(),
            short_symbol.trim().to_ascii_uppercase(),
        ));
    }
    let conversion_start = spot_requests.len();
    for conversion in &row.quote_conversions {
        let Some(venue) = live_request_venue(&conversion.venue) else {
            return;
        };
        let symbol = conversion.symbol.trim().to_ascii_uppercase();
        if symbol.is_empty() {
            return;
        }
        spot_requests.push((venue, symbol));
    }
    if !spot_requests_have_capacity(plan, &spot_requests, conversion_limit) {
        return;
    }
    push_leg_with_limit(plan, long_market, &long_venue, long_symbol, symbol_limit);
    push_leg_with_limit(plan, short_market, &short_venue, short_symbol, symbol_limit);
    for (venue, symbol) in spot_requests.into_iter().skip(conversion_start) {
        let inserted =
            push_market_symbol_with_limit(&mut plan.spot, &venue, &symbol, conversion_limit);
        debug_assert!(inserted);
    }
}

#[cfg(test)]
#[path = "../live_tests.rs"]
mod tests;
