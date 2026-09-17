use super::*;

impl TradingService {
    pub(super) fn replace_open_order_cache(&self, venue: &str, rows: Vec<OrderInfo>) {
        self.open_order_cache
            .replace(venue, self.account_cache_epoch(), rows);
    }

    pub(super) fn replace_position_cache(&self, venue: &str, rows: Vec<PositionInfo>) {
        self.position_cache
            .replace(venue, self.account_cache_epoch(), rows);
    }

    pub(super) fn upsert_position_cache(&self, venue: &str, rows: Vec<PositionInfo>) -> bool {
        self.position_cache
            .upsert(venue, self.account_cache_epoch(), rows)
    }

    pub(super) fn replace_balance_cache(&self, venue: &str, rows: Vec<VenueBalanceInfo>) {
        self.balance_cache
            .replace(venue, self.account_cache_epoch(), rows);
    }

    pub(super) fn upsert_balance_cache(&self, venue: &str, rows: Vec<VenueBalanceInfo>) {
        self.balance_cache
            .upsert(venue, self.account_cache_epoch(), rows);
    }

    pub(in crate::trading_service::private_ws_events) fn mark_private_event_account_dirty(
        &self,
        dirty: PrivateAccountDirty,
    ) -> PrivateAccountDirty {
        self.mark_private_account_dirty(&dirty);
        dirty
    }
}
