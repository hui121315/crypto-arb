use super::*;

pub(super) fn watchlist_ticker_requests(
    watchlist: &[realtime::WatchlistItem],
) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for item in watchlist.iter().filter(|item| item.enabled) {
        push_watchlist_ticker(&mut out, item.venue_long.as_deref(), &item.symbol);
        push_watchlist_ticker(&mut out, item.venue_short.as_deref(), &item.symbol);
    }
    out
}

fn push_watchlist_ticker(
    out: &mut BTreeMap<String, Vec<String>>,
    venue: Option<&str>,
    symbol: &str,
) {
    // adapter `NAME` constants are lower-case, and `Aggregator::get` does an
    // exact-match lookup. Normalize watchlist venue keys before routing.
    let Some(venue) = venue
        .map(str::trim)
        .filter(|venue| !venue.is_empty())
        .map(normalized_venue_name)
    else {
        return;
    };
    if !is_ws_prewarm_venue(&venue) {
        return;
    }
    let symbol = symbol.trim();
    if symbol.is_empty() {
        return;
    }
    let entry = out.entry(venue).or_default();
    if entry.len() >= WATCHLIST_TICKER_SYMBOLS_PER_VENUE {
        return;
    }
    let upper = symbol.to_ascii_uppercase();
    if entry.iter().any(|existing| existing == &upper) {
        return;
    }
    entry.push(upper);
}

pub(super) fn is_ws_prewarm_venue(venue: &str) -> bool {
    VenueId::from_exchange_name(venue)
        .is_some_and(|id| id != VenueId::GateCrossEx && WS_PREWARM_VENUES.contains(&id))
}
