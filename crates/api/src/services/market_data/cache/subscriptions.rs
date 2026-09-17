use super::*;
use shared_types::VenueMarketSubscription;

impl MarketDataCache {
    pub(crate) fn apply_market_subscription(&self, row: &VenueMarketSubscription) {
        if !row.spot_enabled {
            retain_other_venues(&self.spot_ticks, &row.venue);
            retain_other_venues(&self.spot_orderbooks, &row.venue);
            self.mark_spot_tick_projection_dirty();
        }
        if !row.perp_enabled {
            retain_other_venues(&self.tickers, &row.venue);
            retain_other_venues(&self.orderbooks, &row.venue);
            retain_other_venues(&self.mark_indexes, &row.venue);
            self.mark_perp_ticker_projection_dirty();
        }
        if !row.perp_enabled || !row.funding_enabled {
            retain_other_venues(&self.funding, &row.venue);
            self.mark_funding_projection_dirty();
        }
    }
}

fn retain_other_venues<T>(rows: &DashMap<MarketKey, CachedEntry<T>>, disabled_venue: &str) {
    rows.retain(|key, _| !key.belongs_to_venue(disabled_venue));
}
