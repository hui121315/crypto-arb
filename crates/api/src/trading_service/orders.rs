use super::*;

impl TradingService {
    #[cfg(test)]
    pub(crate) fn mock_order_journal(&self) -> &OrderJournal {
        assert_eq!(self.adapter_name(), "mock");
        &self.journal
    }

    pub(crate) fn open_order_count(&self) -> usize {
        self.journal.open_order_count()
    }

    pub(crate) fn list_orders(&self) -> Vec<OrderRecord> {
        let mut orders = self.journal.list();
        orders.sort_by_key(|order| Reverse(order.updated_at_ms));
        orders
    }

    pub(crate) fn list_orders_page(
        &self,
        offset: usize,
        limit: usize,
        state: Option<LiveOrderState>,
        since_ms: Option<i64>,
    ) -> (Vec<OrderRecord>, usize) {
        self.journal
            .list_page_by_updated_at_desc_filtered(offset, limit, state, since_ms)
    }

    #[cfg(test)]
    pub(crate) fn list_execution_ledger_events(&self) -> Vec<ExecutionLedgerEvent> {
        self.journal.ledger_events()
    }

    pub(crate) fn list_execution_ledger_events_for_realized_window(
        &self,
        from_ms: i64,
        to_ms: i64,
    ) -> Vec<ExecutionLedgerEvent> {
        self.journal
            .ledger_events_for_realized_window(from_ms, to_ms)
    }

    pub(crate) async fn list_sql_realized_window(
        &self,
        from_ms: i64,
        to_ms: i64,
    ) -> Option<trading::SqlRealizedWindow> {
        self.journal.sql_realized_window(from_ms, to_ms).await
    }

    pub(crate) fn list_execution_ledger_events_by_query(
        &self,
        query: &ExecutionLedgerQuery,
    ) -> Vec<ExecutionLedgerEvent> {
        self.journal.ledger_events_by_query(query)
    }

    pub(crate) fn execution_ledger_storage_snapshot(&self) -> ExecutionLedgerStorageSnapshot {
        self.journal.execution_ledger_storage_snapshot()
    }

    pub(crate) fn order_snapshot_storage_snapshot(&self) -> OrderSnapshotStorageSnapshot {
        self.journal.order_snapshot_storage_snapshot()
    }

    pub(crate) fn sql_ledger_storage_snapshot(&self) -> SqlLedgerStorageSnapshot {
        self.journal.sql_ledger_storage_snapshot()
    }

    pub(crate) async fn persist_ledger_event_group_durable(
        &self,
        events: &[ExecutionLedgerEvent],
    ) -> Result<(), String> {
        self.journal
            .persist_ledger_event_group_durable(events)
            .await
    }

    pub(crate) async fn claim_sql_projection_jobs(
        &self,
        projector: &str,
        now_ms: i64,
        limit: usize,
        lease_ms: i64,
    ) -> Result<Vec<trading::SqlProjectionJob>, String> {
        self.journal
            .claim_sql_projection_jobs(projector, now_ms, limit, lease_ms)
            .await
    }

    pub(crate) async fn complete_sql_projection_job(
        &self,
        job: &trading::SqlProjectionJob,
        completed_at_ms: i64,
    ) -> Result<trading::SqlProjectionJobAck, String> {
        self.journal
            .complete_sql_projection_job(job, completed_at_ms)
            .await
    }

    pub(crate) async fn retry_sql_projection_job(
        &self,
        job: &trading::SqlProjectionJob,
        available_at_ms: i64,
        last_error: &str,
    ) -> Result<trading::SqlProjectionJobAck, String> {
        self.journal
            .retry_sql_projection_job(job, available_at_ms, last_error)
            .await
    }

    pub(crate) async fn drain_sql_ledger_and_shutdown(&self) -> Result<(), String> {
        self.journal.drain_sql_ledger_and_shutdown().await
    }

    pub(crate) fn append_run_finality_event(&self, event: SqlRunFinalityLedgerEvent) -> bool {
        self.journal.append_run_finality_event(event)
    }

    pub(crate) async fn persist_run_finality_event(
        &self,
        event: &SqlRunFinalityLedgerEvent,
    ) -> Result<trading::SqlLedgerPersistAck, String> {
        self.journal.persist_run_finality_event(event).await
    }

    pub(crate) async fn project_run_cost_event(
        &self,
        event: &ExecutionLedgerEvent,
    ) -> Result<usize, String> {
        self.journal.project_run_cost_event(event).await
    }

    pub(crate) async fn rebuild_run_cost_facts(
        &self,
        page_size: usize,
    ) -> Result<trading::SqlRunCostRebuildReport, String> {
        self.journal.rebuild_run_cost_facts(page_size).await
    }

    #[cfg(test)]
    pub(crate) async fn query_run_cost_facts(
        &self,
        run_kind: &str,
        run_id: &str,
    ) -> Result<Vec<trading::SqlRunCostFact>, String> {
        self.journal.query_run_cost_facts(run_kind, run_id).await
    }

    pub(crate) fn record_orderbook_evidence(
        &self,
        internal_order_id: &str,
        input: &trading::OrderbookDepthLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        self.journal
            .record_orderbook_evidence(internal_order_id, input, source, captured_at_ms)
    }
}
