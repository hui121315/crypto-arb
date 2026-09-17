use super::group::OrderMeta;
use super::*;

pub(super) fn fill_events_for_pnl<'a>(
    ledger: &'a [ExecutionLedgerEvent],
    order_index: &BTreeMap<String, OrderMeta>,
    to_ms: i64,
) -> Vec<&'a ExecutionLedgerEvent> {
    let incremental_order_ids = incremental_fill_order_ids(ledger, order_index, to_ms);
    let mut fills = ledger
        .iter()
        .filter(|event| {
            event.occurred_at_ms < to_ms
                && is_incremental_fill_event(event)
                && order_index.contains_key(&event.order.identity.internal_order_id)
                && realized_fill_snapshot(event, order_index).is_some()
        })
        .collect::<Vec<_>>();
    fills.extend(latest_cumulative_fill_snapshots(
        ledger,
        order_index,
        to_ms,
        &incremental_order_ids,
    ));
    fills.sort_by(|left, right| {
        left.occurred_at_ms
            .cmp(&right.occurred_at_ms)
            .then_with(|| left.captured_at_ms.cmp(&right.captured_at_ms))
            .then_with(|| left.event_id.cmp(&right.event_id))
    });
    fills
}

fn incremental_fill_order_ids(
    ledger: &[ExecutionLedgerEvent],
    order_index: &BTreeMap<String, OrderMeta>,
    to_ms: i64,
) -> BTreeSet<String> {
    ledger
        .iter()
        .filter(|event| {
            event.occurred_at_ms < to_ms
                && is_incremental_fill_event(event)
                && realized_fill_snapshot(event, order_index).is_some()
        })
        .filter_map(|event| {
            let order_id = &event.order.identity.internal_order_id;
            order_index.contains_key(order_id).then(|| order_id.clone())
        })
        .collect()
}

fn latest_cumulative_fill_snapshots<'a>(
    ledger: &'a [ExecutionLedgerEvent],
    order_index: &BTreeMap<String, OrderMeta>,
    to_ms: i64,
    excluded_order_ids: &BTreeSet<String>,
) -> Vec<&'a ExecutionLedgerEvent> {
    latest_snapshots(ledger, order_index, to_ms, |event| {
        if is_incremental_fill_event(event)
            || excluded_order_ids.contains(&event.order.identity.internal_order_id)
        {
            None
        } else {
            realized_fill_snapshot(event, order_index)
        }
    })
}

pub(super) fn latest_fee_snapshots<'a>(
    ledger: &'a [ExecutionLedgerEvent],
    order_index: &BTreeMap<String, OrderMeta>,
    to_ms: i64,
) -> Vec<&'a ExecutionLedgerEvent> {
    latest_snapshots(ledger, order_index, to_ms, fee_snapshot)
}

fn latest_snapshots<'a, T: 'a, F>(
    ledger: &'a [ExecutionLedgerEvent],
    order_index: &BTreeMap<String, OrderMeta>,
    to_ms: i64,
    payload: F,
) -> Vec<&'a ExecutionLedgerEvent>
where
    F: Fn(&'a ExecutionLedgerEvent) -> Option<&'a T>,
{
    let mut by_order = BTreeMap::<String, &'a ExecutionLedgerEvent>::new();
    for event in ledger {
        let order_id = &event.order.identity.internal_order_id;
        if event.occurred_at_ms >= to_ms
            || !order_index.contains_key(order_id)
            || payload(event).is_none()
        {
            continue;
        }
        let replace = match by_order.get(order_id) {
            Some(current) => is_newer_snapshot(current, event),
            None => true,
        };
        if replace {
            by_order.insert(order_id.clone(), event);
        }
    }
    by_order.into_values().collect()
}

pub(super) fn funding_payments<'a>(
    ledger: &'a [ExecutionLedgerEvent],
    order_index: &BTreeMap<String, OrderMeta>,
    to_ms: i64,
) -> Vec<&'a ExecutionLedgerEvent> {
    ledger
        .iter()
        .filter(|event| event.occurred_at_ms < to_ms)
        .filter(|event| {
            order_index.contains_key(&event.order.identity.internal_order_id)
                && funding_payment(event).is_some()
        })
        .collect()
}

pub(super) fn slippage_records<'a>(
    ledger: &'a [ExecutionLedgerEvent],
    order_index: &BTreeMap<String, OrderMeta>,
    to_ms: i64,
    selected_fill_event_ids: &BTreeSet<String>,
) -> Vec<&'a ExecutionLedgerEvent> {
    let known_fill_event_ids = ledger
        .iter()
        .filter(|event| fill_snapshot(event).is_some())
        .map(|event| event.event_id.as_str())
        .collect::<BTreeSet<_>>();
    ledger
        .iter()
        .filter(|event| event.occurred_at_ms < to_ms)
        .filter(|event| {
            order_index.contains_key(&event.order.identity.internal_order_id)
                && slippage_record(event).is_some()
                && slippage_matches_selected_fill(
                    event,
                    &known_fill_event_ids,
                    selected_fill_event_ids,
                )
        })
        .collect()
}

fn slippage_matches_selected_fill(
    event: &ExecutionLedgerEvent,
    known_fill_event_ids: &BTreeSet<&str>,
    selected_fill_event_ids: &BTreeSet<String>,
) -> bool {
    let Some(source_event_id) = event.event_id.strip_prefix("slippage:") else {
        return true;
    };
    !known_fill_event_ids.contains(source_event_id)
        || selected_fill_event_ids.contains(source_event_id)
}

pub(super) fn orderbook_evidence_events<'a>(
    ledger: &'a [ExecutionLedgerEvent],
    order_index: &BTreeMap<String, OrderMeta>,
    to_ms: i64,
) -> Vec<&'a ExecutionLedgerEvent> {
    ledger
        .iter()
        .filter(|event| event.occurred_at_ms < to_ms)
        .filter(|event| {
            order_index.contains_key(&event.order.identity.internal_order_id)
                && orderbook_evidence(event).is_some()
        })
        .collect()
}

pub(super) fn fill_snapshot(event: &ExecutionLedgerEvent) -> Option<&FillLedgerSnapshot> {
    match &event.payload {
        ExecutionLedgerPayload::FillSnapshot(snapshot) => Some(snapshot),
        _ => None,
    }
}

fn realized_fill_snapshot<'a>(
    event: &'a ExecutionLedgerEvent,
    order_index: &BTreeMap<String, OrderMeta>,
) -> Option<&'a FillLedgerSnapshot> {
    let meta = order_index.get(&event.order.identity.internal_order_id)?;
    fill_snapshot(event).filter(|snapshot| {
        let paper_fill = meta.mode == shared_types::ExecutionMode::DryRun
            && snapshot.confidence == shared_types::ExecutionFillConfidence::AdapterAck;
        snapshot.quality != shared_types::ExecutionLedgerQuality::Missing
            && (snapshot.confidence.supports_terminal_fill() || paper_fill)
    })
}

pub(super) fn fee_snapshot(event: &ExecutionLedgerEvent) -> Option<&FeeLedgerSnapshot> {
    match &event.payload {
        ExecutionLedgerPayload::FeeSnapshot(snapshot) => Some(snapshot),
        _ => None,
    }
}

pub(super) fn funding_payment(event: &ExecutionLedgerEvent) -> Option<&FundingPaymentLedgerRecord> {
    match &event.payload {
        ExecutionLedgerPayload::FundingPayment(payment) => Some(payment),
        _ => None,
    }
}

pub(super) fn slippage_record(event: &ExecutionLedgerEvent) -> Option<&SlippageLedgerRecord> {
    match &event.payload {
        ExecutionLedgerPayload::Slippage(record) => Some(record),
        _ => None,
    }
}

pub(super) fn orderbook_evidence(
    event: &ExecutionLedgerEvent,
) -> Option<&OrderbookDepthLedgerRecord> {
    match &event.payload {
        ExecutionLedgerPayload::OrderbookEvidence(record) => Some(record.as_ref()),
        _ => None,
    }
}

fn is_incremental_fill_event(event: &ExecutionLedgerEvent) -> bool {
    event.event_type == ExecutionLedgerEventType::FillEvent
}

fn is_newer_snapshot(current: &ExecutionLedgerEvent, candidate: &ExecutionLedgerEvent) -> bool {
    (
        candidate.occurred_at_ms,
        candidate.captured_at_ms,
        candidate.event_id.as_str(),
    ) > (
        current.occurred_at_ms,
        current.captured_at_ms,
        current.event_id.as_str(),
    )
}
