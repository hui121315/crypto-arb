impl OrderJournal {
    fn restore_order_snapshots(&self, records: &[OrderRecord]) {
        for record in records {
            self.restore_order_snapshot(record);
        }
    }

    fn restore_order_snapshot(&self, record: &OrderRecord) {
        if self
            .records
            .get(&record.intent.id)
            .is_some_and(|current| !snapshot_should_replace(&current, record))
        {
            return;
        }
        let previous = self
            .records
            .insert(record.intent.id.clone(), record.clone());
        if let Some(previous) = previous.as_ref() {
            self.remove_record_indexes(previous);
        }
        self.apply_open_count_delta(previous.as_ref().map(|row| row.state), record.state);
        self.index_record(record);
    }

    fn record_ledger_query_success(&self) {
        self.ledger_query_successes.fetch_add(1, Ordering::AcqRel);
        self.ledger_last_query_at_ms
            .store(common::time::now_ms(), Ordering::Release);
    }

    fn record_ledger_query_failure(&self) {
        self.ledger_query_failures.fetch_add(1, Ordering::AcqRel);
        self.ledger_last_query_at_ms
            .store(common::time::now_ms(), Ordering::Release);
    }

    fn reindex_record(&self, previous: &OrderRecord, next: &OrderRecord) {
        self.remove_record_indexes(previous);
        self.index_record(next);
    }

    fn index_record(&self, record: &OrderRecord) {
        let identity = record.identity_snapshot();
        self.index_client_id(
            &identity.public_client_order_id,
            &identity.internal_order_id,
        );
        if let Some(venue_client_order_id) = identity.venue_client_order_id.as_deref() {
            self.index_client_id(venue_client_order_id, &identity.internal_order_id);
        }
        if let Some(exchange_order_id) = identity.exchange_order_id.as_deref() {
            self.index_exchange_id(exchange_order_id, &identity.internal_order_id);
        }
    }

    fn remove_record_indexes(&self, record: &OrderRecord) {
        let identity = record.identity_snapshot();
        self.remove_client_index(
            &identity.public_client_order_id,
            &identity.internal_order_id,
        );
        if let Some(venue_client_order_id) = identity.venue_client_order_id.as_deref() {
            self.remove_client_index(venue_client_order_id, &identity.internal_order_id);
        }
        if let Some(exchange_order_id) = identity.exchange_order_id.as_deref() {
            self.remove_exchange_index(exchange_order_id, &identity.internal_order_id);
        }
    }

    fn index_client_id(&self, client_order_id: &str, internal_order_id: &str) {
        if !client_order_id.is_empty() {
            self.client_index
                .insert(client_order_id.to_owned(), internal_order_id.to_owned());
        }
    }

    fn index_exchange_id(&self, exchange_order_id: &str, internal_order_id: &str) {
        if !exchange_order_id.is_empty() {
            self.exchange_index
                .insert(exchange_order_id.to_owned(), internal_order_id.to_owned());
        }
    }

    fn remove_client_index(&self, client_order_id: &str, internal_order_id: &str) {
        let should_remove = self
            .client_index
            .get(client_order_id)
            .is_some_and(|entry| entry.value() == internal_order_id);
        if should_remove {
            self.client_index.remove(client_order_id);
        }
    }

    fn remove_exchange_index(&self, exchange_order_id: &str, internal_order_id: &str) {
        let should_remove = self
            .exchange_index
            .get(exchange_order_id)
            .is_some_and(|entry| entry.value() == internal_order_id);
        if should_remove {
            self.exchange_index.remove(exchange_order_id);
        }
    }
}

fn snapshot_should_replace(current: &OrderRecord, candidate: &OrderRecord) -> bool {
    match (
        crate::state_machine::is_terminal(current.state),
        crate::state_machine::is_terminal(candidate.state),
    ) {
        (false, true) => true,
        (true, false) => false,
        _ => candidate.updated_at_ms >= current.updated_at_ms,
    }
}

fn order_matches_filter(
    record: &OrderRecord,
    state: Option<LiveOrderState>,
    since_ms: Option<i64>,
) -> bool {
    if state.is_some_and(|expected| record.state != expected) {
        return false;
    }
    if since_ms.is_some_and(|minimum| record.updated_at_ms < minimum) {
        return false;
    }
    true
}

fn funding_candidate(
    record: &OrderRecord,
    venue: &str,
    symbol_key: &str,
    funding_time_ms: i64,
) -> bool {
    record.intent.source == shared_types::OrderSource::ArbitragePreview
        && !record.intent.reduce_only
        && record.intent.created_at_ms <= funding_time_ms
        && event_venue_matches(&record.intent.exchange, Some(venue))
        && funding_symbol_key(&record.intent.symbol) == symbol_key
}

fn funding_record_is_newer(candidate: &OrderRecord, current: &OrderRecord) -> bool {
    (candidate.updated_at_ms, candidate.intent.id.as_str())
        > (current.updated_at_ms, current.intent.id.as_str())
}

fn funding_symbol_key(symbol: &str) -> String {
    let trimmed = symbol.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let scoped = trimmed.rsplit_once(':').map_or(trimmed, |(_, base)| base);
    let base = scoped
        .split(['-', '_', '/'])
        .next()
        .unwrap_or(scoped)
        .trim();
    strip_quote_suffix(base).to_ascii_uppercase()
}

fn strip_quote_suffix(symbol: &str) -> &str {
    ["USDTM", "USDCM", "USDT", "USDC", "USD", "PERP"]
        .into_iter()
        .find_map(|quote| symbol.strip_suffix(quote))
        .filter(|base| !base.is_empty())
        .unwrap_or(symbol)
}

fn clean_identity_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn matching_identity_record(
    by_exchange: Option<OrderRecord>,
    by_client: Option<OrderRecord>,
) -> Option<OrderRecord> {
    match (by_exchange, by_client) {
        (Some(exchange), Some(client)) if exchange.intent.id == client.intent.id => Some(exchange),
        (Some(_), Some(_)) => None,
        (Some(exchange), None) => Some(exchange),
        (None, Some(client)) => Some(client),
        (None, None) => None,
    }
}

fn fill_identity_matches_record(record: &OrderRecord, identity: &FillOrderIdentity<'_>) -> bool {
    event_venue_matches(&record.intent.exchange, identity.venue)
        && fill_symbol_matches(&record.intent.symbol, identity.symbol)
        && identity.side.is_none_or(|side| record.intent.side == side)
}

fn slippage_input_for_fill(
    record: &OrderRecord,
    fill_event: &ExecutionLedgerEvent,
) -> Option<SlippageLedgerInput> {
    let ExecutionLedgerPayload::FillSnapshot(fill) = &fill_event.payload else {
        return None;
    };
    let reference_price = positive_option(record.intent.price)?;
    let fill_price = positive_value(fill.average_price)?;
    let quantity = positive_value(fill.quantity)?;
    let amount_usd = match record.intent.side {
        OrderSide::Buy => quantity * (fill_price - reference_price),
        OrderSide::Sell => quantity * (reference_price - fill_price),
    };
    amount_usd.is_finite().then(|| SlippageLedgerInput {
        source_event_id: fill_event.event_id.clone(),
        amount_usd,
        reference_price,
        fill_price,
        quantity,
        occurred_at_ms: fill_event.occurred_at_ms,
    })
}

fn event_venue_matches(record_venue: &str, event_venue: Option<&str>) -> bool {
    let Some(event_venue) = clean_identity_text(event_venue) else {
        return true;
    };
    record_venue.eq_ignore_ascii_case(event_venue)
        || (event_venue.eq_ignore_ascii_case("hyperliquid")
            && record_venue
                .to_ascii_lowercase()
                .starts_with("hyperliquid:"))
}

fn fill_symbol_matches(record_symbol: &str, fill_symbol: Option<&str>) -> bool {
    let Some(fill_symbol) = clean_identity_text(fill_symbol) else {
        return true;
    };
    let record_key = funding_symbol_key(record_symbol);
    let fill_key = funding_symbol_key(fill_symbol);
    !record_key.is_empty() && record_key == fill_key
}

fn hedge_group_id(id: &str) -> String {
    id.strip_suffix("-long")
        .or_else(|| id.strip_suffix("-short"))
        .or_else(|| id.strip_suffix("-unwind"))
        .unwrap_or(id)
        .to_owned()
}

fn invalid_ledger_query(query: &ExecutionLedgerQuery) -> bool {
    query.limit == 0
        || query
            .to_ms
            .zip(query.from_ms)
            .is_some_and(|(to, from)| to <= from)
}

fn enrich_intent_policy(intent: &mut OrderIntent) {
    if intent.client_order_id_policy.is_none() {
        intent.client_order_id_policy = Some(exchange::client_order_id_policy(
            &intent.exchange,
            &intent.client_order_id,
        ));
    }
}

fn apply_order_info_fields(record: &mut OrderRecord, info: &OrderInfo) {
    record.identity = record.identity_snapshot();
    record.identity.record_exchange_order_id(&info.order_id);
    record.exchange_order_id = record.identity.exchange_order_id.clone();
    record.message = Some(format!("order status backfill: {:?}", info.status));
    if info.filled_quantity.is_finite() && info.filled_quantity > 0.0 {
        record.filled_quantity = Some(info.filled_quantity);
    }
    if info.filled_price.is_finite() && info.filled_price > 0.0 {
        record.filled_price = Some(info.filled_price);
    }
    if info.fees.is_finite() && info.fees.abs() > f64::EPSILON {
        record.filled_fee = Some(info.fees);
    }
}
