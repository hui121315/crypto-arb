impl OrderJournal {
    pub fn ledger_events_by_query(
        &self,
        query: &ExecutionLedgerQuery,
    ) -> Vec<ExecutionLedgerEvent> {
        if invalid_ledger_query(query) {
            self.record_ledger_query_failure();
            return Vec::new();
        }
        self.record_ledger_query_success();
        self.execution_ledger.query(query)
    }

    pub fn attach_execution_ledger_context(
        &self,
        internal_order_id: &str,
        context: ExecutionLedgerOrderContext,
    ) {
        self.ledger_contexts
            .insert(internal_order_id.to_owned(), context);
    }

    pub fn clear_execution_ledger_context(&self, internal_order_id: &str) {
        self.ledger_contexts.remove(internal_order_id);
    }

    pub fn execution_ledger_storage_snapshot(&self) -> ExecutionLedgerStorageSnapshot {
        let last_append_at_ms = match self.ledger_last_append_at_ms.load(Ordering::Acquire) {
            value if value > 0 => Some(value),
            _ => None,
        };
        let last_query_at_ms = match self.ledger_last_query_at_ms.load(Ordering::Acquire) {
            value if value > 0 => Some(value),
            _ => None,
        };
        ExecutionLedgerStorageSnapshot {
            configured: self.ledger_path.is_some(),
            path: self
                .ledger_path
                .as_ref()
                .map(|path| path.display().to_string()),
            event_count: self.execution_ledger.list().len(),
            replayed_events: self.ledger_replayed_events.load(Ordering::Acquire),
            replay_failures: self.ledger_replay_failures.load(Ordering::Acquire),
            append_successes: self.ledger_append_successes.load(Ordering::Acquire),
            append_failures: self.ledger_append_failures.load(Ordering::Acquire),
            last_append_at_ms,
            query_successes: self.ledger_query_successes.load(Ordering::Acquire),
            query_failures: self.ledger_query_failures.load(Ordering::Acquire),
            last_query_at_ms,
        }
    }

    pub fn order_snapshot_storage_snapshot(&self) -> OrderSnapshotStorageSnapshot {
        let last_append_at_ms = match self
            .order_snapshot_last_append_at_ms
            .load(Ordering::Acquire)
        {
            value if value > 0 => Some(value),
            _ => None,
        };
        OrderSnapshotStorageSnapshot {
            configured: self.order_snapshot_path.is_some(),
            path: self
                .order_snapshot_path
                .as_ref()
                .map(|path| path.display().to_string()),
            record_count: self.records.len(),
            replayed_records: self.order_snapshot_replayed_records.load(Ordering::Acquire),
            replay_failures: self.order_snapshot_replay_failures.load(Ordering::Acquire),
            append_successes: self.order_snapshot_append_successes.load(Ordering::Acquire),
            append_failures: self.order_snapshot_append_failures.load(Ordering::Acquire),
            last_append_at_ms,
        }
    }

    pub fn sql_ledger_storage_snapshot(&self) -> SqlLedgerStorageSnapshot {
        self.sql_ledger_store.as_ref().map_or_else(
            || {
                SqlLedgerStorageSnapshot::without_writer(
                    self.sql_ledger_migration_health.clone(),
                    self.sql_ledger_replay_health.clone(),
                )
            },
            SqlLedgerStore::snapshot,
        )
    }

    pub fn open_order_count(&self) -> usize {
        self.open_count.load(Ordering::Acquire)
    }

    pub fn mark_risk_checked(
        &self,
        internal_order_id: &str,
        risk: RiskDecision,
        at_ms: i64,
    ) -> Option<OrderRecord> {
        let event = if risk.allowed {
            OrderLifecycleEvent::RiskApproved
        } else {
            OrderLifecycleEvent::RiskRejected
        };
        let payload = serde_json::json!({ "risk": &risk });
        self.transition_update(
            TransitionUpdate {
                internal_order_id,
                event,
                source: OrderUpdateSource::Internal,
                at_ms,
                message: None,
                payload,
            },
            move |r| {
                r.risk = Some(risk);
            },
        )
    }

    pub fn mark_submitted(&self, internal_order_id: &str, at_ms: i64) -> Option<OrderRecord> {
        self.transition_update(
            TransitionUpdate {
                internal_order_id,
                event: OrderLifecycleEvent::Submitted,
                source: OrderUpdateSource::Internal,
                at_ms,
                message: None,
                payload: serde_json::Value::Null,
            },
            |_| {},
        )
    }

    pub fn apply_ack(&self, ack: &OrderAck) -> Option<OrderRecord> {
        let event = event_from_ack_state(ack.state);
        let payload = serde_json::to_value(ack).unwrap_or(serde_json::Value::Null);
        let updated = self.transition_update(
            TransitionUpdate {
                internal_order_id: &ack.internal_order_id,
                event,
                source: OrderUpdateSource::AdapterAck,
                at_ms: ack.accepted_at_ms,
                message: ack.message.clone(),
                payload,
            },
            |r| {
                r.identity = r.identity_snapshot();
                r.identity.record_ack(ack);
                r.exchange_order_id = r.identity.exchange_order_id.clone();
                if ack.message.is_some() {
                    r.message = ack.message.clone();
                }
                apply_ack_fill_fields(r, ack);
            },
        )?;
        if ack_has_fill_evidence(ack) {
            self.record_record_fill_snapshot(
                &updated,
                OrderUpdateSource::AdapterAck,
                ack.accepted_at_ms,
            );
        }
        Some(updated)
    }

    pub fn apply_cancel_request_ack(&self, ack: &OrderAck) -> Option<OrderRecord> {
        let payload = serde_json::to_value(ack).unwrap_or(serde_json::Value::Null);
        let message = ack
            .message
            .clone()
            .or_else(|| Some("cancel accepted; awaiting finality".to_owned()));
        self.transition_update(
            TransitionUpdate {
                internal_order_id: &ack.internal_order_id,
                event: OrderLifecycleEvent::CancelRequest,
                source: OrderUpdateSource::AdapterAck,
                at_ms: ack.accepted_at_ms,
                message: message.clone(),
                payload,
            },
            |r| {
                r.identity = r.identity_snapshot();
                r.identity.record_ack(ack);
                r.exchange_order_id = r.identity.exchange_order_id.clone();
                r.message = message;
            },
        )
    }

    pub fn apply_order_info(
        &self,
        internal_order_id: &str,
        info: &OrderInfo,
        at_ms: i64,
    ) -> Option<OrderRecord> {
        self.apply_order_info_from_source(
            internal_order_id,
            info,
            at_ms,
            OrderUpdateSource::OrderQuery,
        )
    }

    pub fn apply_order_info_from_source(
        &self,
        internal_order_id: &str,
        info: &OrderInfo,
        at_ms: i64,
        source: OrderUpdateSource,
    ) -> Option<OrderRecord> {
        let target = live_state_from_order_status(info.status);
        if self.get(internal_order_id)?.state == target {
            return self.patch_order_info(internal_order_id, info, at_ms, source);
        }
        let event = event_from_target_state(target)?;
        let payload = serde_json::to_value(info).unwrap_or(serde_json::Value::Null);
        let updated = self.transition_update(
            TransitionUpdate {
                internal_order_id,
                event,
                source,
                at_ms,
                message: Some(format!("order status backfill: {:?}", info.status)),
                payload,
            },
            |record| apply_order_info_fields(record, info),
        )?;
        self.record_fill_snapshot(&updated, info, source, at_ms);
        Some(updated)
    }

    pub fn apply_order_info_by_client_order_id(
        &self,
        client_order_id: &str,
        info: &OrderInfo,
        at_ms: i64,
    ) -> Option<OrderRecord> {
        self.apply_order_info_by_client_order_id_from_source(
            client_order_id,
            info,
            at_ms,
            OrderUpdateSource::OrderQuery,
        )
    }
}
