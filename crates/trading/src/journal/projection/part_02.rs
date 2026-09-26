/// [`OrderJournal::claim_created`] 的结果。
#[derive(Debug, Clone)]
pub enum CreatedClaim {
    /// 占位成功，本次调用获得唯一提交权。
    New(OrderRecord),
    /// 同 `client_order_id` 已有记录：幂等重放，直接返回既有记录。
    Existing(OrderRecord),
    /// 占位已被并发提交写入但记录尚未落地：调用方应拒绝而非重放。
    Pending,
}
impl OrderJournal {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn new_with_audit_path(path: impl Into<PathBuf>) -> Self {
        Self::new_with_paths(Some(path.into()), None, None)
    }

    pub fn new_with_ledger_path(path: impl Into<PathBuf>) -> Self {
        Self::new_with_paths(None, Some(path.into()), None)
    }

    pub fn new_with_storage_paths(
        ledger_path: Option<PathBuf>,
        order_snapshot_path: Option<PathBuf>,
    ) -> Self {
        Self::new_with_paths(None, ledger_path, order_snapshot_path)
    }

    #[cfg(test)]
    fn default_without_audit() -> Self {
        Self::new_with_paths(None, None, None)
    }

    fn new_with_paths(
        audit_path: Option<PathBuf>,
        ledger_path: Option<PathBuf>,
        order_snapshot_path: Option<PathBuf>,
    ) -> Self {
        Self::new_with_paths_and_sql(
            audit_path,
            ledger_path,
            order_snapshot_path,
            SqlLedgerInit::unconfigured(common::time::now_ms()),
        )
    }

    pub fn new_with_storage_paths_and_sql(
        ledger_path: Option<PathBuf>,
        order_snapshot_path: Option<PathBuf>,
        sql_ledger: SqlLedgerInit,
    ) -> Self {
        Self::new_with_paths_and_sql(None, ledger_path, order_snapshot_path, sql_ledger)
    }

    fn new_with_paths_and_sql(
        audit_path: Option<PathBuf>,
        ledger_path: Option<PathBuf>,
        order_snapshot_path: Option<PathBuf>,
        sql_ledger: SqlLedgerInit,
    ) -> Self {
        let SqlLedgerInit {
            migration_health,
            replay,
            store,
        } = sql_ledger;
        let SqlLedgerReplay {
            events: sql_events,
            order_snapshots: sql_order_snapshots,
            balance_events: _,
            run_finality_events: _,
            health: sql_replay_health,
        } = replay;
        let (execution_ledger, replayed_events, replay_failures) = ledger_path
            .as_ref()
            .map(|path| execution_ledger_from_path(path.as_path()))
            .unwrap_or_default();
        for event in sql_events {
            execution_ledger.upsert(event);
        }
        let order_snapshot_replay = order_snapshot_path
            .as_ref()
            .map(|path| order_snapshots_from_path(path.as_path()))
            .unwrap_or_default();
        let replayed_records = order_snapshot_replay.rows.len();
        let journal = Self {
            records: DashMap::new(),
            client_index: DashMap::new(),
            exchange_index: DashMap::new(),
            ledger_contexts: DashMap::new(),
            execution_ledger,
            events: RwLock::new(Vec::new()),
            ledger_append_lock: Mutex::new(()),
            order_snapshot_append_lock: Mutex::new(()),
            sql_ledger_store: store,
            sql_ledger_migration_health: migration_health,
            sql_ledger_replay_health: sql_replay_health,
            open_count: AtomicUsize::new(0),
            audit_path,
            ledger_path,
            order_snapshot_path,
            ledger_replayed_events: AtomicUsize::new(replayed_events),
            ledger_replay_failures: AtomicUsize::new(replay_failures),
            ledger_append_successes: AtomicUsize::new(0),
            ledger_append_failures: AtomicUsize::new(0),
            ledger_last_append_at_ms: AtomicI64::new(0),
            ledger_query_successes: AtomicUsize::new(0),
            ledger_query_failures: AtomicUsize::new(0),
            ledger_last_query_at_ms: AtomicI64::new(0),
            order_snapshot_replayed_records: AtomicUsize::new(replayed_records),
            order_snapshot_replay_failures: AtomicUsize::new(order_snapshot_replay.failed_lines),
            order_snapshot_append_successes: AtomicUsize::new(0),
            order_snapshot_append_failures: AtomicUsize::new(0),
            order_snapshot_last_append_at_ms: AtomicI64::new(0),
        };
        journal.restore_order_snapshots(&order_snapshot_replay.rows);
        journal.restore_order_snapshots(&sql_order_snapshots);
        journal.restore_ledger_contexts();
        journal
    }

    /// See [`CreatedClaim`].
    ///
    /// submit 幂等占位：以 `client_index` 的 entry 锁为原子性来源。
    /// 首个到达者写入占位并获得唯一提交权（`New`）；同 `client_order_id` 的
    /// 并发/重放请求拿到既有记录（`Existing`）或"占位已写、记录未落地"的
    /// `Pending`——消除此前 `get_by_client_order_id` + `insert_created` 两步
    /// check-then-act 竞态下同一 `client_order_id` 双发到交易所的可能。
    pub fn claim_created(&self, intent: OrderIntent, at_ms: i64) -> CreatedClaim {
        self.claim_created_with_product(intent, shared_types::FeeProduct::Unknown, at_ms)
    }

    pub fn claim_created_with_product(
        &self,
        intent: OrderIntent,
        product: shared_types::FeeProduct,
        at_ms: i64,
    ) -> CreatedClaim {
        self.claim_created_with_account(intent, product, None, at_ms)
    }

    pub fn claim_created_with_account(
        &self,
        intent: OrderIntent,
        product: shared_types::FeeProduct,
        account_scope: Option<String>,
        at_ms: i64,
    ) -> CreatedClaim {
        let occupied = {
            match self.client_index.entry(intent.client_order_id.clone()) {
                dashmap::mapref::entry::Entry::Occupied(entry) => Some(entry.get().clone()),
                dashmap::mapref::entry::Entry::Vacant(slot) => {
                    slot.insert(intent.id.clone());
                    None
                }
            }
            // guard 在此释放；insert_created 内的同 key 重建索引不会自锁。
        };
        match occupied {
            None => CreatedClaim::New(self.insert_created_with_account(intent, product, account_scope, at_ms)),
            Some(existing_internal) => match self.get(&existing_internal) {
                Some(existing) => CreatedClaim::Existing(existing),
                // 占位已写但记录尚未落地：并发提交进行中，调用方应拒绝而非重放。
                None => CreatedClaim::Pending,
            },
        }
    }

    pub fn insert_created(&self, intent: OrderIntent, at_ms: i64) -> OrderRecord {
        self.insert_created_with_product(intent, shared_types::FeeProduct::Unknown, at_ms)
    }

    pub fn insert_created_with_product(
        &self,
        intent: OrderIntent,
        product: shared_types::FeeProduct,
        at_ms: i64,
    ) -> OrderRecord {
        self.insert_created_with_account(intent, product, None, at_ms)
    }

    fn insert_created_with_account(
        &self,
        mut intent: OrderIntent,
        product: shared_types::FeeProduct,
        account_scope: Option<String>,
        at_ms: i64,
    ) -> OrderRecord {
        enrich_intent_policy(&mut intent);
        let mut identity = VenueOrderIdentity::from_intent_with_product(&intent, product);
        identity.account_scope = account_scope;
        let record = OrderRecord {
            intent: intent.clone(),
            state: LiveOrderState::Created,
            risk: None,
            identity,
            last_update_source: OrderUpdateSource::Internal,
            exchange_order_id: None,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
            updated_at_ms: at_ms,
        };
        let previous = self.records.insert(intent.id.clone(), record.clone());
        if let Some(previous) = previous.as_ref() {
            self.remove_record_indexes(previous);
        }
        self.apply_open_count_delta(previous.as_ref().map(|row| row.state), record.state);
        self.index_record(&record);
        let event = OrderEventRecord {
            internal_order_id: intent.id,
            client_order_id: intent.client_order_id,
            exchange_order_id: None,
            source: OrderUpdateSource::Internal,
            identity: record.identity_snapshot(),
            previous_state: None,
            state: LiveOrderState::Created,
            event: OrderLifecycleEvent::Created,
            message: None,
            payload: serde_json::Value::Null,
            occurred_at_ms: at_ms,
        };
        self.push_event(&event);
        self.append_order_snapshot(&record);
        record
    }

    pub fn get(&self, internal_order_id: &str) -> Option<OrderRecord> {
        self.records
            .get(internal_order_id)
            .map(|entry| entry.clone())
    }

    pub fn get_by_client_order_id(&self, client_order_id: &str) -> Option<OrderRecord> {
        let internal_id = self
            .client_index
            .get(client_order_id)
            .map(|entry| entry.clone())?;
        self.get(&internal_id)
    }

    pub fn get_by_exchange_order_id(&self, exchange_order_id: &str) -> Option<OrderRecord> {
        let internal_id = self
            .exchange_index
            .get(exchange_order_id)
            .map(|entry| entry.clone())?;
        self.get(&internal_id)
    }

    pub fn list(&self) -> Vec<OrderRecord> {
        self.records
            .iter()
            .map(|entry| entry.value().clone())
            .collect()
    }

    pub fn list_page_by_updated_at_desc(
        &self,
        offset: usize,
        limit: usize,
    ) -> (Vec<OrderRecord>, usize) {
        self.list_page_by_updated_at_desc_filtered(offset, limit, None, None)
    }

    pub fn list_page_by_updated_at_desc_filtered(
        &self,
        offset: usize,
        limit: usize,
        state: Option<LiveOrderState>,
        since_ms: Option<i64>,
    ) -> (Vec<OrderRecord>, usize) {
        let mut keys = self
            .records
            .iter()
            .filter(|entry| order_matches_filter(entry.value(), state, since_ms))
            .map(|entry| (Reverse(entry.value().updated_at_ms), entry.key().clone()))
            .collect::<Vec<_>>();
        keys.sort_unstable();
        let total_rows = keys.len();
        let rows = keys
            .into_iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(_, id)| self.get(&id))
            .collect();
        (rows, total_rows)
    }

    pub fn events(&self) -> Vec<OrderEventRecord> {
        self.events.read().clone()
    }

    pub fn ledger_events(&self) -> Vec<ExecutionLedgerEvent> {
        self.execution_ledger.list()
    }

    pub fn ledger_events_for_realized_window(
        &self,
        from_ms: i64,
        to_ms: i64,
    ) -> Vec<ExecutionLedgerEvent> {
        if to_ms <= from_ms {
            self.record_ledger_query_failure();
            return Vec::new();
        }
        self.record_ledger_query_success();
        self.execution_ledger.realized_window_events(from_ms, to_ms)
    }

    pub async fn sql_realized_window(&self, from_ms: i64, to_ms: i64) -> Option<SqlRealizedWindow> {
        let store = self.sql_ledger_store.as_ref()?;
        match store.query_realized_window(from_ms, to_ms).await {
            Ok(window) => Some(window),
            Err(error) => {
                tracing::warn!(%error, "trading SQL ledger realized query failed");
                None
            }
        }
    }
}
