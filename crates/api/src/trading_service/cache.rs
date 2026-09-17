use super::private_ws_events::{PrivateAccountDirty, PrivateAccountScope};
use super::*;

mod balance;
mod epoch;
mod order;

use epoch::uses_event_driven_position_cache;

impl TradingService {
    pub(crate) fn clear_account_cache(&self) {
        self.open_order_cache.clear();
        self.position_cache.clear();
        self.balance_cache.clear();
        self.account_summaries.clear();
        self.asset_valuations.clear();
        self.account_evidence_refresh_after_ms.clear();
    }

    pub(crate) fn mark_private_account_dirty(&self, dirty: &PrivateAccountDirty) {
        let venue = normalized_venue_name(&dirty.venue);
        if venue.is_empty() {
            return;
        }
        if dirty.scope.invalidates_positions() {
            self.position_cache.invalidate(&venue);
        }
        if dirty.scope.invalidates_balances() {
            self.balance_cache.invalidate(&venue);
            self.account_summaries.remove(&venue);
            self.asset_valuations
                .retain(|(current, _), _| current != &venue);
        }
    }

    pub(crate) fn unresolved_private_account_scope(
        &self,
        venue: &str,
        requested: PrivateAccountScope,
        now_ms: i64,
    ) -> Option<PrivateAccountScope> {
        let venue = normalized_venue_name(venue);
        if venue.is_empty() {
            return Some(requested);
        }
        let epoch = self.account_cache_epoch();
        let positions_missing = requested.invalidates_positions()
            && self.position_cache.fresh(&venue, epoch, now_ms).is_none();
        let balances_missing = requested.invalidates_balances()
            && self.balance_cache.fresh(&venue, epoch, now_ms).is_none();
        match (positions_missing, balances_missing) {
            (true, true) => Some(PrivateAccountScope::All),
            (true, false) => Some(PrivateAccountScope::Positions),
            (false, true) => Some(PrivateAccountScope::Balances),
            (false, false) => None,
        }
    }

    pub(crate) fn initialize_account_reader(
        &self,
        credentials: AdapterCredentials,
    ) -> Result<(), exchange::ExchangeError> {
        self.replace_account_reader(credentials, false)
    }

    pub(crate) fn refresh_account_reader(
        &self,
        credentials: AdapterCredentials,
    ) -> Result<(), exchange::ExchangeError> {
        self.replace_account_reader(credentials, true)
    }

    fn replace_account_reader(
        &self,
        credentials: AdapterCredentials,
        invalidate_cache: bool,
    ) -> Result<(), exchange::ExchangeError> {
        let reader = live_adapters::account_reader_from_credentials(
            credentials,
            Arc::clone(&self.route_failures),
        )?;
        if self.adapter_name() == LIVE_ROUTER_ADAPTER_ID {
            if let Some(router) = reader.as_ref() {
                let allowed_exchanges = retained_live_execution_scope(
                    &self.risk.config().allowed_exchanges,
                    router.allowed_exchanges(),
                );
                let execution_router: Arc<dyn exchange::LiveTradingAdapter> =
                    Arc::<live_adapters::LiveVenueRouter>::clone(router);
                self.engine.set_adapter(execution_router);
                self.risk.update_config(|config| {
                    config.live_trading_enabled = true;
                    config.allowed_exchanges = allowed_exchanges;
                });
            } else {
                self.engine.set_adapter(Arc::new(MockLiveAdapter::new()));
                *self.adapter_name.write() = "mock";
                self.risk.update_config(|config| {
                    config.live_trading_enabled = false;
                    config.allowed_exchanges = BTreeSet::new();
                });
            }
        }
        self.account_reader.store(reader);
        if invalidate_cache {
            self.invalidate_account_credentials();
        }
        Ok(())
    }

    pub(super) fn account_reader_venues(&self) -> Vec<String> {
        self.account_reader
            .load_full()
            .map(|reader| reader.allowed_exchanges().into_iter().collect())
            .unwrap_or_default()
    }

    pub(crate) fn observe_private_ws_order_session_activity(&self, venue: &str) {
        let epoch = self.account_cache_epoch();
        for cache_venue in self.private_ws_cache_venues(venue) {
            if uses_event_driven_position_cache(&cache_venue) {
                self.open_order_cache.touch(&cache_venue, epoch);
            }
        }
    }

    pub(crate) fn observe_private_ws_account_session_activity(&self, venue: &str) {
        let epoch = self.account_cache_epoch();
        for cache_venue in self.private_ws_cache_venues(venue) {
            if uses_event_driven_position_cache(&cache_venue) {
                self.position_cache.touch(&cache_venue, epoch);
            }
        }
    }

    pub(crate) fn invalidate_private_ws_session_cache(&self, venue: &str) {
        for cache_venue in self.private_ws_cache_venues(venue) {
            self.open_order_cache.invalidate(&cache_venue);
            if uses_event_driven_position_cache(&cache_venue) {
                self.position_cache.invalidate(&cache_venue);
            }
        }
    }

    fn private_ws_cache_venues(&self, venue: &str) -> Vec<String> {
        expand_private_ws_cache_venues(venue, self.account_reader_venues())
    }

    pub(crate) fn invalidate_account_credentials(&self) {
        self.account_cache_epoch.fetch_add(1, Ordering::Relaxed);
        self.clear_account_cache();
        self.balance_fetch_backoffs.clear();
        self.open_order_fetch_backoffs.clear();
        self.position_fetch_backoffs.clear();
    }

    pub(super) async fn fetch_adapter_positions(
        &self,
    ) -> Result<Vec<PositionInfo>, exchange::ExchangeError> {
        match self.account_reader.load_full() {
            Some(reader) => reader.get_positions(None).await,
            None => self.engine.adapter().get_positions(None).await,
        }
    }

    pub(super) async fn fetch_adapter_positions_for_venues(
        &self,
        venues: &[String],
    ) -> Result<Vec<PositionInfo>, exchange::ExchangeError> {
        match self.account_reader.load_full() {
            Some(reader) if !venues.is_empty() => reader.positions_for_venues(venues, None).await,
            _ => self.fetch_adapter_positions().await,
        }
    }

    pub(super) async fn fetch_adapter_account_read(
        &self,
    ) -> Result<VenueAccountRead, exchange::ExchangeError> {
        match self.account_reader.load_full() {
            Some(reader) => reader.get_account_read(None).await,
            None => self.engine.adapter().get_account_read(None).await,
        }
    }

    pub(super) async fn fetch_adapter_account_read_for_venues(
        &self,
        venues: &[String],
    ) -> Result<VenueAccountRead, exchange::ExchangeError> {
        match self.account_reader.load_full() {
            Some(reader) if !venues.is_empty() => {
                reader.account_read_for_venues(venues, None).await
            }
            _ => self.fetch_adapter_account_read().await,
        }
    }

    pub(super) async fn fetch_adapter_account_evidence_for_venues(
        &self,
        venues: &[String],
    ) -> Result<VenueAccountRead, exchange::ExchangeError> {
        match self.account_reader.load_full() {
            Some(reader) => reader.account_evidence_for_venues(venues).await,
            None => self.fetch_adapter_account_read().await,
        }
    }

    pub(super) async fn fetch_adapter_open_orders(
        &self,
    ) -> Result<Vec<shared_types::OrderInfo>, exchange::ExchangeError> {
        match self.account_reader.load_full() {
            Some(reader) => reader.get_open_orders(None).await,
            None => self.engine.adapter().get_open_orders(None).await,
        }
    }

    pub(super) async fn fetch_adapter_open_orders_for_venues(
        &self,
        venues: &[String],
    ) -> Result<Vec<shared_types::OrderInfo>, exchange::ExchangeError> {
        match self.account_reader.load_full() {
            Some(reader) if !venues.is_empty() => reader.open_orders_for_venues(venues, None).await,
            _ => self.fetch_adapter_open_orders().await,
        }
    }
}

fn retained_live_execution_scope(
    current: &BTreeSet<String>,
    available: BTreeSet<String>,
) -> BTreeSet<String> {
    if current.is_empty() {
        return available;
    }
    current
        .intersection(&available)
        .cloned()
        .collect::<BTreeSet<_>>()
}

fn expand_private_ws_cache_venues(venue: &str, account_venues: Vec<String>) -> Vec<String> {
    let venue = normalized_venue_name(venue);
    if venue != "hyperliquid" {
        return (!venue.is_empty()).then_some(venue).into_iter().collect();
    }
    let mut venues = account_venues
        .into_iter()
        .map(|candidate| normalized_venue_name(&candidate))
        .filter(|candidate| candidate == "hyperliquid" || candidate.starts_with("hyperliquid:"))
        .collect::<Vec<_>>();
    venues.push(venue);
    venues.sort_unstable();
    venues.dedup();
    venues
}

#[cfg(test)]
mod tests;
