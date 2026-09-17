use super::*;
use shared_types::OrderInfo;

const OPEN_ORDER_OPERATION: &str = "open_orders";
const OPEN_ORDER_FETCH_ERROR_BACKOFF_MS: i64 = 30_000;
const HYPERLIQUID_OPEN_ORDER_FETCH_ERROR_BACKOFF_MS: i64 = 60_000;

impl TradingService {
    pub(crate) async fn list_open_orders(&self) -> Result<Vec<OrderInfo>, exchange::ExchangeError> {
        let epoch = self.account_cache_epoch();
        let venues = self.open_order_cache_venues();
        if venues.is_empty() {
            return self.fetch_adapter_open_orders().await;
        }
        let now_ms = common::time::now_ms();
        let (mut rows, missing) = self.read_open_order_cache(&venues, epoch, now_ms);
        let (missing, deferred, blocked_error) =
            self.partition_open_order_refresh(&missing, epoch, now_ms);
        rows.extend(deferred);
        if missing.is_empty() {
            sort_open_orders(&mut rows);
            return blocked_error.map_or(Ok(rows), Err);
        }
        let stale = self.open_order_cache.stale_all(&missing, epoch, now_ms);
        let _refresh_guard = match self.open_order_fetch_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => match stale {
                Some(stale_rows) => {
                    rows.extend(stale_rows);
                    sort_open_orders(&mut rows);
                    return blocked_error.map_or(Ok(rows), Err);
                }
                None => self.open_order_fetch_lock.lock().await,
            },
        };
        let refresh_now_ms = common::time::now_ms();
        let (mut rows, missing) = self.read_open_order_cache(&venues, epoch, refresh_now_ms);
        let (missing, deferred, blocked_error) =
            self.partition_open_order_refresh(&missing, epoch, refresh_now_ms);
        rows.extend(deferred);
        if missing.is_empty() {
            sort_open_orders(&mut rows);
            return blocked_error.map_or(Ok(rows), Err);
        }
        self.route_failures.record(OPEN_ORDER_OPERATION, Vec::new());
        match self.fetch_adapter_open_orders_for_venues(&missing).await {
            Ok(fetched) => {
                rows.extend(self.merge_open_order_refresh(
                    epoch,
                    &missing,
                    refresh_now_ms,
                    fetched,
                ));
                sort_open_orders(&mut rows);
                blocked_error.map_or(Ok(rows), Err)
            }
            Err(error) => {
                self.record_open_order_fetch_error(&missing, &error, refresh_now_ms);
                match self
                    .open_order_cache
                    .stale_all(&missing, epoch, refresh_now_ms)
                {
                    Some(stale_rows) => {
                        rows.extend(stale_rows);
                        sort_open_orders(&mut rows);
                        blocked_error.map_or(Ok(rows), Err)
                    }
                    None => Err(error),
                }
            }
        }
    }

    pub(crate) fn open_order_cache_latest_change_ms(&self) -> i64 {
        self.open_order_cache.latest_change_ms()
    }

    pub(super) fn apply_open_order_cache(&self, order: &OrderInfo) -> bool {
        self.open_order_cache.apply_order(
            &order.exchange,
            self.account_cache_epoch(),
            order.clone(),
        )
    }

    pub(super) fn remove_open_order_cache(&self, venue: &str, exchange_order_id: &str) -> bool {
        self.open_order_cache.remove_by_exchange_order_id(
            venue,
            self.account_cache_epoch(),
            exchange_order_id,
        )
    }

    fn open_order_cache_venues(&self) -> Vec<String> {
        let account_reader_venues = self.account_reader_venues();
        if !account_reader_venues.is_empty() {
            return account_reader_venues;
        }
        let venues = self
            .risk
            .config()
            .allowed_exchanges
            .iter()
            .map(|venue| normalized_venue_name(venue))
            .collect::<Vec<_>>();
        if venues.is_empty() {
            self.open_order_cache.venues(self.account_cache_epoch())
        } else {
            venues
        }
    }

    fn read_open_order_cache(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> (Vec<OrderInfo>, Vec<String>) {
        let mut rows = Vec::new();
        let mut missing = Vec::new();
        for venue in venues {
            match self.open_order_cache.fresh(venue, epoch, now_ms) {
                Some(cached) => rows.extend(cached),
                None => missing.push(venue.clone()),
            }
        }
        (rows, missing)
    }

    fn merge_open_order_refresh(
        &self,
        epoch: u64,
        venues: &[String],
        now_ms: i64,
        mut rows: Vec<OrderInfo>,
    ) -> Vec<OrderInfo> {
        let failed = self.record_open_order_route_failure_backoffs(venues, now_ms);
        rows.retain(|row| !failed.contains(&normalized_venue_name(&row.exchange)));
        self.seed_successful_open_order_venues(epoch, venues, &rows, &failed);

        for venue in failed {
            self.open_order_cache.invalidate(&venue);
            if let Some(stale) = self.open_order_cache.stale(&venue, epoch, now_ms) {
                tracing::warn!(
                    venue = %venue,
                    rows = stale.len(),
                    "open-order refresh failed; serving bounded per-venue stale rows"
                );
                rows.extend(stale);
            }
        }
        rows.sort_by(|left, right| {
            left.exchange
                .cmp(&right.exchange)
                .then(left.symbol.cmp(&right.symbol))
                .then(left.order_id.cmp(&right.order_id))
        });
        rows
    }

    fn partition_open_order_refresh(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> (Vec<String>, Vec<OrderInfo>, Option<exchange::ExchangeError>) {
        let mut refresh = Vec::new();
        let mut deferred = Vec::new();
        let mut blocked_error = None;
        for venue in venues {
            if let Some(error) = self.open_order_backoff_error(venue, now_ms) {
                if let Some(rows) = self.open_order_cache.stale(venue, epoch, now_ms) {
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

    fn open_order_backoff_error(
        &self,
        venue: &str,
        now_ms: i64,
    ) -> Option<exchange::ExchangeError> {
        let venue = normalized_venue_name(venue);
        let entry = self.open_order_fetch_backoffs.get(&venue)?;
        if now_ms <= entry.retry_until_ms {
            return Some(entry.error.to_exchange_error());
        }
        drop(entry);
        self.open_order_fetch_backoffs.remove(&venue);
        None
    }

    fn record_open_order_fetch_error(
        &self,
        venues: &[String],
        error: &exchange::ExchangeError,
        now_ms: i64,
    ) {
        for venue in venues {
            let venue = normalized_venue_name(venue);
            if !venue.is_empty() {
                let minimum_backoff_ms =
                    if venue == "hyperliquid" || venue.starts_with("hyperliquid:") {
                        HYPERLIQUID_OPEN_ORDER_FETCH_ERROR_BACKOFF_MS
                    } else {
                        OPEN_ORDER_FETCH_ERROR_BACKOFF_MS
                    };
                let backoff_ms = error
                    .retry_after_ms()
                    .unwrap_or(minimum_backoff_ms as u64)
                    .max(minimum_backoff_ms as u64);
                self.open_order_fetch_backoffs.insert(
                    venue,
                    BalanceFetchBackoff {
                        retry_until_ms: now_ms.saturating_add(backoff_ms as i64),
                        error: CachedExchangeError::from(error),
                    },
                );
            }
        }
    }

    fn record_open_order_route_failure_backoffs(
        &self,
        venues: &[String],
        now_ms: i64,
    ) -> HashSet<String> {
        let route_errors = self.route_failures.exchange_errors(OPEN_ORDER_OPERATION);
        let failed = route_errors
            .iter()
            .map(|(venue, _)| normalized_venue_name(venue))
            .collect::<HashSet<_>>();
        for (venue, error) in route_errors {
            self.record_open_order_fetch_error(&[venue], &error, now_ms);
        }
        for venue in venues {
            let venue = normalized_venue_name(venue);
            if !failed.contains(&venue) {
                self.open_order_fetch_backoffs.remove(&venue);
            }
        }
        failed
    }

    fn seed_successful_open_order_venues(
        &self,
        epoch: u64,
        venues: &[String],
        rows: &[OrderInfo],
        failed: &HashSet<String>,
    ) {
        let mut by_venue: HashMap<String, Vec<OrderInfo>> = HashMap::new();
        for row in rows {
            by_venue
                .entry(normalized_venue_name(&row.exchange))
                .or_default()
                .push(row.clone());
        }
        for venue in venues {
            if !failed.contains(venue) {
                self.open_order_cache.replace(
                    venue,
                    epoch,
                    by_venue.get(venue).cloned().unwrap_or_default(),
                );
            }
        }
    }
}

fn sort_open_orders(rows: &mut [OrderInfo]) {
    rows.sort_by(|left, right| {
        left.exchange
            .cmp(&right.exchange)
            .then(left.symbol.cmp(&right.symbol))
            .then(left.order_id.cmp(&right.order_id))
    });
}

#[cfg(test)]
#[path = "open_orders/tests.rs"]
mod tests;
