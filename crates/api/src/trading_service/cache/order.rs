use super::super::private_ws_events::{PrivateAccountDirty, PrivateAccountScope};
use super::super::TradingService;

impl TradingService {
    pub(crate) fn mark_open_order_cache_stale(&self, venue: &str) {
        self.open_order_cache.invalidate(venue);
    }

    pub(crate) fn mark_filled_account_cache_stale(&self, venue: &str, reason: &str) {
        self.mark_private_account_dirty(&PrivateAccountDirty::new(
            venue,
            PrivateAccountScope::All,
            reason,
        ));
    }
}
