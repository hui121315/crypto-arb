impl OrderJournal {
    pub fn apply_order_info_by_client_order_id_from_source(
        &self,
        client_order_id: &str,
        info: &OrderInfo,
        at_ms: i64,
        source: OrderUpdateSource,
    ) -> Option<OrderRecord> {
        let internal_order_id = self
            .client_index
            .get(client_order_id)
            .map(|entry| entry.clone())?;
        self.apply_order_info_from_source(&internal_order_id, info, at_ms, source)
    }

    pub fn apply_order_info_by_exchange_order_id(
        &self,
        exchange_order_id: &str,
        info: &OrderInfo,
        at_ms: i64,
    ) -> Option<OrderRecord> {
        self.apply_order_info_by_exchange_order_id_from_source(
            exchange_order_id,
            info,
            at_ms,
            OrderUpdateSource::OrderQuery,
        )
    }

    pub fn apply_order_info_by_exchange_order_id_from_source(
        &self,
        exchange_order_id: &str,
        info: &OrderInfo,
        at_ms: i64,
        source: OrderUpdateSource,
    ) -> Option<OrderRecord> {
        let internal_order_id = self
            .exchange_index
            .get(exchange_order_id)
            .map(|entry| entry.clone())?;
        self.apply_order_info_from_source(&internal_order_id, info, at_ms, source)
    }

    pub fn record_fill_by_exchange_order_id(
        &self,
        exchange_order_id: &str,
        input: &FillLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_fill_by_order_identity(
            FillOrderIdentity {
                venue: None,
                exchange_order_id: Some(exchange_order_id),
                client_order_id: None,
                symbol: None,
                side: None,
            },
            input,
            source,
            captured_at_ms,
        )
        .into_iter()
        .find(|event| event.event_type == shared_types::ExecutionLedgerEventType::FillEvent)
    }

    pub fn record_fill_by_order_identity(
        &self,
        identity: FillOrderIdentity<'_>,
        input: &FillLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Vec<ExecutionLedgerEvent> {
        self.record_fill_by_order_identity_with_sql_mode(FillLedgerRecordRequest {
            identity,
            input,
            source,
            captured_at_ms,
            transport_metadata: None,
            sql_mode: LedgerSqlWriteMode::BestEffort,
        })
    }

    fn record_fill_by_order_identity_with_sql_mode(
        &self,
        request: FillLedgerRecordRequest<'_>,
    ) -> Vec<ExecutionLedgerEvent> {
        let Some(record) = self.fill_record_for_identity(&request.identity) else {
            return Vec::new();
        };
        let context = self.ledger_context_for(&record);
        let events = {
            let _guard = self.ledger_append_lock.lock();
            let event_id = fill_venue_event_id(&record, request.input, request.source);
            let (fill_event, mut events) = if let Some(fill_event) =
                self.execution_ledger.get(&event_id)
            {
                (fill_event, Vec::with_capacity(1))
            } else {
                let Some(fill_event) = self
                    .execution_ledger
                    .record_fill_event_with_context_and_metadata(
                        &record,
                        request.input,
                        request.source,
                        request.captured_at_ms,
                        FillLedgerEventContext::new(context.as_ref(), request.transport_metadata),
                    )
                else {
                    return Vec::new();
                };
                (fill_event.clone(), vec![fill_event])
            };
            let slippage_event_id = format!("slippage:{}", fill_event.event_id);
            if !self.execution_ledger.contains_event(&slippage_event_id) {
                if let Some(slippage_event) = self.record_slippage_for_fill(FillSlippageRecord {
                    record: &record,
                    fill_event: &fill_event,
                    context: context.as_ref(),
                }) {
                    events.push(slippage_event);
                }
            }
            events
        };
        for event in &events {
            self.append_ledger_event_with_sql_mode(event, request.sql_mode);
        }
        events
    }

    fn record_slippage_for_fill(
        &self,
        request: FillSlippageRecord<'_>,
    ) -> Option<ExecutionLedgerEvent> {
        let slippage = slippage_input_for_fill(request.record, request.fill_event)?;
        self.execution_ledger.record_slippage_event_with_context(
            request.record,
            &slippage,
            request.fill_event.source,
            request.fill_event.captured_at_ms,
            request.context,
        )
    }

    pub fn fill_record_for_identity(&self, identity: &FillOrderIdentity<'_>) -> Option<OrderRecord> {
        let by_exchange = clean_identity_text(identity.exchange_order_id)
            .and_then(|id| self.get_by_exchange_order_id(id));
        let by_client = clean_identity_text(identity.client_order_id)
            .and_then(|id| self.get_by_client_order_id(id));
        let record = matching_identity_record(by_exchange, by_client)?;
        fill_identity_matches_record(&record, identity).then_some(record)
    }

    pub fn record_funding_by_venue_symbol_reported(
        &self,
        venue: &str,
        symbol: &str,
        input: &FundingLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Result<ExecutionLedgerEvent, FundingPaymentIngestSkipReason> {
        self.record_funding_by_venue_symbol_reported_with_sql_mode(FundingLedgerRecordRequest {
            venue,
            symbol,
            input,
            source,
            captured_at_ms,
            sql_mode: LedgerSqlWriteMode::BestEffort,
        })
    }

    fn record_funding_by_venue_symbol_reported_with_sql_mode(
        &self,
        request: FundingLedgerRecordRequest<'_>,
    ) -> Result<ExecutionLedgerEvent, FundingPaymentIngestSkipReason> {
        let _guard = self.ledger_append_lock.lock();
        let record =
            self.funding_record_for(request.venue, request.symbol, request.input.funding_time_ms)?;
        let context = self.ledger_context_for(&record);
        let event = self
            .execution_ledger
            .record_funding_payment_with_context(
                &record,
                request.input,
                request.source,
                request.captured_at_ms,
                context.as_ref(),
            )
            .ok_or(FundingPaymentIngestSkipReason::DuplicateOrAlreadyRecorded)?;
        self.append_ledger_event_with_sql_mode_locked(&event, request.sql_mode);
        Ok(event)
    }

    pub fn update_state(
        &self,
        internal_order_id: &str,
        state: LiveOrderState,
        message: Option<String>,
        at_ms: i64,
    ) -> Option<OrderRecord> {
        self.update_state_from_source(
            internal_order_id,
            state,
            message,
            at_ms,
            OrderUpdateSource::Manual,
        )
    }

    pub fn update_state_from_source(
        &self,
        internal_order_id: &str,
        state: LiveOrderState,
        message: Option<String>,
        at_ms: i64,
        source: OrderUpdateSource,
    ) -> Option<OrderRecord> {
        let event = event_from_target_state(state)?;
        self.transition_update(
            TransitionUpdate {
                internal_order_id,
                event,
                source,
                at_ms,
                message: message.clone(),
                payload: serde_json::Value::Null,
            },
            |r| {
                r.message = message;
            },
        )
    }

    pub fn update_state_by_exchange_order_id_from_source(
        &self,
        exchange_order_id: &str,
        state: LiveOrderState,
        message: Option<String>,
        at_ms: i64,
        source: OrderUpdateSource,
    ) -> Option<OrderRecord> {
        let internal_order_id = self
            .exchange_index
            .get(exchange_order_id)
            .map(|entry| entry.clone())?;
        if self.get(&internal_order_id)?.state == state {
            return self.get(&internal_order_id);
        }
        let event = event_from_target_state(state)?;
        self.transition_update(
            TransitionUpdate {
                internal_order_id: &internal_order_id,
                event,
                source,
                at_ms,
                message: message.clone(),
                payload: serde_json::Value::Null,
            },
            |record| {
                record.message = message;
            },
        )
    }
}

fn fill_venue_event_id(
    record: &OrderRecord,
    input: &FillLedgerInput,
    source: OrderUpdateSource,
) -> String {
    format!(
        "fill_event:{}:{}:{}",
        record.intent.id,
        ledger_source_key(source),
        input.venue_event_id
    )
}

const fn ledger_source_key(source: OrderUpdateSource) -> &'static str {
    match source {
        OrderUpdateSource::Unknown => "unknown",
        OrderUpdateSource::Internal => "internal",
        OrderUpdateSource::AdapterAck => "adapter_ack",
        OrderUpdateSource::OrderQuery => "order_query",
        OrderUpdateSource::PrivateWs => "private_ws",
        OrderUpdateSource::FundingPoller => "funding_poller",
        OrderUpdateSource::Reconcile => "reconcile",
        OrderUpdateSource::Manual => "manual",
    }
}
