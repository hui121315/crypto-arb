impl OrderJournal {
    fn transition_update<F>(&self, input: TransitionUpdate<'_>, f: F) -> Option<OrderRecord>
    where
        F: FnOnce(&mut OrderRecord),
    {
        let mut record = self.records.get_mut(input.internal_order_id)?;
        let previous_state = record.state;
        let previous_record = record.clone();
        let next_state = match transition(previous_state, input.event) {
            Ok(next) => next,
            Err(err) => {
                let audit = OrderEventRecord {
                    internal_order_id: input.internal_order_id.to_owned(),
                    client_order_id: record.intent.client_order_id.clone(),
                    exchange_order_id: record.exchange_order_id.clone(),
                    source: input.source,
                    identity: record.identity_snapshot(),
                    previous_state: Some(previous_state),
                    state: previous_state,
                    event: input.event,
                    message: Some(format!("audit::illegal_transition: {err}")),
                    payload: input.payload,
                    occurred_at_ms: input.at_ms,
                };
                drop(record);
                self.push_event(&audit);
                return None;
            }
        };
        f(&mut record);
        record.state = next_state;
        record.last_update_source = input.source;
        record.updated_at_ms = input.at_ms;
        let out = record.clone();
        let audit = OrderEventRecord {
            internal_order_id: input.internal_order_id.to_owned(),
            client_order_id: out.intent.client_order_id.clone(),
            exchange_order_id: out.exchange_order_id.clone(),
            source: input.source,
            identity: out.identity_snapshot(),
            previous_state: Some(previous_state),
            state: next_state,
            event: input.event,
            message: input.message,
            payload: input.payload,
            occurred_at_ms: input.at_ms,
        };
        drop(record);
        self.apply_open_count_delta(Some(previous_state), next_state);
        self.reindex_record(&previous_record, &out);
        self.push_event(&audit);
        self.record_order_state(&out, &audit);
        self.append_order_snapshot(&out);
        Some(out)
    }

    fn push_event(&self, event: &OrderEventRecord) {
        self.events.write().push(event.clone());
        if let Some(path) = &self.audit_path {
            if let Err(err) = append_jsonl(path, event) {
                tracing::warn!(path = %path.display(), error = %err, "failed to append order audit event");
            }
        }
    }

    fn apply_open_count_delta(&self, previous: Option<LiveOrderState>, next: LiveOrderState) {
        match (
            previous.map(is_open_state).unwrap_or(false),
            is_open_state(next),
        ) {
            (false, true) => {
                self.open_count.fetch_add(1, Ordering::AcqRel);
            }
            (true, false) => {
                self.open_count.fetch_sub(1, Ordering::AcqRel);
            }
            _ => {}
        }
    }

    fn patch_order_info(
        &self,
        internal_order_id: &str,
        info: &OrderInfo,
        at_ms: i64,
        source: OrderUpdateSource,
    ) -> Option<OrderRecord> {
        let mut record = self.records.get_mut(internal_order_id)?;
        let previous_record = record.clone();
        apply_order_info_fields(&mut record, info);
        record.last_update_source = source;
        record.updated_at_ms = at_ms;
        let out = record.clone();
        drop(record);
        self.reindex_record(&previous_record, &out);
        self.record_fill_snapshot(&out, info, source, at_ms);
        self.append_order_snapshot(&out);
        Some(out)
    }

    fn record_fill_snapshot(
        &self,
        record: &OrderRecord,
        info: &OrderInfo,
        source: OrderUpdateSource,
        at_ms: i64,
    ) {
        let context = self.ledger_context_for(record);
        if let Some(event) = self.execution_ledger.record_fill_snapshot_with_context(
            record,
            info,
            source,
            at_ms,
            context.as_ref(),
        ) {
            self.append_ledger_event(&event);
        }
    }

    fn record_record_fill_snapshot(
        &self,
        record: &OrderRecord,
        source: OrderUpdateSource,
        at_ms: i64,
    ) {
        let context = self.ledger_context_for(record);
        if let Some(fill_event) = self
            .execution_ledger
            .record_record_fill_snapshot_with_context(record, source, at_ms, context.as_ref())
        {
            self.append_ledger_event(&fill_event);
            if let Some(slippage_event) = self.record_slippage_for_fill(FillSlippageRecord {
                record,
                fill_event: &fill_event,
                context: context.as_ref(),
            }) {
                self.append_ledger_event(&slippage_event);
            }
        }
    }

    fn record_order_state(&self, record: &OrderRecord, event: &OrderEventRecord) {
        let context = self.ledger_context_for(record);
        if let Some(event) =
            self.execution_ledger
                .record_order_state_with_context(record, event, context.as_ref())
        {
            self.append_ledger_event(&event);
        }
    }

    fn ledger_context_for(&self, record: &OrderRecord) -> Option<ExecutionLedgerOrderContext> {
        self.ledger_contexts
            .get(&record.intent.id)
            .map(|entry| entry.value().clone())
    }

    fn funding_record_for(
        &self,
        venue: &str,
        symbol: &str,
        funding_time_ms: i64,
    ) -> Result<OrderRecord, FundingPaymentIngestSkipReason> {
        let venue = venue.trim();
        let symbol_key = funding_symbol_key(symbol);
        if venue.is_empty() || symbol_key.is_empty() {
            return Err(FundingPaymentIngestSkipReason::InvalidMatchKey);
        }
        let mut selected: Option<OrderRecord> = None;
        let mut selected_group: Option<String> = None;
        let mut missing_fill_anchor = false;
        for entry in self.records.iter() {
            let record = entry.value();
            if !funding_candidate(record, venue, &symbol_key, funding_time_ms) {
                continue;
            }
            if !self.record_has_fill_before(&record.intent.id, funding_time_ms) {
                missing_fill_anchor = true;
                continue;
            }
            let group_id = hedge_group_id(&record.intent.id);
            if selected_group
                .as_deref()
                .is_some_and(|current| current != group_id)
            {
                return Err(FundingPaymentIngestSkipReason::AmbiguousOrderGroup);
            }
            selected_group.get_or_insert(group_id);
            if selected
                .as_ref()
                .is_none_or(|current| funding_record_is_newer(record, current))
            {
                selected = Some(record.clone());
            }
        }
        selected.ok_or(if missing_fill_anchor {
            FundingPaymentIngestSkipReason::NoFilledAnchor
        } else {
            FundingPaymentIngestSkipReason::NoMatchingOrder
        })
    }

    fn record_has_fill_before(&self, internal_order_id: &str, funding_time_ms: i64) -> bool {
        self.execution_ledger.list().iter().any(|event| {
            event.order.identity.internal_order_id == internal_order_id
                && event.occurred_at_ms <= funding_time_ms
                && matches!(
                    event.event_type,
                    shared_types::ExecutionLedgerEventType::FillSnapshot
                        | shared_types::ExecutionLedgerEventType::FillEvent
                )
                && matches!(event.payload, ExecutionLedgerPayload::FillSnapshot(_))
        })
    }

    fn append_ledger_event(&self, event: &ExecutionLedgerEvent) {
        self.append_ledger_event_with_sql_mode(event, LedgerSqlWriteMode::BestEffort);
    }

    fn append_ledger_event_with_sql_mode(
        &self,
        event: &ExecutionLedgerEvent,
        sql_mode: LedgerSqlWriteMode,
    ) {
        let _guard = self.ledger_append_lock.lock();
        self.append_ledger_event_with_sql_mode_locked(event, sql_mode);
    }

    fn append_ledger_event_with_sql_mode_locked(
        &self,
        event: &ExecutionLedgerEvent,
        sql_mode: LedgerSqlWriteMode,
    ) {
        if sql_mode == LedgerSqlWriteMode::BestEffort {
            if let Some(store) = &self.sql_ledger_store {
                store.append_event(event);
            }
        }
        if let Some(path) = &self.ledger_path {
            if let Err(err) = ledger_store::append_jsonl(path, event) {
                self.ledger_append_failures.fetch_add(1, Ordering::AcqRel);
                tracing::warn!(path = %path.display(), error = %err, "failed to append execution ledger event");
            } else {
                self.ledger_append_successes.fetch_add(1, Ordering::AcqRel);
                self.ledger_last_append_at_ms
                    .store(common::time::now_ms(), Ordering::Release);
            }
        }
    }

    fn append_order_snapshot(&self, record: &OrderRecord) {
        if let Some(store) = &self.sql_ledger_store {
            store.append_order_snapshot(record);
        }
        if let Some(path) = &self.order_snapshot_path {
            let _guard = self.order_snapshot_append_lock.lock();
            if let Err(err) = order_store::append_jsonl(path, record) {
                self.order_snapshot_append_failures
                    .fetch_add(1, Ordering::AcqRel);
                tracing::warn!(path = %path.display(), error = %err, "failed to append order snapshot");
            } else {
                self.order_snapshot_append_successes
                    .fetch_add(1, Ordering::AcqRel);
                self.order_snapshot_last_append_at_ms
                    .store(common::time::now_ms(), Ordering::Release);
            }
        }
    }
}
