use super::*;

pub(super) fn live_request_venue(venue: &str) -> Option<String> {
    let venue = normalized_venue_name(venue.trim());
    (!venue.is_empty() && requests::is_ws_prewarm_venue(&venue)).then_some(venue)
}

pub(super) fn leg_request_symbol<'a>(
    market: LegMarket,
    fallback_symbol: &'a str,
    evidence_symbol: Option<&'a str>,
) -> Option<&'a str> {
    match market {
        LegMarket::Perp => Some(fallback_symbol),
        LegMarket::Spot => evidence_symbol.filter(|symbol| !symbol.trim().is_empty()),
        LegMarket::None => None,
    }
}

pub(super) fn opportunity_leg_markets(
    strategy_kind: Option<StrategyKind>,
    spot_leg_mode: Option<SpotLegMode>,
) -> (LegMarket, LegMarket) {
    match strategy_kind {
        Some(
            StrategyKind::PerpCross | StrategyKind::PerpPriceSpread | StrategyKind::FundingCarry,
        )
        | None => (LegMarket::Perp, LegMarket::Perp),
        Some(StrategyKind::SpotPerp | StrategyKind::CrossSpotPerp | StrategyKind::CashAndCarry) => {
            match spot_leg_mode {
                Some(SpotLegMode::BuySpot) => (LegMarket::Spot, LegMarket::Perp),
                Some(SpotLegMode::SellInventory | SpotLegMode::BorrowAndSell) => {
                    (LegMarket::Perp, LegMarket::Spot)
                }
                None => (LegMarket::None, LegMarket::None),
            }
        }
        Some(StrategyKind::SpotCross) => (LegMarket::Spot, LegMarket::Spot),
        Some(
            StrategyKind::Triangular
            | StrategyKind::OptionsPerpBasis
            | StrategyKind::QuarterlyPerp
            | StrategyKind::OnchainDepeg,
        ) => (LegMarket::None, LegMarket::None),
    }
}

pub(super) fn push_leg_with_limit(
    plan: &mut LiveRequestPlan,
    market: LegMarket,
    venue: &str,
    symbol: &str,
    symbol_limit: usize,
) {
    match market {
        LegMarket::Perp => {
            if push_market_symbol_with_limit(&mut plan.perp, venue, symbol, symbol_limit) {
                push_funding_symbol(&mut plan.funding, venue, symbol);
            }
        }
        LegMarket::Spot => {
            push_market_symbol_with_limit(&mut plan.spot, venue, symbol, symbol_limit);
        }
        LegMarket::None => {}
    }
}

pub(super) fn push_market_symbol_with_limit(
    requests: &mut BTreeMap<String, Vec<String>>,
    venue: &str,
    symbol: &str,
    limit: usize,
) -> bool {
    push_symbol_with_limit(requests, venue, symbol, limit)
}

pub(super) fn push_funding_symbol(
    requests: &mut BTreeMap<String, Vec<String>>,
    venue: &str,
    symbol: &str,
) {
    push_symbol_with_limit(requests, venue, symbol, WS_LIVE_FUNDING_SYMBOLS_PER_VENUE);
}

fn push_symbol_with_limit(
    requests: &mut BTreeMap<String, Vec<String>>,
    venue: &str,
    symbol: &str,
    limit: usize,
) -> bool {
    let venue = normalized_venue_name(venue.trim());
    let symbol = symbol.trim().to_ascii_uppercase();
    if venue.is_empty() || symbol.is_empty() || !requests::is_ws_prewarm_venue(&venue) {
        return false;
    }
    let symbols = requests.entry(venue).or_default();
    if symbols.contains(&symbol) {
        return true;
    }
    if symbols.len() >= limit {
        return false;
    }
    symbols.push(symbol);
    true
}
