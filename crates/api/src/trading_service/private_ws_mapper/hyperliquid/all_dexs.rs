use super::*;
use std::collections::BTreeSet;

/// `allDexsClearinghouseState` is a complete per-user DEX state record. A
/// configured DEX omitted from the record therefore has no position state and
/// must replace any cache invalidated by the previous WS session with an empty
/// snapshot. DEXes that contain positions keep their scoped REST recovery,
/// because the WS payload does not carry an official mark price.
pub(super) fn map_all_dexs_clearinghouse(
    rows: Vec<hyperliquid_ws_user::HyperliquidDexClearinghouseState>,
) -> Vec<PrivateWsEvent> {
    let present = rows
        .iter()
        .map(|row| {
            let dex = row
                .state
                .dex
                .as_deref()
                .map(str::trim)
                .filter(|dex| !dex.is_empty())
                .unwrap_or(row.dex.as_str());
            shared_types::normalized_venue_name(&hyperliquid_clearinghouse_venue(Some(dex)))
        })
        .collect::<BTreeSet<_>>();
    let mut events = rows
        .into_iter()
        .flat_map(map_hyperliquid_dex_clearinghouse)
        .collect::<Vec<_>>();

    events.extend(
        exchange::HyperliquidMarket::CONFIGURED_MARKETS
            .iter()
            .map(|market| market.venue())
            .filter(|venue| !present.contains(*venue))
            .map(|venue| {
                PrivateWsEvent::Positions(PrivatePositionsSnapshot {
                    venue: venue.to_owned(),
                    rows: Vec::new(),
                })
            }),
    );
    events
}
