use dashmap::DashMap;
use shared_types::{
    ExecutionFillConfidence, ExecutionLedgerEvent, ExecutionLedgerEventType,
    ExecutionLedgerOrderRef, ExecutionLedgerPayload, ExecutionLedgerQuality, FeeLedgerSnapshot,
    FillLedgerSnapshot, FundingPaymentLedgerRecord, HedgeLegRole, OrderEventRecord, OrderInfo,
    OrderLifecycleEvent, OrderRecord, OrderTransportMetadata, OrderUpdateSource,
    OrderbookDepthLedgerRecord, SlippageLedgerRecord,
};
use std::collections::BTreeSet;

#[derive(Debug, Default)]
pub struct ExecutionLedger {
    events: DashMap<String, ExecutionLedgerEvent>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FillLedgerInput {
    pub venue_event_id: String,
    pub quantity: f64,
    pub price: f64,
    pub fee_amount: Option<f64>,
    pub fee_currency: Option<String>,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FundingLedgerInput {
    pub venue_event_id: String,
    pub amount: f64,
    pub currency: String,
    pub funding_time_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SlippageLedgerInput {
    pub source_event_id: String,
    pub amount_usd: f64,
    pub reference_price: f64,
    pub fill_price: f64,
    pub quantity: f64,
    pub occurred_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderbookDepthLedgerInput {
    pub reference_price: Option<f64>,
    pub bid: Option<f64>,
    pub ask: Option<f64>,
    pub mid: Option<f64>,
    pub open_vwap_price: Option<f64>,
    pub open_slippage_bps: Option<f64>,
    pub close_vwap_price: Option<f64>,
    pub close_slippage_bps: Option<f64>,
    pub depth_usd_5bps: Option<f64>,
    pub depth_usd_10bps: Option<f64>,
    pub depth_usd_20bps: Option<f64>,
    pub max_notional_usd: Option<f64>,
    pub market_timestamp_ms: Option<i64>,
    pub health: Option<shared_types::MarketDataHealth>,
    pub reason: Option<String>,
    pub quality: ExecutionLedgerQuality,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionLedgerOrderContext {
    pub run_id: String,
    pub ticket_id: String,
    pub leg_role: HedgeLegRole,
}

impl ExecutionLedgerOrderContext {
    pub fn new(run_id: String, ticket_id: String, leg_role: HedgeLegRole) -> Self {
        Self {
            run_id,
            ticket_id,
            leg_role,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FillLedgerEventContext<'a> {
    order_context: Option<&'a ExecutionLedgerOrderContext>,
    transport_metadata: Option<&'a OrderTransportMetadata>,
}

impl<'a> FillLedgerEventContext<'a> {
    pub(crate) const fn new(
        order_context: Option<&'a ExecutionLedgerOrderContext>,
        transport_metadata: Option<&'a OrderTransportMetadata>,
    ) -> Self {
        Self {
            order_context,
            transport_metadata,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExecutionLedgerQuery {
    pub internal_order_id: Option<String>,
    pub exchange_order_id: Option<String>,
    pub hedge_group_id: Option<String>,
    pub run_id: Option<String>,
    pub ticket_id: Option<String>,
    pub leg_role: Option<HedgeLegRole>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
}

impl ExecutionLedger {
    pub fn from_events(events: impl IntoIterator<Item = ExecutionLedgerEvent>) -> Self {
        let ledger = Self::default();
        for event in events {
            ledger.upsert(event);
        }
        ledger
    }

    pub fn list(&self) -> Vec<ExecutionLedgerEvent> {
        let mut events: Vec<_> = self
            .events
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        sort_events(&mut events);
        events
    }

    pub fn upsert(&self, event: ExecutionLedgerEvent) {
        self.events.insert(event.event_id.clone(), event);
    }

    pub fn contains_event(&self, event_id: &str) -> bool {
        self.events.contains_key(event_id)
    }

    pub(crate) fn get(&self, event_id: &str) -> Option<ExecutionLedgerEvent> {
        self.events.get(event_id).map(|entry| entry.value().clone())
    }

    pub fn realized_window_events(&self, from_ms: i64, to_ms: i64) -> Vec<ExecutionLedgerEvent> {
        if to_ms <= from_ms {
            return Vec::new();
        }
        let groups = self.realized_window_groups(from_ms, to_ms);
        if groups.is_empty() {
            return Vec::new();
        }
        let mut events = self
            .events
            .iter()
            .filter(|entry| {
                let event = entry.value();
                event.occurred_at_ms < to_ms
                    && is_realized_payload(&event.payload)
                    && groups.contains(&event_group_id(event))
            })
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        sort_events(&mut events);
        events
    }

    pub fn query(&self, query: &ExecutionLedgerQuery) -> Vec<ExecutionLedgerEvent> {
        let limit = query.limit.max(1);
        let mut events = self
            .events
            .iter()
            .filter(|entry| query_matches_event(query, entry.value()))
            .map(|entry| entry.value().clone())
            .collect::<Vec<_>>();
        sort_events_desc(&mut events);
        events.truncate(limit);
        sort_events(&mut events);
        events
    }

    pub fn record_fill_snapshot(
        &self,
        record: &OrderRecord,
        info: &OrderInfo,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_fill_snapshot_with_context(record, info, source, captured_at_ms, None)
    }

    pub fn record_fill_snapshot_with_context(
        &self,
        record: &OrderRecord,
        info: &OrderInfo,
        source: OrderUpdateSource,
        captured_at_ms: i64,
        context: Option<&ExecutionLedgerOrderContext>,
    ) -> Option<ExecutionLedgerEvent> {
        let payload = fill_payload(info, source, ExecutionLedgerEventType::FillSnapshot)?;
        let event = ExecutionLedgerEvent {
            event_id: fill_event_id(record, info, source),
            event_type: ExecutionLedgerEventType::FillSnapshot,
            source,
            order: order_ref(record, context),
            payload,
            occurred_at_ms: captured_at_ms,
            captured_at_ms,
        };
        self.upsert(event.clone());
        Some(event)
    }

    pub fn record_record_fill_snapshot(
        &self,
        record: &OrderRecord,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_record_fill_snapshot_with_context(record, source, captured_at_ms, None)
    }

    pub fn record_record_fill_snapshot_with_context(
        &self,
        record: &OrderRecord,
        source: OrderUpdateSource,
        captured_at_ms: i64,
        context: Option<&ExecutionLedgerOrderContext>,
    ) -> Option<ExecutionLedgerEvent> {
        let payload = record_fill_payload(record, source, ExecutionLedgerEventType::FillSnapshot)?;
        let event = ExecutionLedgerEvent {
            event_id: record_fill_event_id(record, source),
            event_type: ExecutionLedgerEventType::FillSnapshot,
            source,
            order: order_ref(record, context),
            payload,
            occurred_at_ms: captured_at_ms,
            captured_at_ms,
        };
        self.upsert(event.clone());
        Some(event)
    }

    pub fn record_fill_event(
        &self,
        record: &OrderRecord,
        input: &FillLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_fill_event_with_context(record, input, source, captured_at_ms, None)
    }

    pub(crate) fn record_fill_event_with_context(
        &self,
        record: &OrderRecord,
        input: &FillLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
        context: Option<&ExecutionLedgerOrderContext>,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_fill_event_with_context_and_metadata(
            record,
            input,
            source,
            captured_at_ms,
            FillLedgerEventContext::new(context, None),
        )
    }

    pub(crate) fn record_fill_event_with_context_and_metadata(
        &self,
        record: &OrderRecord,
        input: &FillLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
        context: FillLedgerEventContext<'_>,
    ) -> Option<ExecutionLedgerEvent> {
        let mut payload = fill_payload_from_values(
            input.quantity,
            input.price,
            input.fee_amount,
            fill_confidence(source, ExecutionLedgerEventType::FillEvent),
        )?;
        // Keep explicit fees, including zero and tiny amounts; absent fees remain unknown.
        if let Some(amount) = input.fee_amount.filter(|amount| amount.is_finite()) {
            if let ExecutionLedgerPayload::FillSnapshot(fill) = &mut payload {
                fill.fee = Some(FeeLedgerSnapshot {
                    amount,
                    currency: input.fee_currency.clone(),
                    quality: ExecutionLedgerQuality::Actual,
                });
            }
        }
        let mut order = order_ref(record, context.order_context);
        if let Some(metadata) = context.transport_metadata {
            order.identity.transport_metadata.merge_from(metadata);
        }
        let event = ExecutionLedgerEvent {
            event_id: external_fill_event_id(record, input, source),
            event_type: ExecutionLedgerEventType::FillEvent,
            source,
            order,
            payload: with_fee_currency(payload, input.fee_currency.clone()),
            occurred_at_ms: input.occurred_at_ms,
            captured_at_ms,
        };
        self.upsert(event.clone());
        Some(event)
    }

    pub fn record_funding_payment(
        &self,
        record: &OrderRecord,
        input: &FundingLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_funding_payment_with_context(record, input, source, captured_at_ms, None)
    }

    pub fn record_funding_payment_with_context(
        &self,
        record: &OrderRecord,
        input: &FundingLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
        context: Option<&ExecutionLedgerOrderContext>,
    ) -> Option<ExecutionLedgerEvent> {
        let payload = funding_payload(input)?;
        let event = ExecutionLedgerEvent {
            event_id: funding_event_id(record, input, source),
            event_type: ExecutionLedgerEventType::FundingPayment,
            source,
            order: order_ref(record, context),
            payload,
            occurred_at_ms: input.funding_time_ms,
            captured_at_ms,
        };
        if self.contains_event(&event.event_id) {
            return None;
        }
        self.upsert(event.clone());
        Some(event)
    }

    pub fn record_slippage_event(
        &self,
        record: &OrderRecord,
        input: &SlippageLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_slippage_event_with_context(record, input, source, captured_at_ms, None)
    }

    pub fn record_slippage_event_with_context(
        &self,
        record: &OrderRecord,
        input: &SlippageLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
        context: Option<&ExecutionLedgerOrderContext>,
    ) -> Option<ExecutionLedgerEvent> {
        let payload = slippage_payload(input)?;
        let event = ExecutionLedgerEvent {
            event_id: slippage_event_id(input),
            event_type: ExecutionLedgerEventType::Slippage,
            source,
            order: order_ref(record, context),
            payload,
            occurred_at_ms: input.occurred_at_ms,
            captured_at_ms,
        };
        self.upsert(event.clone());
        Some(event)
    }

    pub fn record_orderbook_evidence(
        &self,
        record: &OrderRecord,
        input: &OrderbookDepthLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_orderbook_evidence_with_context(record, input, source, captured_at_ms, None)
    }

    pub fn record_orderbook_evidence_with_context(
        &self,
        record: &OrderRecord,
        input: &OrderbookDepthLedgerInput,
        source: OrderUpdateSource,
        captured_at_ms: i64,
        context: Option<&ExecutionLedgerOrderContext>,
    ) -> Option<ExecutionLedgerEvent> {
        let payload = orderbook_payload(input)?;
        let event = ExecutionLedgerEvent {
            event_id: orderbook_event_id(record, input, source),
            event_type: ExecutionLedgerEventType::OrderbookEvidence,
            source,
            order: order_ref(record, context),
            payload,
            occurred_at_ms: input
                .market_timestamp_ms
                .or_else(|| input.health.as_ref().map(|health| health.observed_at_ms))
                .unwrap_or(captured_at_ms),
            captured_at_ms,
        };
        self.upsert(event.clone());
        Some(event)
    }

    pub fn record_order_state(
        &self,
        record: &OrderRecord,
        lifecycle: &OrderEventRecord,
    ) -> Option<ExecutionLedgerEvent> {
        self.record_order_state_with_context(record, lifecycle, None)
    }

    pub fn record_order_state_with_context(
        &self,
        record: &OrderRecord,
        lifecycle: &OrderEventRecord,
        context: Option<&ExecutionLedgerOrderContext>,
    ) -> Option<ExecutionLedgerEvent> {
        let event_type = state_event_type(lifecycle.event)?;
        let event = ExecutionLedgerEvent {
            event_id: state_event_id(record, lifecycle),
            event_type,
            source: lifecycle.source,
            order: order_ref(record, context),
            payload: ExecutionLedgerPayload::OrderState {
                state: lifecycle.state,
                message: lifecycle.message.clone(),
            },
            occurred_at_ms: lifecycle.occurred_at_ms,
            captured_at_ms: lifecycle.occurred_at_ms,
        };
        self.upsert(event.clone());
        Some(event)
    }
}

impl ExecutionLedger {
    fn realized_window_groups(&self, from_ms: i64, to_ms: i64) -> BTreeSet<String> {
        self.events
            .iter()
            .filter(|entry| {
                let event = entry.value();
                event.occurred_at_ms >= from_ms
                    && event.occurred_at_ms < to_ms
                    && is_fill_payload(&event.payload)
            })
            .map(|entry| event_group_id(entry.value()))
            .collect()
    }
}

fn sort_events(events: &mut [ExecutionLedgerEvent]) {
    events.sort_by(|left, right| {
        left.occurred_at_ms
            .cmp(&right.occurred_at_ms)
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
}

fn sort_events_desc(events: &mut [ExecutionLedgerEvent]) {
    events.sort_by(|left, right| {
        right
            .occurred_at_ms
            .cmp(&left.occurred_at_ms)
            .then_with(|| right.event_id.cmp(&left.event_id))
    });
}

fn query_matches_event(query: &ExecutionLedgerQuery, event: &ExecutionLedgerEvent) -> bool {
    query_matches_time(query, event)
        && query_matches_internal_order(query, event)
        && query_matches_exchange_order(query, event)
        && query_matches_group(query, event)
        && query_matches_run(query, event)
        && query_matches_ticket(query, event)
        && query_matches_leg_role(query, event)
}

fn query_matches_time(query: &ExecutionLedgerQuery, event: &ExecutionLedgerEvent) -> bool {
    if query
        .from_ms
        .is_some_and(|from_ms| event.occurred_at_ms < from_ms)
    {
        return false;
    }
    if query
        .to_ms
        .is_some_and(|to_ms| event.occurred_at_ms >= to_ms)
    {
        return false;
    }
    true
}

fn query_matches_internal_order(
    query: &ExecutionLedgerQuery,
    event: &ExecutionLedgerEvent,
) -> bool {
    query
        .internal_order_id
        .as_deref()
        .is_none_or(|id| event.order.identity.internal_order_id == id)
}

fn query_matches_exchange_order(
    query: &ExecutionLedgerQuery,
    event: &ExecutionLedgerEvent,
) -> bool {
    query
        .exchange_order_id
        .as_deref()
        .is_none_or(|id| event.order.identity.exchange_order_id.as_deref() == Some(id))
}

fn query_matches_group(query: &ExecutionLedgerQuery, event: &ExecutionLedgerEvent) -> bool {
    query
        .hedge_group_id
        .as_deref()
        .is_none_or(|id| event_group_id(event) == id)
}

fn query_matches_run(query: &ExecutionLedgerQuery, event: &ExecutionLedgerEvent) -> bool {
    query
        .run_id
        .as_deref()
        .is_none_or(|id| event.order.run_id.as_deref() == Some(id))
}

fn query_matches_ticket(query: &ExecutionLedgerQuery, event: &ExecutionLedgerEvent) -> bool {
    query
        .ticket_id
        .as_deref()
        .is_none_or(|id| event.order.ticket_id.as_deref() == Some(id))
}

fn query_matches_leg_role(query: &ExecutionLedgerQuery, event: &ExecutionLedgerEvent) -> bool {
    query
        .leg_role
        .is_none_or(|role| event.order.leg_role == Some(role))
}

fn event_group_id(event: &ExecutionLedgerEvent) -> String {
    hedge_group_id(&event.order.identity.internal_order_id)
}

fn hedge_group_id(id: &str) -> String {
    id.strip_suffix("-long")
        .or_else(|| id.strip_suffix("-short"))
        .or_else(|| id.strip_suffix("-unwind"))
        .unwrap_or(id)
        .to_owned()
}

fn is_realized_payload(payload: &ExecutionLedgerPayload) -> bool {
    matches!(
        payload,
        ExecutionLedgerPayload::FillSnapshot(_)
            | ExecutionLedgerPayload::FeeSnapshot(_)
            | ExecutionLedgerPayload::FundingPayment(_)
            | ExecutionLedgerPayload::Slippage(_)
            | ExecutionLedgerPayload::OrderbookEvidence(_)
    )
}

fn is_fill_payload(payload: &ExecutionLedgerPayload) -> bool {
    matches!(payload, ExecutionLedgerPayload::FillSnapshot(_))
}

fn state_event_type(event: OrderLifecycleEvent) -> Option<ExecutionLedgerEventType> {
    match event {
        OrderLifecycleEvent::Submitted
        | OrderLifecycleEvent::AdapterAccepted
        | OrderLifecycleEvent::AdapterRejected
        | OrderLifecycleEvent::AdapterFailed
        | OrderLifecycleEvent::ExchangeUnknown => Some(ExecutionLedgerEventType::OrderState),
        OrderLifecycleEvent::CancelRequest | OrderLifecycleEvent::CancelAck => {
            Some(ExecutionLedgerEventType::Cancel)
        }
        OrderLifecycleEvent::Created
        | OrderLifecycleEvent::RiskApproved
        | OrderLifecycleEvent::RiskRejected
        | OrderLifecycleEvent::PartialFill
        | OrderLifecycleEvent::FullFill
        | OrderLifecycleEvent::Timeout => None,
    }
}

fn fill_payload(
    info: &OrderInfo,
    source: OrderUpdateSource,
    event_type: ExecutionLedgerEventType,
) -> Option<ExecutionLedgerPayload> {
    fill_payload_from_values(
        info.filled_quantity,
        info.filled_price,
        Some(info.fees),
        fill_confidence(source, event_type),
    )
}

fn record_fill_payload(
    record: &OrderRecord,
    source: OrderUpdateSource,
    event_type: ExecutionLedgerEventType,
) -> Option<ExecutionLedgerPayload> {
    fill_payload_from_values(
        record.filled_quantity?,
        record.filled_price?,
        record.filled_fee,
        fill_confidence(source, event_type),
    )
}

fn fill_payload_from_values(
    quantity: f64,
    average_price: f64,
    fee_amount: Option<f64>,
    confidence: ExecutionFillConfidence,
) -> Option<ExecutionLedgerPayload> {
    if !is_positive(quantity) || !is_positive(average_price) {
        return None;
    }
    let quote_value = quantity * average_price;
    let fee = fee_amount.and_then(fee_snapshot);
    Some(ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
        quantity,
        average_price,
        quote_value,
        quality: ExecutionLedgerQuality::Actual,
        confidence,
        fee,
    }))
}

fn fill_confidence(
    source: OrderUpdateSource,
    event_type: ExecutionLedgerEventType,
) -> ExecutionFillConfidence {
    ExecutionFillConfidence::from_source(event_type, source)
}

fn with_fee_currency(
    payload: ExecutionLedgerPayload,
    currency: Option<String>,
) -> ExecutionLedgerPayload {
    match payload {
        ExecutionLedgerPayload::FillSnapshot(mut fill) => {
            if let Some(fee) = fill.fee.as_mut() {
                fee.currency = currency;
            }
            ExecutionLedgerPayload::FillSnapshot(fill)
        }
        other => other,
    }
}

fn funding_payload(input: &FundingLedgerInput) -> Option<ExecutionLedgerPayload> {
    let currency = input.currency.trim();
    if !input.amount.is_finite() || currency.is_empty() {
        return None;
    }
    Some(ExecutionLedgerPayload::FundingPayment(
        FundingPaymentLedgerRecord {
            amount: input.amount,
            currency: currency.to_owned(),
            funding_time_ms: input.funding_time_ms,
            quality: ExecutionLedgerQuality::Actual,
        },
    ))
}

fn slippage_payload(input: &SlippageLedgerInput) -> Option<ExecutionLedgerPayload> {
    if input.source_event_id.trim().is_empty()
        || !input.amount_usd.is_finite()
        || !is_positive(input.reference_price)
        || !is_positive(input.fill_price)
        || !is_positive(input.quantity)
    {
        return None;
    }
    Some(ExecutionLedgerPayload::Slippage(SlippageLedgerRecord {
        amount_usd: input.amount_usd,
        reference_price: input.reference_price,
        fill_price: input.fill_price,
        quantity: input.quantity,
        quality: ExecutionLedgerQuality::Actual,
    }))
}

fn orderbook_payload(input: &OrderbookDepthLedgerInput) -> Option<ExecutionLedgerPayload> {
    if !orderbook_input_has_evidence(input) {
        return None;
    }
    Some(ExecutionLedgerPayload::OrderbookEvidence(Box::new(
        OrderbookDepthLedgerRecord {
            reference_price: finite_option(input.reference_price),
            bid: finite_option(input.bid),
            ask: finite_option(input.ask),
            mid: finite_option(input.mid),
            open_vwap_price: finite_option(input.open_vwap_price),
            open_slippage_bps: finite_option(input.open_slippage_bps),
            close_vwap_price: finite_option(input.close_vwap_price),
            close_slippage_bps: finite_option(input.close_slippage_bps),
            depth_usd_5bps: finite_option(input.depth_usd_5bps),
            depth_usd_10bps: finite_option(input.depth_usd_10bps),
            depth_usd_20bps: finite_option(input.depth_usd_20bps),
            max_notional_usd: finite_option(input.max_notional_usd),
            market_timestamp_ms: input.market_timestamp_ms,
            health: input.health.clone(),
            reason: clean_optional_text(input.reason.as_deref()).map(str::to_owned),
            quality: input.quality,
        },
    )))
}

fn orderbook_input_has_evidence(input: &OrderbookDepthLedgerInput) -> bool {
    [
        input.reference_price,
        input.bid,
        input.ask,
        input.mid,
        input.open_vwap_price,
        input.open_slippage_bps,
        input.close_vwap_price,
        input.close_slippage_bps,
        input.depth_usd_5bps,
        input.depth_usd_10bps,
        input.depth_usd_20bps,
        input.max_notional_usd,
    ]
    .into_iter()
    .any(|value| value.is_some_and(f64::is_finite))
        || input.market_timestamp_ms.is_some()
        || input.health.is_some()
        || clean_optional_text(input.reason.as_deref()).is_some()
}

fn fee_snapshot(amount: f64) -> Option<FeeLedgerSnapshot> {
    (amount.is_finite() && amount.abs() > f64::EPSILON).then_some(FeeLedgerSnapshot {
        amount,
        currency: None,
        quality: ExecutionLedgerQuality::Actual,
    })
}

fn order_ref(
    record: &OrderRecord,
    context: Option<&ExecutionLedgerOrderContext>,
) -> ExecutionLedgerOrderRef {
    ExecutionLedgerOrderRef {
        run_id: context.map(|ctx| ctx.run_id.clone()),
        ticket_id: context.map(|ctx| ctx.ticket_id.clone()),
        leg_role: context.map(|ctx| ctx.leg_role),
        reduce_only: Some(record.intent.reduce_only),
        exchange: record.intent.exchange.clone(),
        symbol: record.intent.symbol.clone(),
        side: record.intent.side,
        identity: record.identity_snapshot(),
    }
}

fn fill_event_id(record: &OrderRecord, info: &OrderInfo, source: OrderUpdateSource) -> String {
    format!(
        "fill_snapshot:{}:{}:{}:{}:{}",
        record.intent.id,
        source_key(source),
        decimal_key(info.filled_quantity),
        decimal_key(info.filled_price),
        decimal_key(info.fees)
    )
}

fn record_fill_event_id(record: &OrderRecord, source: OrderUpdateSource) -> String {
    format!(
        "fill_snapshot:{}:{}:{}:{}:{}",
        record.intent.id,
        source_key(source),
        optional_decimal_key(record.filled_quantity),
        optional_decimal_key(record.filled_price),
        optional_decimal_key(record.filled_fee)
    )
}

fn external_fill_event_id(
    record: &OrderRecord,
    input: &FillLedgerInput,
    source: OrderUpdateSource,
) -> String {
    format!(
        "fill_event:{}:{}:{}",
        record.intent.id,
        source_key(source),
        input.venue_event_id
    )
}

fn funding_event_id(
    record: &OrderRecord,
    input: &FundingLedgerInput,
    source: OrderUpdateSource,
) -> String {
    format!(
        "funding_payment:{}:{}:{}",
        record.intent.id,
        source_key(source),
        input.venue_event_id
    )
}

fn slippage_event_id(input: &SlippageLedgerInput) -> String {
    format!("slippage:{}", input.source_event_id)
}

fn orderbook_event_id(
    record: &OrderRecord,
    input: &OrderbookDepthLedgerInput,
    source: OrderUpdateSource,
) -> String {
    format!(
        "orderbook_evidence:{}:{}:{}:{}:{}:{}",
        record.intent.id,
        source_key(source),
        optional_ms_key(
            input
                .market_timestamp_ms
                .or_else(|| input.health.as_ref().map(|health| health.observed_at_ms))
        ),
        optional_decimal_key(input.reference_price),
        optional_decimal_key(input.max_notional_usd),
        optional_decimal_key(input.depth_usd_20bps)
    )
}

fn state_event_id(record: &OrderRecord, lifecycle: &OrderEventRecord) -> String {
    format!(
        "order_state:{}:{}:{:?}:{:?}:{}",
        record.intent.id,
        source_key(lifecycle.source),
        lifecycle.event,
        lifecycle.state,
        lifecycle.occurred_at_ms
    )
}

fn source_key(source: OrderUpdateSource) -> &'static str {
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

fn decimal_key(value: f64) -> String {
    if value.is_finite() {
        format!("{value:.12}")
    } else {
        "nan".into()
    }
}

fn optional_decimal_key(value: Option<f64>) -> String {
    value.map(decimal_key).unwrap_or_else(|| "none".into())
}

fn optional_ms_key(value: Option<i64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "none".into())
}

fn finite_option(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite())
}

fn clean_optional_text(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn is_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        ExecutionFillConfidence, ExecutionMode, LiveOrderState, MarginMode, MarketDataHealth,
        MarketDataQuality, MarketDataSourceKind, OrderSide, OrderSource, OrderStatus, OrderType,
        TimeInForce, VenueOrderIdentity,
    };

    #[test]
    fn records_actual_fill_snapshot() {
        let ledger = ExecutionLedger::default();
        let record = order_record();
        let info = order_info();

        let event = ledger
            .record_fill_snapshot(&record, &info, OrderUpdateSource::PrivateWs, 10)
            .expect("ledger event");

        assert_eq!(event.source, OrderUpdateSource::PrivateWs);
        assert_eq!(event.order.identity.public_client_order_id, "client-1");
        assert!(matches!(
            event.payload,
            ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
                confidence: ExecutionFillConfidence::VenueOrderSnapshot,
                ..
            })
        ));
        assert_eq!(ledger.list().len(), 1);
    }

    #[test]
    fn ignores_missing_fill_snapshot() {
        let ledger = ExecutionLedger::default();
        let record = order_record();
        let info = OrderInfo {
            execution_style: None,
            venue_time_in_force: None,
            client_order_id: None,
            reduce_only: None,
            filled_quantity: 0.0,
            ..order_info()
        };

        assert!(ledger
            .record_fill_snapshot(&record, &info, OrderUpdateSource::OrderQuery, 10)
            .is_none());
        assert!(ledger.list().is_empty());
    }

    #[test]
    fn records_fill_snapshot_from_record_when_evidence_exists() {
        let ledger = ExecutionLedger::default();
        let record = order_record();

        let event = ledger
            .record_record_fill_snapshot(&record, OrderUpdateSource::AdapterAck, 11)
            .expect("record fill snapshot");

        assert_eq!(event.source, OrderUpdateSource::AdapterAck);
        assert!(matches!(
            event.payload,
            ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
                quantity: 0.5,
                average_price: 10.0,
                ..
            })
        ));
    }

    #[test]
    fn records_external_fill_event_with_fee_currency() {
        let ledger = ExecutionLedger::default();
        let record = order_record();
        let input = FillLedgerInput {
            venue_event_id: "hl-fill-1".into(),
            quantity: 0.25,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 20,
        };

        let event = ledger
            .record_fill_event(&record, &input, OrderUpdateSource::PrivateWs, 21)
            .expect("external fill event");

        assert_eq!(event.event_id, "fill_event:ord-1:private_ws:hl-fill-1");
        assert_eq!(event.event_type, ExecutionLedgerEventType::FillEvent);
        assert_eq!(event.occurred_at_ms, 20);
        assert_eq!(event.captured_at_ms, 21);
        assert!(matches!(
            event.payload,
            ExecutionLedgerPayload::FillSnapshot(FillLedgerSnapshot {
                quantity: 0.25,
                average_price: 100.0,
                confidence: ExecutionFillConfidence::VenueFill,
                fee: Some(FeeLedgerSnapshot {
                    amount: 0.01,
                    currency: Some(ref currency),
                    ..
                }),
                ..
            }) if currency == "USDC"
        ));
    }

    #[test]
    fn external_fill_preserves_explicit_zero_fee_without_inventing_missing_fees() {
        let ledger = ExecutionLedger::default();
        let record = order_record();
        for fee in [Some(0.0), None, Some(-0.01), Some(1e-20)] {
            let input = FillLedgerInput {
                venue_event_id: format!("fee-{fee:?}"),
                quantity: 1.0,
                price: 100.0,
                fee_amount: fee,
                fee_currency: Some("USD".into()),
                occurred_at_ms: 10,
            };
            let event = ledger
                .record_fill_event(&record, &input, OrderUpdateSource::PrivateWs, 11)
                .unwrap();
            let ExecutionLedgerPayload::FillSnapshot(fill) = event.payload else {
                panic!("fill expected");
            };
            assert_eq!(fill.fee.as_ref().map(|fee| fee.amount), fee);
            if let Some(fee) = fill.fee {
                assert_eq!(fee.currency.as_deref(), Some("USD"));
            }
        }
    }

    #[test]
    fn records_external_fill_context_and_transport_metadata() {
        let ledger = ExecutionLedger::default();
        let record = order_record();
        let input = FillLedgerInput {
            venue_event_id: "fill-with-context".into(),
            quantity: 0.25,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 20,
        };
        let order_context =
            ExecutionLedgerOrderContext::new("run-1".into(), "ticket-1".into(), HedgeLegRole::Long);
        let transport_metadata = OrderTransportMetadata::default()
            .with_native_transport("private_ws")
            .with_native_request_id("venue-fill-1");

        let event = ledger
            .record_fill_event_with_context_and_metadata(
                &record,
                &input,
                OrderUpdateSource::PrivateWs,
                21,
                FillLedgerEventContext::new(Some(&order_context), Some(&transport_metadata)),
            )
            .expect("external fill event");

        assert_eq!(event.order.run_id.as_deref(), Some("run-1"));
        assert_eq!(event.order.ticket_id.as_deref(), Some("ticket-1"));
        assert_eq!(event.order.leg_role, Some(HedgeLegRole::Long));
        assert_eq!(
            event
                .order
                .identity
                .transport_metadata
                .native_request_id
                .as_deref(),
            Some("venue-fill-1")
        );
    }

    #[test]
    fn order_ref_records_reduce_only_cost_phase() {
        let ledger = ExecutionLedger::default();
        let mut record = order_record_with_id("hedge-1-unwind", OrderSide::Sell);
        record.intent.reduce_only = true;
        let input = FillLedgerInput {
            venue_event_id: "unwind-fill-1".into(),
            quantity: 0.25,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 20,
        };

        let event = ledger
            .record_fill_event(&record, &input, OrderUpdateSource::PrivateWs, 21)
            .expect("external fill event");

        assert_eq!(event.order.reduce_only, Some(true));
    }

    #[test]
    fn fill_confidence_reflects_update_source() {
        assert_eq!(
            fill_confidence(
                OrderUpdateSource::PrivateWs,
                ExecutionLedgerEventType::FillEvent
            ),
            ExecutionFillConfidence::VenueFill
        );
        assert_eq!(
            fill_confidence(
                OrderUpdateSource::OrderQuery,
                ExecutionLedgerEventType::FillSnapshot
            ),
            ExecutionFillConfidence::OrderQuery
        );
        assert_eq!(
            fill_confidence(
                OrderUpdateSource::AdapterAck,
                ExecutionLedgerEventType::FillSnapshot
            ),
            ExecutionFillConfidence::AdapterAck
        );
    }

    #[test]
    fn records_funding_payment_for_order_group() {
        let ledger = ExecutionLedger::default();
        let record = order_record_with_id("hedge-1-long", OrderSide::Buy);
        let input = FundingLedgerInput {
            venue_event_id: "hl-funding:BTC:20".into(),
            amount: -0.12,
            currency: "USDC".into(),
            funding_time_ms: 20,
        };

        let event = ledger
            .record_funding_payment(&record, &input, OrderUpdateSource::PrivateWs, 21)
            .expect("funding ledger event");

        assert_eq!(
            event.event_id,
            "funding_payment:hedge-1-long:private_ws:hl-funding:BTC:20"
        );
        assert_eq!(event.event_type, ExecutionLedgerEventType::FundingPayment);
        assert_eq!(event.occurred_at_ms, 20);
        assert!(matches!(
            event.payload,
            ExecutionLedgerPayload::FundingPayment(FundingPaymentLedgerRecord {
                amount: -0.12,
                ref currency,
                ..
            }) if currency == "USDC"
        ));
    }

    #[test]
    fn records_funding_poller_provenance() {
        let ledger = ExecutionLedger::default();
        let record = order_record_with_id("hedge-1-long", OrderSide::Buy);
        let input = FundingLedgerInput {
            venue_event_id: "rest-funding:BTC:20".into(),
            amount: -0.12,
            currency: "USDC".into(),
            funding_time_ms: 20,
        };

        let event = ledger
            .record_funding_payment(&record, &input, OrderUpdateSource::FundingPoller, 21)
            .expect("funding poller ledger event");

        assert_eq!(event.source, OrderUpdateSource::FundingPoller);
        assert_ne!(event.source, OrderUpdateSource::PrivateWs);
        assert_eq!(
            event.event_id,
            "funding_payment:hedge-1-long:funding_poller:rest-funding:BTC:20"
        );
    }

    #[test]
    fn records_slippage_event_from_fill_reference() {
        let ledger = ExecutionLedger::default();
        let record = order_record_with_id("hedge-1-long", OrderSide::Buy);
        let input = SlippageLedgerInput {
            source_event_id: "fill_event:hedge-1-long:private_ws:fill-1".into(),
            amount_usd: 0.25,
            reference_price: 100.0,
            fill_price: 101.0,
            quantity: 0.25,
            occurred_at_ms: 20,
        };

        let event = ledger
            .record_slippage_event(&record, &input, OrderUpdateSource::PrivateWs, 21)
            .expect("slippage ledger event");

        assert_eq!(
            event.event_id,
            "slippage:fill_event:hedge-1-long:private_ws:fill-1"
        );
        assert_eq!(event.event_type, ExecutionLedgerEventType::Slippage);
        assert!(matches!(
            event.payload,
            ExecutionLedgerPayload::Slippage(SlippageLedgerRecord {
                amount_usd: 0.25,
                reference_price: 100.0,
                fill_price: 101.0,
                quantity: 0.25,
                quality: ExecutionLedgerQuality::Actual,
            })
        ));
    }

    #[test]
    fn rejects_slippage_event_without_positive_price_or_quantity() {
        let ledger = ExecutionLedger::default();
        let record = order_record_with_id("hedge-1-long", OrderSide::Buy);
        let input = SlippageLedgerInput {
            source_event_id: "fill-1".into(),
            amount_usd: 0.0,
            reference_price: 100.0,
            fill_price: 0.0,
            quantity: 0.25,
            occurred_at_ms: 20,
        };

        assert!(ledger
            .record_slippage_event(&record, &input, OrderUpdateSource::PrivateWs, 21)
            .is_none());
        assert!(ledger.list().is_empty());
    }

    #[test]
    fn records_orderbook_evidence_with_run_context() {
        let ledger = ExecutionLedger::default();
        let record = order_record_with_id("hedge-1-long", OrderSide::Buy);
        let context = ExecutionLedgerOrderContext::new(
            "run-1".to_owned(),
            "ticket-1".to_owned(),
            HedgeLegRole::Long,
        );
        let input = orderbook_input();

        let event = ledger
            .record_orderbook_evidence_with_context(
                &record,
                &input,
                OrderUpdateSource::Internal,
                22,
                Some(&context),
            )
            .expect("orderbook ledger event");
        let rows = ledger.query(&ExecutionLedgerQuery {
            run_id: Some("run-1".to_owned()),
            ticket_id: Some("ticket-1".to_owned()),
            leg_role: Some(HedgeLegRole::Long),
            limit: 10,
            ..ExecutionLedgerQuery::default()
        });

        assert_eq!(
            event.event_type,
            ExecutionLedgerEventType::OrderbookEvidence
        );
        assert!(event.event_id.starts_with("orderbook_evidence:"));
        assert_eq!(event.order.run_id.as_deref(), Some("run-1"));
        assert_eq!(event.occurred_at_ms, 20);
        assert_eq!(rows.len(), 1);
        let ExecutionLedgerPayload::OrderbookEvidence(record) = &event.payload else {
            panic!("orderbook payload");
        };
        assert_eq!(record.max_notional_usd, Some(1500.0));
        assert_eq!(record.depth_usd_20bps, Some(1500.0));
        assert_eq!(record.quality, ExecutionLedgerQuality::Actual);
    }

    #[test]
    fn rejects_empty_orderbook_evidence() {
        let ledger = ExecutionLedger::default();
        let record = order_record();
        let input = OrderbookDepthLedgerInput {
            reference_price: None,
            bid: None,
            ask: None,
            mid: None,
            open_vwap_price: None,
            open_slippage_bps: None,
            close_vwap_price: None,
            close_slippage_bps: None,
            depth_usd_5bps: None,
            depth_usd_10bps: None,
            depth_usd_20bps: None,
            max_notional_usd: None,
            market_timestamp_ms: None,
            health: None,
            reason: None,
            quality: ExecutionLedgerQuality::Missing,
        };

        assert!(ledger
            .record_orderbook_evidence(&record, &input, OrderUpdateSource::Internal, 22)
            .is_none());
    }

    #[test]
    fn records_order_state_event() {
        let ledger = ExecutionLedger::default();
        let record = order_record();

        let event = ledger
            .record_order_state(
                &record,
                &order_event_record(
                    OrderLifecycleEvent::AdapterAccepted,
                    OrderUpdateSource::AdapterAck,
                    LiveOrderState::Accepted,
                    Some("accepted".to_owned()),
                    11,
                ),
            )
            .expect("state event");

        assert_eq!(event.event_type, ExecutionLedgerEventType::OrderState);
        assert_eq!(event.source, OrderUpdateSource::AdapterAck);
        assert!(matches!(
            event.payload,
            ExecutionLedgerPayload::OrderState {
                state: LiveOrderState::Accepted,
                ..
            }
        ));
        assert_eq!(ledger.list().len(), 1);
    }

    #[test]
    fn realized_window_events_include_candidate_group_history_only() {
        let ledger = ExecutionLedger::default();
        let stale_long = order_record_with_id("stale-hedge-long", OrderSide::Buy);
        let stale_short = order_record_with_id("stale-hedge-short", OrderSide::Sell);
        let kept_long = order_record_with_id("kept-hedge-long", OrderSide::Buy);
        let kept_short = order_record_with_id("kept-hedge-short", OrderSide::Sell);

        ledger.record_fill_snapshot(&stale_long, &order_info(), OrderUpdateSource::PrivateWs, 1);
        ledger.record_fill_snapshot(&stale_short, &order_info(), OrderUpdateSource::PrivateWs, 2);
        ledger.record_fill_snapshot(&kept_long, &order_info(), OrderUpdateSource::PrivateWs, 3);
        ledger.record_fill_snapshot(&kept_short, &order_info(), OrderUpdateSource::PrivateWs, 10);
        ledger.record_slippage_event(
            &kept_long,
            &SlippageLedgerInput {
                source_event_id: "fill_snapshot:kept-hedge-long:private_ws".into(),
                amount_usd: 0.2,
                reference_price: 9.8,
                fill_price: 10.0,
                quantity: 1.0,
                occurred_at_ms: 3,
            },
            OrderUpdateSource::PrivateWs,
            4,
        );
        ledger.record_order_state(
            &kept_short,
            &order_event_record(
                OrderLifecycleEvent::AdapterAccepted,
                OrderUpdateSource::PrivateWs,
                LiveOrderState::Accepted,
                None,
                11,
            ),
        );

        let events = ledger.realized_window_events(5, 20);
        let order_ids = events
            .iter()
            .map(|event| event.order.identity.internal_order_id.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            order_ids,
            ["kept-hedge-long", "kept-hedge-long", "kept-hedge-short"]
        );
        assert_eq!(
            events
                .iter()
                .map(|event| event.event_type)
                .collect::<Vec<_>>(),
            [
                ExecutionLedgerEventType::FillSnapshot,
                ExecutionLedgerEventType::Slippage,
                ExecutionLedgerEventType::FillSnapshot,
            ]
        );
    }

    #[test]
    fn query_returns_bounded_order_events() {
        let ledger = ExecutionLedger::default();
        let first = order_record_with_id("hedge-1-long", OrderSide::Buy);
        let second = order_record_with_id("hedge-1-short", OrderSide::Sell);
        let other = order_record_with_id("hedge-2-long", OrderSide::Buy);

        ledger.record_fill_snapshot(&first, &order_info(), OrderUpdateSource::PrivateWs, 10);
        ledger.record_fill_snapshot(&second, &order_info(), OrderUpdateSource::PrivateWs, 20);
        ledger.record_fill_snapshot(&other, &order_info(), OrderUpdateSource::PrivateWs, 30);

        let events = ledger.query(&ExecutionLedgerQuery {
            hedge_group_id: Some("hedge-1".to_owned()),
            from_ms: Some(0),
            to_ms: Some(25),
            limit: 1,
            ..ExecutionLedgerQuery::default()
        });

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].order.identity.internal_order_id, "hedge-1-short");
    }

    #[test]
    fn query_matches_exchange_order_id() {
        let ledger = ExecutionLedger::default();
        let record = order_record_with_id("hedge-1-long", OrderSide::Buy);

        ledger.record_fill_snapshot(&record, &order_info(), OrderUpdateSource::PrivateWs, 10);

        let events = ledger.query(&ExecutionLedgerQuery {
            exchange_order_id: Some("x1".to_owned()),
            limit: 10,
            ..ExecutionLedgerQuery::default()
        });

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].order.identity.exchange_order_id.as_deref(),
            Some("x1")
        );
    }

    #[test]
    fn query_matches_run_ticket_and_leg_role() {
        let ledger = ExecutionLedger::default();
        let long = order_record_with_id("hedge-1-long", OrderSide::Buy);
        let short = order_record_with_id("hedge-1-short", OrderSide::Sell);
        let long_context = ExecutionLedgerOrderContext::new(
            "run-1".to_owned(),
            "ticket-1".to_owned(),
            HedgeLegRole::Long,
        );
        let short_context = ExecutionLedgerOrderContext::new(
            "run-1".to_owned(),
            "ticket-1".to_owned(),
            HedgeLegRole::Short,
        );

        ledger.record_fill_snapshot_with_context(
            &long,
            &order_info(),
            OrderUpdateSource::PrivateWs,
            10,
            Some(&long_context),
        );
        ledger.record_fill_snapshot_with_context(
            &short,
            &order_info(),
            OrderUpdateSource::PrivateWs,
            11,
            Some(&short_context),
        );

        let events = ledger.query(&ExecutionLedgerQuery {
            run_id: Some("run-1".to_owned()),
            ticket_id: Some("ticket-1".to_owned()),
            leg_role: Some(HedgeLegRole::Short),
            limit: 10,
            ..ExecutionLedgerQuery::default()
        });

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].order.run_id.as_deref(), Some("run-1"));
        assert_eq!(events[0].order.ticket_id.as_deref(), Some("ticket-1"));
        assert_eq!(events[0].order.leg_role, Some(HedgeLegRole::Short));
        assert_eq!(events[0].order.identity.internal_order_id, "hedge-1-short");
    }

    #[test]
    fn records_cancel_event_without_fill_payload() {
        let ledger = ExecutionLedger::default();
        let record = order_record();

        let event = ledger
            .record_order_state(
                &record,
                &order_event_record(
                    OrderLifecycleEvent::CancelAck,
                    OrderUpdateSource::PrivateWs,
                    LiveOrderState::Cancelled,
                    None,
                    12,
                ),
            )
            .expect("cancel event");

        assert_eq!(event.event_type, ExecutionLedgerEventType::Cancel);
        assert!(matches!(
            event.payload,
            ExecutionLedgerPayload::OrderState {
                state: LiveOrderState::Cancelled,
                ..
            }
        ));
    }

    fn order_event_record(
        event: OrderLifecycleEvent,
        source: OrderUpdateSource,
        state: LiveOrderState,
        message: Option<String>,
        occurred_at_ms: i64,
    ) -> OrderEventRecord {
        OrderEventRecord {
            internal_order_id: "ord-1".into(),
            client_order_id: "client-1".into(),
            exchange_order_id: Some("ex-1".into()),
            source,
            identity: VenueOrderIdentity {
                account_scope: None,
                internal_order_id: "ord-1".into(),
                public_client_order_id: "client-1".into(),
                product: shared_types::FeeProduct::Perp,
                venue_client_order_id: None,
                exchange_order_id: Some("ex-1".into()),
                client_order_id_policy: None,
                transport_metadata: Default::default(),
            },
            previous_state: None,
            state,
            event,
            message,
            payload: serde_json::Value::Null,
            occurred_at_ms,
        }
    }

    fn order_record() -> OrderRecord {
        order_record_with_id("ord-1", OrderSide::Buy)
    }

    fn order_record_with_id(id: &str, side: OrderSide) -> OrderRecord {
        let intent = shared_types::OrderIntent {
            id: id.into(),
            source: OrderSource::Manual,
            strategy: None,
            mode: ExecutionMode::DryRun,
            exchange: "mock".into(),
            symbol: "BTC".into(),
            side,
            order_type: OrderType::Limit,
            quantity: 1.0,
            price: Some(10.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: TimeInForce::Ioc,
            post_only: false,
            margin_mode: MarginMode::Cross,
            leverage: 1.0,
            client_order_id: client_order_id_for(id),
            client_order_id_policy: None,
            created_at_ms: 1,
        };
        OrderRecord {
            identity: VenueOrderIdentity::from_intent(&intent),
            intent,
            state: LiveOrderState::Filled,
            risk: None,
            last_update_source: OrderUpdateSource::PrivateWs,
            exchange_order_id: Some("x1".into()),
            message: None,
            filled_quantity: Some(0.5),
            filled_price: Some(10.0),
            filled_fee: Some(0.01),
            updated_at_ms: 10,
        }
    }

    fn client_order_id_for(id: &str) -> String {
        if id == "ord-1" {
            "client-1".into()
        } else {
            format!("{id}-client")
        }
    }

    fn order_info() -> OrderInfo {
        OrderInfo {
            execution_style: None,
            venue_time_in_force: None,
            client_order_id: None,
            reduce_only: None,
            order_id: "x1".into(),
            symbol: "BTC".into(),
            exchange: "mock".into(),
            side: OrderSide::Buy,
            order_type: OrderType::Limit,
            status: OrderStatus::Filled,
            quantity: 1.0,
            price: 10.0,
            filled_quantity: 0.5,
            filled_price: 10.0,
            fees: 0.01,
            created_at: chrono::Utc::now(),
        }
    }

    fn orderbook_input() -> OrderbookDepthLedgerInput {
        OrderbookDepthLedgerInput {
            reference_price: Some(100.0),
            bid: Some(99.9),
            ask: Some(100.1),
            mid: Some(100.0),
            open_vwap_price: Some(100.1),
            open_slippage_bps: Some(1.0),
            close_vwap_price: Some(99.9),
            close_slippage_bps: Some(1.0),
            depth_usd_5bps: Some(500.0),
            depth_usd_10bps: Some(1000.0),
            depth_usd_20bps: Some(1500.0),
            max_notional_usd: Some(1500.0),
            market_timestamp_ms: Some(20),
            health: Some(MarketDataHealth {
                quality: MarketDataQuality::Fresh,
                source: MarketDataSourceKind::WsPush,
                freshness_ms: Some(5),
                retry_after_ms: None,
                last_error: None,
                observed_at_ms: 20,
                coverage: None,
                problem: None,
            }),
            reason: None,
            quality: ExecutionLedgerQuality::Actual,
        }
    }
}
