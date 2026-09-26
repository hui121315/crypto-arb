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
    (fill.quality == ExecutionLedgerQuality::Actual
        && fill.quantity.is_finite()
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
    ledger: &mut Option<shared_types::CloseFillLedger>,
    target_quantity: f64,
    event: &ExecutionLedgerEvent,
    update: &CloseLedgerUpdate<'_>,
) -> bool {
    let trusted_source = |source| close_fill_source(order.intent.mode, source);
    if update.fill.is_some() && !trusted_source(event.source) {
        return false;
    }
    if !trusted_source(order.last_update_source) {
        order.filled_quantity = None;
        order.filled_price = None;
        order.filled_fee = None;
    }
    let use_source = trusted_source(event.source) || !trusted_source(order.last_update_source);
    if let Some(fill) = update.fill {
        if !apply_ledger_fill(
            order,
            ledger,
            target_quantity,
            event,
            fill,
            update.incremental_fill,
        ) {
            return false;
        }
    } else if !terminal_close_order(order.state) || terminal_close_order(update.state) {
        // A late accepted/submitted notification must not reopen an ended order.
        if ledger_update_time(event) >= order.updated_at_ms {
            order.state = update.state;
        }
    }
    if use_source && (update.fill.is_some() || ledger_update_time(event) >= order.updated_at_ms) {
        order.last_update_source = event.source;
    }
    order.updated_at_ms = order.updated_at_ms.max(ledger_update_time(event));
    if let Some(message) = update.message {
        order.message = Some(message.to_owned());
    }
    true
}

pub(super) fn apply_ledger_fill(
    order: &mut OrderRecord,
    ledger: &mut Option<shared_types::CloseFillLedger>,
    target_quantity: f64,
    event: &ExecutionLedgerEvent,
    fill: &FillLedgerSnapshot,
    incremental: bool,
) -> bool {
    let (quantity, price, fee) = if incremental {
        if event.event_id.trim().is_empty() {
            return false;
        }
        let ledger = ledger.get_or_insert_with(Default::default);
        if ledger.event_ids.contains(&event.event_id) {
            return false;
        }
        let quantity = ledger.totals.quantity + fill.quantity;
        let notional = ledger.totals.notional + fill.quantity * fill.average_price;
        if !quantity.is_finite() || !notional.is_finite() {
            return false;
        }
        let fee = fill
            .fee
            .as_ref()
            .filter(|fee| fee.quality == ExecutionLedgerQuality::Actual && fee.amount.is_finite())
            .map(|fee| ledger.totals.fee.unwrap_or(0.0) + fee.amount)
            .or(ledger.totals.fee);
        if fee.is_some_and(|fee| !fee.is_finite()) {
            return false;
        }
        ledger.last_fill_at_ms = if event.occurred_at_ms <= 0 {
            None
        } else if ledger.event_ids.is_empty() {
            Some(event.occurred_at_ms)
        } else {
            ledger.last_fill_at_ms.map(|time| time.max(event.occurred_at_ms))
        };
        ledger.event_ids.push(event.event_id.clone());
        ledger.totals.quantity = quantity;
        ledger.totals.notional = notional;
        ledger.totals.fee = fee;
        (quantity, notional / quantity, ledger.totals.fee)
    } else {
        (
            fill.quantity,
            fill.average_price,
            fill.fee
                .as_ref()
                .filter(|fee| {
                    fee.quality == ExecutionLedgerQuality::Actual && fee.amount.is_finite()
                })
                .map(|fee| fee.amount),
        )
    };
    // A cumulative reply and its individual fills describe the same traded quantity.
    let previous_quantity = order
        .filled_quantity
        .filter(|q| q.is_finite() && *q >= 0.0)
        .unwrap_or(0.0);
    if quantity > previous_quantity
        || (quantity == previous_quantity && ledger_update_time(event) >= order.updated_at_ms)
    {
        order.filled_quantity = Some(quantity);
        order.filled_price = Some(price);
        order.filled_fee = fee.or(order.filled_fee);
        if !terminal_close_order(order.state) {
            order.state = close_fill_state(quantity, target_quantity);
        }
    }
    true
}

fn terminal_close_order(state: LiveOrderState) -> bool {
    matches!(
        state,
        LiveOrderState::Filled
            | LiveOrderState::Cancelled
            | LiveOrderState::Rejected
            | LiveOrderState::Failed
    )
}

fn close_fill_source(mode: ExecutionMode, source: OrderUpdateSource) -> bool {
    matches!(
        source,
        OrderUpdateSource::PrivateWs | OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile
    ) || (mode == ExecutionMode::DryRun && source == OrderUpdateSource::AdapterAck)
}

pub(super) fn merge_close_order(
    previous: Option<&OrderRecord>,
    incoming: &OrderRecord,
) -> OrderRecord {
    let mut next = incoming.clone();
    let Some(previous) = previous else {
        return next;
    };
    if !close_fill_source(previous.intent.mode, previous.last_update_source) {
        return next;
    }
    let authoritative_fill = close_fill_source(incoming.intent.mode, incoming.last_update_source);
    if !authoritative_fill
        || previous.filled_quantity.is_some_and(|q| {
            q.is_finite()
                && q >= 0.0
                && !incoming.filled_quantity.is_some_and(|n| {
                    n.is_finite()
                        && (n > q || (n == q && incoming.updated_at_ms >= previous.updated_at_ms))
                })
        })
    {
        next.filled_quantity = previous.filled_quantity;
        next.filled_price = previous.filled_price;
        next.filled_fee = previous.filled_fee;
        next.last_update_source = previous.last_update_source;
    }
    if (!authoritative_fill
        && (incoming.state == LiveOrderState::Filled || terminal_close_order(previous.state)))
        || incoming.updated_at_ms < previous.updated_at_ms
        || (terminal_close_order(previous.state) && !terminal_close_order(incoming.state))
    {
        next.state = previous.state;
        next.updated_at_ms = previous.updated_at_ms;
        next.last_update_source = previous.last_update_source;
    }
    next
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
        LiveOrderState::Filled
            if has_fill_evidence(record)
                && record.filled_quantity.is_some_and(|quantity| {
                    let target = record.intent.quantity;
                    target.is_finite()
                        && target > 0.0
                        && (quantity - target).abs() <= f64::EPSILON * 32.0 * target.abs()
                }) =>
        {
            CloseLegStatus::Filled
        }
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

pub(super) fn updated_finality_source(
    previous_state: Option<LiveOrderState>,
    previous_source: Option<OrderUpdateSource>,
    record: &OrderRecord,
    observed_state: Option<LiveOrderState>,
    observed_source: OrderUpdateSource,
) -> Option<OrderUpdateSource> {
    if terminal_close_order(record.state) && previous_state == Some(record.state) {
        if previous_source.is_some_and(|source| matches!(source,
            OrderUpdateSource::PrivateWs | OrderUpdateSource::OrderQuery | OrderUpdateSource::Reconcile))
            || observed_state != Some(record.state) {
            // Fill data proves quantity, not a locally inferred cancellation.
            return previous_source;
        }
    }
    if observed_state == Some(record.state) {
        close_finality_source(record).map(|_| observed_source)
    } else {
        close_finality_source(record)
    }
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
            finality_problem(record, "平仓成交数量或价格尚未核清，请核对剩余持仓"),
        ),
        CloseLegStatus::Cancelled | CloseLegStatus::Rejected | CloseLegStatus::Failed => Some(
            finality_problem(record, "close order reached a non-filled terminal state"),
        ),
        _ => None,
    }
}
