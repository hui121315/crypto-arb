use super::*;

impl TradingService {
    pub(super) fn position_backoff_error(
        &self,
        venue: &str,
        now_ms: i64,
    ) -> Option<exchange::ExchangeError> {
        let venue = normalized_venue_name(venue);
        let entry = self.position_fetch_backoffs.get(&venue)?;
        if now_ms <= entry.retry_until_ms {
            return Some(entry.error.to_exchange_error());
        }
        drop(entry);
        self.position_fetch_backoffs.remove(&venue);
        None
    }

    pub(super) fn record_position_fetch_error(
        &self,
        venues: &[String],
        error: &exchange::ExchangeError,
        now_ms: i64,
    ) {
        let backoff_ms = error
            .retry_after_ms()
            .unwrap_or(POSITION_FETCH_ERROR_BACKOFF_MS as u64)
            .max(POSITION_FETCH_ERROR_BACKOFF_MS as u64);
        let backoff = BalanceFetchBackoff {
            retry_until_ms: now_ms.saturating_add(backoff_ms as i64),
            error: CachedExchangeError::from(error),
        };
        for venue in venues {
            let venue = normalized_venue_name(venue);
            if !venue.is_empty() {
                self.position_fetch_backoffs.insert(venue, backoff.clone());
            }
        }
    }

    pub(super) fn record_position_route_failure_backoffs(
        &self,
        venues: &[String],
        now_ms: i64,
    ) -> HashSet<String> {
        let route_errors = self.route_failures.exchange_errors(POSITION_OPERATION);
        let failed = route_errors
            .iter()
            .map(|(venue, _)| normalized_venue_name(venue))
            .collect::<HashSet<_>>();
        for (venue, error) in route_errors {
            self.record_position_fetch_error(&[venue], &error, now_ms);
        }
        for venue in venues {
            let venue = normalized_venue_name(venue);
            if !failed.contains(&venue) {
                self.position_fetch_backoffs.remove(&venue);
            }
        }
        failed
    }

    pub(super) fn partition_position_refresh(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> (
        Vec<String>,
        Vec<PositionInfo>,
        Option<exchange::ExchangeError>,
    ) {
        let mut refresh = Vec::new();
        let mut deferred = Vec::new();
        let mut blocked_error = None;
        for venue in venues {
            if let Some(error) = self.position_backoff_error(venue, now_ms) {
                if let Some(rows) = self.position_cache.stale(venue, epoch, now_ms) {
                    deferred.extend(rows);
                } else if blocked_error.is_none() {
                    blocked_error = Some(error);
                }
                continue;
            }
            refresh.push(venue.clone());
        }
        (refresh, deferred, blocked_error)
    }
}
