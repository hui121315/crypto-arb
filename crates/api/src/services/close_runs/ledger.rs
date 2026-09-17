use super::*;

#[derive(Clone, Copy)]
pub(super) struct CloseLedgerUpdate<'a> {
    pub(super) state: LiveOrderState,
    pub(super) message: Option<&'a str>,
    pub(super) fill: Option<&'a FillLedgerSnapshot>,
    pub(super) incremental_fill: bool,
}

pub(super) fn replaces_paper_adapter_fill(
    order: &OrderRecord,
    event: &ExecutionLedgerEvent,
    fill: Option<&FillLedgerSnapshot>,
) -> bool {
    fill.is_some()
        && order.intent.mode == ExecutionMode::DryRun
        && order.last_update_source == OrderUpdateSource::AdapterAck
        && event.source != OrderUpdateSource::AdapterAck
}

pub(super) fn remove_paper_adapter_cost_events(events: &mut Vec<CloseRunCostLedgerEvent>) {
    events.retain(|event| event.source != OrderUpdateSource::AdapterAck);
}

impl<'a> CloseLedgerUpdate<'a> {
    pub(super) fn from_event(event: &'a ExecutionLedgerEvent) -> Option<Self> {
        match &event.payload {
            ExecutionLedgerPayload::OrderState { state, message } => Some(Self {
                state: *state,
                message: message.as_deref(),
                fill: None,
                incremental_fill: false,
            }),
            ExecutionLedgerPayload::FillSnapshot(fill) => {
                valid_ledger_fill(fill).map(|fill| Self {
                    state: LiveOrderState::PartiallyFilled,
                    message: None,
                    fill: Some(fill),
                    incremental_fill: event.event_type == ExecutionLedgerEventType::FillEvent,
                })
            }
            ExecutionLedgerPayload::FeeSnapshot(_) | ExecutionLedgerPayload::FundingPayment(_) => {
                None
            }
            ExecutionLedgerPayload::Slippage(_) => None,
            ExecutionLedgerPayload::OrderbookEvidence(_) => None,
        }
    }
}

pub(super) fn valid_ledger_fill(fill: &FillLedgerSnapshot) -> Option<&FillLedgerSnapshot> {
    (fill.quantity.is_finite()
        && fill.quantity > 0.0
        && fill.average_price.is_finite()
        && fill.average_price > 0.0)
        .then_some(fill)
}

pub(super) fn apply_ledger_identity(order: &mut OrderRecord, event: &ExecutionLedgerEvent) {
    let mut identity = order.identity_snapshot();
    if let Some(venue_client_order_id) = event.order.identity.venue_client_order_id.as_deref() {
        identity.record_venue_client_order_id(venue_client_order_id);
    }
    if let Some(exchange_order_id) = event.order.identity.exchange_order_id.as_deref() {
        identity.record_exchange_order_id(exchange_order_id);
        order.exchange_order_id = Some(exchange_order_id.to_owned());
    }
    order.identity = identity;
}

pub(super) fn apply_ledger_order_update(
    order: &mut OrderRecord,
    target_quantity: f64,
    event: &ExecutionLedgerEvent,
    update: &CloseLedgerUpdate<'_>,
) {
    order.last_update_source = event.source;
    order.updated_at_ms = ledger_update_time(event);
    if let Some(message) = update.message {
        order.message = Some(message.to_owned());
    }
    if let Some(fill) = update.fill {
        apply_ledger_fill(order, target_quantity, fill, update.incremental_fill);
    } else {
        order.state = update.state;
    }
}

pub(super) fn apply_ledger_fill(
    order: &mut OrderRecord,
    target_quantity: f64,
    fill: &FillLedgerSnapshot,
    incremental: bool,
) {
    let quantity = if incremental {
        order.filled_quantity.unwrap_or(0.0) + fill.quantity
    } else {
        fill.quantity
    };
    order.filled_quantity = Some(quantity);
    order.filled_price = Some(fill.average_price);
    order.filled_fee = ledger_fill_fee_after_event(order, fill, incremental);
    order.state = close_fill_state(quantity, target_quantity);
}

pub(super) fn ledger_fill_fee_after_event(
    order: &OrderRecord,
    fill: &FillLedgerSnapshot,
    incremental: bool,
) -> Option<f64> {
    let amount = fill.fee.as_ref()?.amount;
    if !amount.is_finite() {
        return order.filled_fee;
    }
    if incremental {
        Some(order.filled_fee.unwrap_or(0.0) + amount)
    } else {
        Some(amount)
    }
}

pub(super) fn close_fill_state(quantity: f64, target_quantity: f64) -> LiveOrderState {
    if target_quantity > 0.0 && quantity + f64::EPSILON < target_quantity {
        LiveOrderState::PartiallyFilled
    } else {
        LiveOrderState::Filled
    }
}

pub(super) fn ledger_update_time(event: &ExecutionLedgerEvent) -> i64 {
    if event.occurred_at_ms > 0 {
        event.occurred_at_ms
    } else {
        event.captured_at_ms
    }
}

pub(super) fn record_fill_fee_cost_event(
    events: &mut Vec<CloseRunCostLedgerEvent>,
    event: &ExecutionLedgerEvent,
    fill: Option<&FillLedgerSnapshot>,
) -> bool {
    let Some(fee) = fill.and_then(|fill| fill.fee.as_ref()) else {
        return false;
    };
    if fee.quality != ExecutionLedgerQuality::Actual || !fee.amount.is_finite() {
        return false;
    }
    record_cost_event(
        events,
        ledger_cost_event(event, CloseRunCostComponent::Fee, fee.amount),
    )
}

pub(super) fn record_paper_fill_slippage_cost_event(
    events: &mut Vec<CloseRunCostLedgerEvent>,
    order: &OrderRecord,
    event: &ExecutionLedgerEvent,
    fill: Option<&FillLedgerSnapshot>,
) -> bool {
    let Some(fill) = fill else {
        return false;
    };
    if order.intent.mode != ExecutionMode::DryRun
        || event.source != OrderUpdateSource::AdapterAck
        || fill.quality != ExecutionLedgerQuality::Actual
    {
        return false;
    }
    let Some(reference_price) = order
        .intent
        .price
        .filter(|price| price.is_finite() && *price > 0.0)
    else {
        return false;
    };
    let price_delta = match order.intent.side {
        OrderSide::Buy => fill.average_price - reference_price,
        OrderSide::Sell => reference_price - fill.average_price,
    };
    let amount_usd = price_delta * fill.quantity;
    if !amount_usd.is_finite() {
        return false;
    }
    record_cost_event(
        events,
        CloseRunCostLedgerEvent {
            event_id: format!("slippage:{}", event.event_id),
            ..ledger_cost_event(event, CloseRunCostComponent::Slippage, amount_usd)
        },
    )
}

pub(super) fn ledger_slippage_cost_event(
    event: &ExecutionLedgerEvent,
) -> Option<CloseRunCostLedgerEvent> {
    let ExecutionLedgerPayload::Slippage(slippage) = &event.payload else {
        return None;
    };
    valid_slippage_amount(slippage)
        .map(|amount| ledger_cost_event(event, CloseRunCostComponent::Slippage, amount))
}

pub(super) fn ledger_funding_cost_event(
    event: &ExecutionLedgerEvent,
) -> Option<CloseRunCostLedgerEvent> {
    let ExecutionLedgerPayload::FundingPayment(payment) = &event.payload else {
        return None;
    };
    funding_payment_usd_amount(payment)
        .map(|amount| ledger_cost_event(event, CloseRunCostComponent::Funding, amount))
}

fn valid_slippage_amount(slippage: &SlippageLedgerRecord) -> Option<f64> {
    (slippage.quality == ExecutionLedgerQuality::Actual && slippage.amount_usd.is_finite())
        .then_some(slippage.amount_usd)
}

pub(super) fn funding_payment_usd_amount(payment: &FundingPaymentLedgerRecord) -> Option<f64> {
    (payment.quality == ExecutionLedgerQuality::Actual
        && payment.amount.is_finite()
        && is_usd_settlement_currency(&payment.currency))
    .then_some(payment.amount)
}

fn is_usd_settlement_currency(currency: &str) -> bool {
    let currency = currency.trim();
    currency.eq_ignore_ascii_case("USD")
        || currency.eq_ignore_ascii_case("USDC")
        || currency.eq_ignore_ascii_case("USDT")
}

pub(super) fn ledger_cost_event(
    event: &ExecutionLedgerEvent,
    component: CloseRunCostComponent,
    amount_usd: f64,
) -> CloseRunCostLedgerEvent {
    CloseRunCostLedgerEvent {
        event_id: event.event_id.clone(),
        component,
        amount_usd,
        source: event.source,
        quality: ExecutionLedgerQuality::Actual,
        occurred_at_ms: event.occurred_at_ms,
        captured_at_ms: event.captured_at_ms,
    }
}

pub(super) fn record_cost_event(
    events: &mut Vec<CloseRunCostLedgerEvent>,
    event: CloseRunCostLedgerEvent,
) -> bool {
    if events.iter().any(|existing| {
        existing.component == event.component && existing.event_id == event.event_id
    }) {
        return false;
    }
    events.push(event);
    true
}

pub(super) fn close_leg_status(record: &OrderRecord) -> CloseLegStatus {
    match record.state {
        LiveOrderState::Created | LiveOrderState::RiskChecked | LiveOrderState::Submitted => {
            CloseLegStatus::Submitted
        }
        LiveOrderState::Accepted | LiveOrderState::Unknown => CloseLegStatus::Accepted,
        LiveOrderState::PartiallyFilled => CloseLegStatus::PartiallyFilled,
        LiveOrderState::Filled if has_fill_evidence(record) => CloseLegStatus::Filled,
        LiveOrderState::Filled => CloseLegStatus::Accepted,
        LiveOrderState::CancelRequested => CloseLegStatus::CancelRequested,
        LiveOrderState::Cancelled => CloseLegStatus::Cancelled,
        LiveOrderState::Rejected => CloseLegStatus::Rejected,
        LiveOrderState::Failed => CloseLegStatus::Failed,
    }
}

pub(super) fn has_fill_evidence(record: &OrderRecord) -> bool {
    record
        .filled_quantity
        .is_some_and(|quantity| quantity.is_finite() && quantity > 0.0)
        && record
            .filled_price
            .is_some_and(|price| price.is_finite() && price > 0.0)
}

pub(super) fn close_finality_source(record: &OrderRecord) -> Option<OrderUpdateSource> {
    matches!(
        record.state,
        LiveOrderState::PartiallyFilled
            | LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
    .then_some(record.last_update_source)
}

pub(super) fn confirmed_filled_at_ms(leg: &CloseLeg, _record: &OrderRecord) -> Option<i64> {
    leg.confirmed_filled_at_ms
}

pub(super) fn confirmed_filled_at_ms_from(
    current: Option<i64>,
    status: CloseLegStatus,
    updated_at_ms: i64,
) -> Option<i64> {
    if status != CloseLegStatus::Filled || updated_at_ms <= 0 {
        return current;
    }
    Some(current.map_or(updated_at_ms, |current| current.max(updated_at_ms)))
}

pub(super) fn confirmed_filled_at_ms_from_ledger(
    current: Option<i64>,
    status: CloseLegStatus,
    mode: ExecutionMode,
    event: &ExecutionLedgerEvent,
) -> Option<i64> {
    confirmed_filled_at_ms_from(
        current,
        status,
        confirmed_fill_time_from_ledger(mode, event),
    )
}

pub(super) fn confirmed_fill_time_from_ledger(
    mode: ExecutionMode,
    event: &ExecutionLedgerEvent,
) -> i64 {
    let paper_fill_snapshot = mode == ExecutionMode::DryRun
        && event.event_type == ExecutionLedgerEventType::FillSnapshot
        && event.source == OrderUpdateSource::AdapterAck
        && matches!(
            &event.payload,
            ExecutionLedgerPayload::FillSnapshot(fill)
                if fill.quality == ExecutionLedgerQuality::Actual
                    && fill.confidence == shared_types::ExecutionFillConfidence::AdapterAck
        );
    if event.event_type == ExecutionLedgerEventType::FillEvent || paper_fill_snapshot {
        event.occurred_at_ms
    } else {
        0
    }
}

pub(super) fn close_leg_problem(
    record: &OrderRecord,
    status: CloseLegStatus,
) -> Option<ApiProblem> {
    match status {
        CloseLegStatus::Accepted if record.state == LiveOrderState::Filled => Some(
            finality_problem(record, "filled order is missing fill evidence"),
        ),
        CloseLegStatus::Cancelled | CloseLegStatus::Rejected | CloseLegStatus::Failed => Some(
            finality_problem(record, "close order reached a non-filled terminal state"),
        ),
        _ => None,
    }
}
