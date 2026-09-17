use super::LiveRequestPlan;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LegMarket {
    None,
    Perp,
    Spot,
}

pub(super) fn leg_pair_has_capacity(
    plan: &LiveRequestPlan,
    long: (LegMarket, &str, &str),
    short: (LegMarket, &str, &str),
    symbol_limit: usize,
) -> bool {
    if long.0 == short.0 && long.1 == short.1 {
        return same_market_pair_has_capacity(plan, long.0, long.1, long.2, short.2, symbol_limit);
    }
    leg_has_capacity(plan, long.0, long.1, long.2, symbol_limit)
        && leg_has_capacity(plan, short.0, short.1, short.2, symbol_limit)
}

pub(super) fn spot_requests_have_capacity(
    plan: &LiveRequestPlan,
    additions: &[(String, String)],
    symbol_limit: usize,
) -> bool {
    let mut pending: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for (venue, symbol) in additions {
        if plan
            .spot
            .get(venue)
            .is_some_and(|symbols| symbols.contains(symbol))
        {
            continue;
        }
        pending
            .entry(venue.as_str())
            .or_default()
            .insert(symbol.as_str());
    }
    pending.into_iter().all(|(venue, symbols)| {
        plan.spot.get(venue).map_or(0, Vec::len) + symbols.len() <= symbol_limit
    })
}

fn same_market_pair_has_capacity(
    plan: &LiveRequestPlan,
    market: LegMarket,
    venue: &str,
    long_symbol: &str,
    short_symbol: &str,
    symbol_limit: usize,
) -> bool {
    let Some(requests) = market_requests(plan, market) else {
        return false;
    };
    let symbols = requests.get(venue);
    let long_symbol = long_symbol.trim().to_ascii_uppercase();
    let short_symbol = short_symbol.trim().to_ascii_uppercase();
    let contains = |symbol: &String| symbols.is_some_and(|rows| rows.contains(symbol));
    let additional = usize::from(!contains(&long_symbol))
        + usize::from(short_symbol != long_symbol && !contains(&short_symbol));
    symbols.map_or(0, Vec::len) + additional <= symbol_limit
}

fn leg_has_capacity(
    plan: &LiveRequestPlan,
    market: LegMarket,
    venue: &str,
    symbol: &str,
    symbol_limit: usize,
) -> bool {
    let Some(requests) = market_requests(plan, market) else {
        return false;
    };
    let Some(symbols) = requests.get(venue) else {
        return true;
    };
    let symbol = symbol.trim().to_ascii_uppercase();
    symbols.contains(&symbol) || symbols.len() < symbol_limit
}

fn market_requests(
    plan: &LiveRequestPlan,
    market: LegMarket,
) -> Option<&BTreeMap<String, Vec<String>>> {
    match market {
        LegMarket::Perp => Some(&plan.perp),
        LegMarket::Spot => Some(&plan.spot),
        LegMarket::None => None,
    }
}
