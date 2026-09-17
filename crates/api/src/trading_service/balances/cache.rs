use super::*;

impl TradingService {
    pub(super) fn extend_fresh_hyperliquid_cache_venues(&self, venues: &mut Vec<String>) {
        let epoch = self.account_cache_epoch();
        let now_ms = common::time::now_ms();
        let mut seen = venues.iter().cloned().collect::<HashSet<_>>();
        for snapshot in self.balance_cache.snapshots(epoch, now_ms) {
            let venue = normalized_venue_name(&snapshot.venue);
            if snapshot.quality == AccountCacheQuality::Fresh
                && is_hyperliquid_cache_dispatcher_venue(&venue)
                && seen.insert(venue.clone())
            {
                venues.push(venue);
            }
        }
    }

    pub(in crate::trading_service) fn read_balance_cache(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> BalanceCacheRead {
        let mut read = BalanceCacheRead::default();
        for venue in venues {
            match self.balance_cache.fresh(venue, epoch, now_ms) {
                Some(rows) => read.push_fresh(venue, rows),
                None => read.missing.push(venue.clone()),
            }
        }
        read
    }

    pub(in crate::trading_service) async fn fetch_configured_balances_or_stale(
        &self,
        fetch: ConfiguredBalanceFetch,
        epoch: u64,
        now_ms: i64,
        missing: &[String],
        merged: &mut Vec<VenueBalanceInfo>,
    ) -> Result<Option<Vec<VenueBalanceInfo>>, exchange::ExchangeError> {
        match self.fetch_configured_balances(fetch, missing).await {
            Ok(rows) => Ok(Some(rows)),
            Err(error) => {
                self.record_balance_fetch_error(missing, &error, common::time::now_ms());
                let error_message = error.to_string();
                let result =
                    self.merge_stale_balances_or_error(epoch, now_ms, missing, merged, error);
                if matches!(result, Ok(None)) {
                    tracing::warn!(
                        error = %error_message,
                        missing = ?missing,
                        "balance cache refresh failed; serving bounded stale rows"
                    );
                }
                result
            }
        }
    }

    pub(in crate::trading_service) fn merge_stale_balances_or_error(
        &self,
        epoch: u64,
        now_ms: i64,
        missing: &[String],
        merged: &mut Vec<VenueBalanceInfo>,
        error: exchange::ExchangeError,
    ) -> Result<Option<Vec<VenueBalanceInfo>>, exchange::ExchangeError> {
        let Some(stale_rows) = self.stale_balance_rows(epoch, now_ms, missing) else {
            return Err(error);
        };
        for rows in stale_rows {
            merged.extend(rows);
        }
        Ok(None)
    }

    pub(in crate::trading_service) fn stale_balance_rows(
        &self,
        epoch: u64,
        now_ms: i64,
        venues: &[String],
    ) -> Option<Vec<Vec<VenueBalanceInfo>>> {
        venues
            .iter()
            .map(|venue| self.balance_cache.stale(venue, epoch, now_ms))
            .collect()
    }

    pub(in crate::trading_service) fn seed_balance_cache_from_full_read(
        &self,
        epoch: u64,
        missing: &[String],
        fetched: &[VenueBalanceInfo],
        failed: &HashSet<String>,
    ) {
        let mut by_venue: HashMap<String, Vec<VenueBalanceInfo>> = HashMap::new();
        for row in fetched {
            by_venue
                .entry(row.venue.clone())
                .or_default()
                .push(row.clone());
        }
        for venue in missing {
            if failed.contains(&normalized_venue_name(venue)) {
                continue;
            }
            let rows = by_venue.get(venue).cloned().unwrap_or_default();
            self.balance_cache.replace(venue, epoch, rows);
        }
    }

    pub(in crate::trading_service) fn merge_failed_balance_stale(
        &self,
        epoch: u64,
        now_ms: i64,
        missing: &[String],
        failed: &HashSet<String>,
        merged: &mut Vec<VenueBalanceInfo>,
    ) {
        let missing = missing
            .iter()
            .map(|venue| normalized_venue_name(venue))
            .collect::<HashSet<_>>();
        for venue in failed.iter().filter(|venue| missing.contains(*venue)) {
            if let Some(rows) = self.balance_cache.stale(venue, epoch, now_ms) {
                tracing::warn!(
                    venue = %venue,
                    rows = rows.len(),
                    "balance refresh failed; serving bounded per-venue stale rows"
                );
                merged.extend(rows);
            }
        }
    }
}
