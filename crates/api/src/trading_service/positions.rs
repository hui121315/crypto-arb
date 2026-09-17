use super::*;

mod backoff;

const POSITION_OPERATION: &str = "positions";
const POSITION_FETCH_ERROR_BACKOFF_MS: i64 = 30_000;

impl TradingService {
    pub(crate) async fn refresh_scoped_positions(
        &self,
        venues: &[String],
    ) -> Result<Vec<PositionInfo>, exchange::ExchangeError> {
        let venues = scoped_balance_venues(venues);
        if venues.is_empty() {
            return Ok(Vec::new());
        }
        let _refresh_guard = self.position_fetch_lock.lock().await;
        self.refresh_scoped_positions_locked(&venues).await
    }

    async fn refresh_scoped_positions_locked(
        &self,
        venues: &[String],
    ) -> Result<Vec<PositionInfo>, exchange::ExchangeError> {
        let now_ms = common::time::now_ms();
        let epoch = self.account_cache_epoch();
        let (refresh_venues, mut rows, blocked_error) =
            self.partition_position_refresh(venues, epoch, now_ms);
        if refresh_venues.is_empty() {
            sort_positions(&mut rows);
            return blocked_error.map_or(Ok(rows), Err);
        }
        for venue in &refresh_venues {
            self.position_cache.invalidate(venue);
        }
        self.route_failures.record(POSITION_OPERATION, Vec::new());
        match self
            .fetch_adapter_positions_for_venues(&refresh_venues)
            .await
        {
            Ok(fetched) => {
                rows.extend(self.merge_position_refresh(epoch, &refresh_venues, now_ms, fetched));
                sort_positions(&mut rows);
                blocked_error.map_or(Ok(rows), Err)
            }
            Err(error) => {
                self.record_position_fetch_error(&refresh_venues, &error, now_ms);
                match self
                    .position_cache
                    .stale_all(&refresh_venues, epoch, now_ms)
                {
                    Some(stale_rows) => {
                        rows.extend(stale_rows);
                        sort_positions(&mut rows);
                        blocked_error.map_or(Ok(rows), Err)
                    }
                    None => Err(error),
                }
            }
        }
    }

    pub(crate) async fn list_positions_low_latency(
        self: &Arc<Self>,
    ) -> Result<Vec<PositionInfo>, exchange::ExchangeError> {
        let venues = self.position_cache_venues();
        if venues.is_empty() {
            return self.list_positions().await;
        }
        self.list_position_venues_low_latency(venues).await
    }

    pub(crate) async fn list_scoped_positions_low_latency(
        self: &Arc<Self>,
        venues: &[String],
    ) -> Result<Vec<PositionInfo>, exchange::ExchangeError> {
        let venues = scoped_balance_venues(venues);
        if venues.is_empty() {
            return Ok(Vec::new());
        }
        self.list_position_venues_low_latency(venues).await
    }

    async fn list_position_venues_low_latency(
        self: &Arc<Self>,
        venues: Vec<String>,
    ) -> Result<Vec<PositionInfo>, exchange::ExchangeError> {
        let epoch = self.account_cache_epoch();
        let now_ms = common::time::now_ms();
        let (mut rows, missing) = self.read_position_cache(&venues, epoch, now_ms);
        if missing.is_empty() {
            sort_positions(&mut rows);
            return Ok(rows);
        }
        let Some(stale_rows) = self.position_cache.stale_all(&missing, epoch, now_ms) else {
            return self.refresh_scoped_positions(&venues).await;
        };
        rows.extend(stale_rows);
        sort_positions(&mut rows);

        if self.position_fetch_lock.try_lock().is_ok() {
            let service = Arc::clone(self);
            tokio::spawn(async move {
                if let Err(error) = service.refresh_scoped_positions(&missing).await {
                    tracing::warn!(%error, "background position cache refresh failed");
                }
            });
        }
        Ok(rows)
    }

    pub(crate) async fn list_positions(
        &self,
    ) -> Result<Vec<PositionInfo>, exchange::ExchangeError> {
        let epoch = self.account_cache_epoch();
        let venues = self.position_cache_venues();
        if venues.is_empty() {
            return self.fetch_adapter_positions().await;
        }
        let now_ms = common::time::now_ms();
        let (mut rows, missing) = self.read_position_cache(&venues, epoch, now_ms);
        if missing.is_empty() {
            sort_positions(&mut rows);
            return Ok(rows);
        }
        let stale = self.position_cache.stale_all(&missing, epoch, now_ms);
        let _refresh_guard = match self.position_fetch_lock.try_lock() {
            Ok(guard) => guard,
            Err(_) => match stale {
                Some(stale_rows) => {
                    rows.extend(stale_rows);
                    sort_positions(&mut rows);
                    return Ok(rows);
                }
                None => self.position_fetch_lock.lock().await,
            },
        };
        let refresh_now_ms = common::time::now_ms();
        let (mut rows, missing) = self.read_position_cache(&venues, epoch, refresh_now_ms);
        if missing.is_empty() {
            sort_positions(&mut rows);
            return Ok(rows);
        }
        match self.refresh_scoped_positions_locked(&missing).await {
            Ok(refreshed) => {
                rows.extend(refreshed);
                sort_positions(&mut rows);
                Ok(rows)
            }
            Err(error) => match self
                .position_cache
                .stale_all(&missing, epoch, refresh_now_ms)
            {
                Some(stale_rows) => {
                    rows.extend(stale_rows);
                    sort_positions(&mut rows);
                    Ok(rows)
                }
                None => Err(error),
            },
        }
    }

    fn merge_position_refresh(
        &self,
        epoch: u64,
        venues: &[String],
        now_ms: i64,
        mut rows: Vec<PositionInfo>,
    ) -> Vec<PositionInfo> {
        let failed = self.record_position_route_failure_backoffs(venues, now_ms);
        rows.retain(|row| !failed.contains(&normalized_venue_name(&row.exchange)));
        self.seed_successful_position_venues(epoch, venues, &rows, &failed);

        for venue in failed {
            self.position_cache.invalidate(&venue);
            if let Some(stale) = self.position_cache.stale(&venue, epoch, now_ms) {
                tracing::warn!(
                    venue = %venue,
                    rows = stale.len(),
                    "position refresh failed; serving bounded per-venue stale rows"
                );
                rows.extend(stale);
            }
        }
        rows.sort_by(|left, right| {
            left.exchange
                .cmp(&right.exchange)
                .then(left.symbol.cmp(&right.symbol))
                .then(left.side.cmp(&right.side))
        });
        rows
    }

    fn position_cache_venues(&self) -> Vec<String> {
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
            self.position_cache
                .fresh_venues(self.account_cache_epoch(), common::time::now_ms())
        } else {
            venues
        }
    }

    fn read_position_cache(
        &self,
        venues: &[String],
        epoch: u64,
        now_ms: i64,
    ) -> (Vec<PositionInfo>, Vec<String>) {
        let mut rows = Vec::new();
        let mut missing = Vec::new();
        for venue in venues {
            match self.position_cache.fresh(venue, epoch, now_ms) {
                Some(cached) => rows.extend(cached),
                None => missing.push(venue.clone()),
            }
        }
        (rows, missing)
    }

    fn seed_successful_position_venues(
        &self,
        epoch: u64,
        venues: &[String],
        rows: &[PositionInfo],
        failed: &HashSet<String>,
    ) {
        let mut by_venue: HashMap<String, Vec<PositionInfo>> = HashMap::new();
        for row in rows {
            by_venue
                .entry(normalized_venue_name(&row.exchange))
                .or_default()
                .push(row.clone());
        }
        for venue in venues {
            if !failed.contains(venue) {
                self.position_cache.replace(
                    venue,
                    epoch,
                    by_venue.get(venue).cloned().unwrap_or_default(),
                );
            }
        }
    }
}

fn sort_positions(rows: &mut [PositionInfo]) {
    rows.sort_by(|left, right| {
        left.exchange
            .cmp(&right.exchange)
            .then(left.symbol.cmp(&right.symbol))
            .then(left.side.cmp(&right.side))
    });
}

#[cfg(test)]
#[path = "positions/tests.rs"]
mod tests;
