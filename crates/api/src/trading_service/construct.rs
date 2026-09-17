use super::*;

impl TradingService {
    #[cfg(test)]
    pub(crate) fn new_mock() -> Self {
        Self::new_mock_with_journal(Arc::new(OrderJournal::new()))
    }

    pub(crate) fn new_mock_with_storage_paths_and_sql(
        execution_ledger_path: Option<PathBuf>,
        order_snapshot_path: Option<PathBuf>,
        sql_ledger: SqlLedgerInit,
    ) -> Self {
        let balance_replay = sql_ledger.replay.balance_events.clone();
        let service =
            Self::new_mock_with_journal(Arc::new(OrderJournal::new_with_storage_paths_and_sql(
                execution_ledger_path,
                order_snapshot_path,
                sql_ledger,
            )));
        service.seed_balance_cache_from_sql_replay(&balance_replay);
        service
    }

    pub(super) fn seed_balance_cache_from_sql_replay(
        &self,
        events: &[SqlBalanceLedgerReplayEvent],
    ) -> usize {
        let latest = latest_balance_replay_events(events);
        let mut grouped: HashMap<String, BalanceReplaySeed> = HashMap::new();
        for ((venue, currency), event) in latest {
            let mut row = event.row;
            row.venue = venue.clone();
            row.currency = currency;
            grouped
                .entry(venue)
                .or_insert_with(|| BalanceReplaySeed::new(event.observed_at_ms))
                .push(row, event.observed_at_ms);
        }
        let epoch = self.account_cache_epoch();
        let mut seeded = 0usize;
        for (venue, seed) in grouped {
            seeded = seeded.saturating_add(seed.rows.len());
            self.balance_cache
                .replace_at(&venue, epoch, seed.rows, seed.observed_at_ms);
        }
        seeded
    }

    #[cfg(test)]
    pub(crate) fn seed_balance_cache_from_sql_replay_for_test(
        &self,
        events: &[SqlBalanceLedgerReplayEvent],
    ) -> usize {
        self.seed_balance_cache_from_sql_replay(events)
    }

    pub(super) fn new_mock_with_journal(journal: Arc<OrderJournal>) -> Self {
        let adapter: Arc<dyn LiveTradingAdapter> = Arc::new(MockLiveAdapter::new());
        let risk_config = RiskConfig::default();
        let risk = RiskEngine::new(risk_config);
        let engine = Arc::new(ExecutionEngine::new(
            adapter,
            risk.clone(),
            Arc::clone(&journal),
        ));
        Self {
            engine,
            journal,
            risk,
            live_order_proof_health: Arc::new(
                crate::services::live_order_proof_health::LiveOrderProofHealthStore::default(),
            ),
            adapter_name: RwLock::new("mock"),
            account_reader: ArcSwapOption::empty(),
            account_cache_epoch: AtomicU64::new(0),
            balance_cache: VenueBalanceCache::new(BALANCE_CACHE_TTL_MS, BALANCE_CACHE_MAX_STALE_MS),
            account_summaries: DashMap::new(),
            asset_valuations: DashMap::new(),
            account_evidence_refresh_after_ms: DashMap::new(),
            balance_fetch_locks: DashMap::new(),
            balance_fetch_backoffs: DashMap::new(),
            open_order_fetch_lock: tokio::sync::Mutex::new(()),
            open_order_fetch_backoffs: DashMap::new(),
            open_order_cache: VenueOpenOrderCache::new(
                OPEN_ORDER_CACHE_TTL_MS,
                OPEN_ORDER_CACHE_MAX_STALE_MS,
            ),
            position_fetch_lock: tokio::sync::Mutex::new(()),
            position_fetch_backoffs: DashMap::new(),
            position_cache: VenuePositionCache::new(
                POSITION_CACHE_TTL_MS,
                POSITION_CACHE_MAX_STALE_MS,
            ),
            route_failures: Arc::new(RouteFailureSink::default()),
            latest_funding_payment_ingest: ArcSwapOption::empty(),
        }
    }

    pub(crate) fn with_live_order_proof_health(
        mut self,
        store: Arc<crate::services::live_order_proof_health::LiveOrderProofHealthStore>,
    ) -> Self {
        store.replay_persisted_place_ack_evidence(
            &self.journal.list(),
            &self.journal.ledger_events(),
        );
        self.live_order_proof_health = store;
        self
    }

    pub(crate) fn adapter_name(&self) -> &'static str {
        *self.adapter_name.read()
    }
}
