use super::*;

impl TradingService {
    pub(in crate::trading_service) async fn list_scoped_balance_venues(
        &self,
        venues: &[String],
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        if venues.is_empty() {
            return Ok(Vec::new());
        }
        let epoch = self.account_cache_epoch();
        let now_ms = common::time::now_ms();
        let mut merged = Vec::new();
        for venue in venues {
            match self.balance_cache.fresh(venue, epoch, now_ms) {
                Some(rows) => merged.extend(rows),
                None => merged.extend(
                    self.fetch_scoped_balance_venue(venue, epoch, now_ms)
                        .await?,
                ),
            }
        }
        Ok(merged)
    }

    pub(in crate::trading_service) async fn fetch_scoped_balance_venue(
        &self,
        venue: &str,
        epoch: u64,
        now_ms: i64,
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        let fetch_lock = self.balance_fetch_lock(venue);
        let _guard = fetch_lock.lock().await;
        let locked_now_ms = common::time::now_ms();
        if let Some(rows) = self.balance_cache.fresh(venue, epoch, locked_now_ms) {
            return Ok(rows);
        }
        if let Some(error) = self.active_balance_backoff_error(&[venue.to_owned()], locked_now_ms) {
            return self.scoped_stale_balance_or_error(
                venue,
                epoch,
                now_ms.max(locked_now_ms),
                error,
                "scoped balance refresh backoff active; serving bounded stale rows",
            );
        }

        let result = self
            .engine
            .adapter()
            .get_exchange_balances(venue, None)
            .await;
        self.handle_scoped_balance_fetch_result(venue, epoch, now_ms.max(locked_now_ms), result)
    }

    pub(in crate::trading_service) fn handle_scoped_balance_fetch_result(
        &self,
        venue: &str,
        epoch: u64,
        stale_now_ms: i64,
        result: Result<Vec<VenueBalanceInfo>, exchange::ExchangeError>,
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        match result {
            Ok(rows) => {
                self.balance_cache.replace(venue, epoch, rows.clone());
                self.clear_balance_backoffs(&[venue.to_owned()]);
                Ok(rows)
            }
            Err(error) => {
                self.record_balance_fetch_error(
                    &[venue.to_owned()],
                    &error,
                    common::time::now_ms(),
                );
                self.scoped_stale_balance_or_error(
                    venue,
                    epoch,
                    stale_now_ms,
                    error,
                    "scoped balance refresh failed; serving bounded stale rows",
                )
            }
        }
    }

    pub(in crate::trading_service) fn scoped_stale_balance_or_error(
        &self,
        venue: &str,
        epoch: u64,
        now_ms: i64,
        error: exchange::ExchangeError,
        message: &'static str,
    ) -> Result<Vec<VenueBalanceInfo>, exchange::ExchangeError> {
        match self.balance_cache.stale(venue, epoch, now_ms) {
            Some(rows) => {
                tracing::warn!(%error, venue = %venue, message);
                Ok(rows)
            }
            None => Err(error),
        }
    }

    pub(in crate::trading_service) fn balance_fetch_lock(
        &self,
        venue: &str,
    ) -> Arc<tokio::sync::Mutex<()>> {
        let entry = self
            .balance_fetch_locks
            .entry(normalized_venue_name(venue))
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())));
        Arc::clone(entry.value())
    }

    pub(in crate::trading_service) async fn lock_balance_fetch_venues(
        &self,
        venues: &[String],
    ) -> Vec<tokio::sync::OwnedMutexGuard<()>> {
        let mut venues = scoped_balance_venues(venues);
        venues.sort_unstable();
        let mut guards = Vec::with_capacity(venues.len());
        for venue in venues {
            guards.push(self.balance_fetch_lock(&venue).lock_owned().await);
        }
        guards
    }

    pub(in crate::trading_service) fn active_balance_backoff_error(
        &self,
        venues: &[String],
        now_ms: i64,
    ) -> Option<exchange::ExchangeError> {
        venues
            .iter()
            .find_map(|venue| self.balance_backoff_error(venue, now_ms))
    }

    pub(in crate::trading_service) fn balance_backoff_error(
        &self,
        venue: &str,
        now_ms: i64,
    ) -> Option<exchange::ExchangeError> {
        let venue = normalized_venue_name(venue);
        let entry = self.balance_fetch_backoffs.get(&venue)?;
        if now_ms <= entry.retry_until_ms {
            return Some(entry.error.to_exchange_error());
        }
        drop(entry);
        self.balance_fetch_backoffs.remove(&venue);
        None
    }

    pub(in crate::trading_service) fn record_balance_fetch_error(
        &self,
        venues: &[String],
        error: &exchange::ExchangeError,
        observed_at_ms: i64,
    ) {
        let retry_until_ms = observed_at_ms.saturating_add(balance_fetch_backoff_ms(error) as i64);
        let cached = BalanceFetchBackoff {
            retry_until_ms,
            error: CachedExchangeError::from(error),
        };
        for venue in venues {
            let venue = normalized_venue_name(venue);
            if !venue.is_empty() {
                self.balance_fetch_backoffs.insert(venue, cached.clone());
            }
        }
    }

    pub(in crate::trading_service) fn clear_balance_backoffs(&self, venues: &[String]) {
        for venue in venues {
            self.balance_fetch_backoffs
                .remove(&normalized_venue_name(venue));
        }
    }
}
