use super::*;

#[cfg(test)]
pub(super) fn push_leg(plan: &mut LiveRequestPlan, market: LegMarket, venue: &str, symbol: &str) {
    push_leg_with_limit(plan, market, venue, symbol, WS_LIVE_SYMBOLS_PER_VENUE);
}

#[cfg(test)]
pub(super) fn push_market_symbol(
    requests: &mut BTreeMap<String, Vec<String>>,
    venue: &str,
    symbol: &str,
) -> bool {
    push_market_symbol_with_limit(requests, venue, symbol, WS_LIVE_SYMBOLS_PER_VENUE)
}
