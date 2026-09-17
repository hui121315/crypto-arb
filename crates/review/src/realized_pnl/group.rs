use super::amounts::{
    day_start_ms, durable_slippage_amount, fee_amount, filled_notional, funding_amount,
    hedge_group_id, positive_price, slippage_usd,
};
use super::events::{
    fee_snapshot, fill_snapshot, funding_payment, orderbook_evidence, slippage_record,
};
use super::*;

pub(super) fn eligible_order_index(orders: &[OrderRecord]) -> BTreeMap<String, OrderMeta> {
    orders
        .iter()
        .filter(|order| {
            order.intent.source == OrderSource::ArbitragePreview
                && !order.intent.reduce_only
                && order.state == shared_types::LiveOrderState::Filled
        })
        .map(|order| {
            (
                order.intent.id.clone(),
                OrderMeta {
                    group_id: hedge_group_id(&order.intent.id),
                    side: order.intent.side,
                    mode: order.intent.mode,
                    reference_price: positive_price(order.intent.price),
                },
            )
        })
        .collect()
}

pub(super) fn add_fills(
    groups: &mut BTreeMap<String, PnlGroup>,
    fills: Vec<&ExecutionLedgerEvent>,
    order_index: &BTreeMap<String, OrderMeta>,
) {
    for event in fills {
        let Some(snapshot) = fill_snapshot(event) else {
            continue;
        };
        let Some(notional) = filled_notional(snapshot) else {
            continue;
        };
        let order_id = &event.order.identity.internal_order_id;
        let Some(meta) = order_index.get(order_id) else {
            continue;
        };
        groups
            .entry(meta.group_id.clone())
            .or_default()
            .add_fill(event, meta, notional, snapshot);
    }
}

pub(super) fn add_fee_snapshots(
    groups: &mut BTreeMap<String, PnlGroup>,
    fees: Vec<&ExecutionLedgerEvent>,
    order_index: &BTreeMap<String, OrderMeta>,
) {
    for event in fees {
        let order_id = &event.order.identity.internal_order_id;
        let Some(meta) = order_index.get(order_id) else {
            continue;
        };
        let Some(snapshot) = fee_snapshot(event) else {
            continue;
        };
        groups
            .entry(meta.group_id.clone())
            .or_default()
            .add_fee(event, snapshot);
    }
}

pub(super) fn add_funding_payments(
    groups: &mut BTreeMap<String, PnlGroup>,
    payments: Vec<&ExecutionLedgerEvent>,
    order_index: &BTreeMap<String, OrderMeta>,
) {
    for event in payments {
        let order_id = &event.order.identity.internal_order_id;
        let Some(meta) = order_index.get(order_id) else {
            continue;
        };
        let Some(payment) = funding_payment(event) else {
            continue;
        };
        groups
            .entry(meta.group_id.clone())
            .or_default()
            .add_funding(event, payment);
    }
}

pub(super) fn add_slippage_records(
    groups: &mut BTreeMap<String, PnlGroup>,
    records: Vec<&ExecutionLedgerEvent>,
    order_index: &BTreeMap<String, OrderMeta>,
) {
    for event in records {
        let order_id = &event.order.identity.internal_order_id;
        let Some(meta) = order_index.get(order_id) else {
            continue;
        };
        let Some(record) = slippage_record(event) else {
            continue;
        };
        groups
            .entry(meta.group_id.clone())
            .or_default()
            .add_slippage(event, record);
    }
}

pub(super) fn add_orderbook_evidence(
    groups: &mut BTreeMap<String, PnlGroup>,
    records: Vec<&ExecutionLedgerEvent>,
    order_index: &BTreeMap<String, OrderMeta>,
) {
    for event in records {
        let order_id = &event.order.identity.internal_order_id;
        let Some(meta) = order_index.get(order_id) else {
            continue;
        };
        if orderbook_evidence(event).is_none() {
            continue;
        }
        groups
            .entry(meta.group_id.clone())
            .or_default()
            .add_orderbook(event);
    }
}

#[derive(Debug, Clone)]
pub(super) struct OrderMeta {
    group_id: String,
    side: OrderSide,
    pub(super) mode: shared_types::ExecutionMode,
    reference_price: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PnlGroup {
    realized_at_ms: i64,
    buy_notional_usd: f64,
    sell_notional_usd: f64,
    fee_usd: f64,
    funding_usd: f64,
    slippage_usd: f64,
    order_ids: BTreeSet<String>,
    durable_slippage_order_ids: BTreeSet<String>,
    evidence: ReviewPnlEvidence,
}

impl PnlGroup {
    fn add_fill(
        &mut self,
        event: &ExecutionLedgerEvent,
        meta: &OrderMeta,
        notional: f64,
        snapshot: &FillLedgerSnapshot,
    ) {
        self.realized_at_ms = self.realized_at_ms.max(event.occurred_at_ms);
        self.order_ids
            .insert(event.order.identity.internal_order_id.clone());
        self.evidence.fill_event_ids.push(event.event_id.clone());
        self.evidence.record_ledger_event(event);
        self.evidence.record_fill_confidence(snapshot.confidence);
        if let Some(fee) = snapshot.fee.as_ref().and_then(fee_amount) {
            self.fee_usd += fee;
            self.evidence.fee_event_ids.push(event.event_id.clone());
        }
        if !self
            .durable_slippage_order_ids
            .contains(&event.order.identity.internal_order_id)
        {
            self.add_fill_derived_slippage(event, meta, snapshot);
        }
        match meta.side {
            OrderSide::Buy => self.buy_notional_usd += notional,
            OrderSide::Sell => self.sell_notional_usd += notional,
        }
    }

    fn add_fill_derived_slippage(
        &mut self,
        event: &ExecutionLedgerEvent,
        meta: &OrderMeta,
        snapshot: &FillLedgerSnapshot,
    ) {
        if let Some(slippage) = meta
            .reference_price
            .and_then(|reference| slippage_usd(meta.side, reference, snapshot))
        {
            self.slippage_usd += slippage;
            self.evidence
                .estimated_slippage_fill_event_ids
                .push(event.event_id.clone());
        }
    }

    fn add_fee(&mut self, event: &ExecutionLedgerEvent, snapshot: &FeeLedgerSnapshot) {
        let Some(fee) = fee_amount(snapshot) else {
            return;
        };
        self.fee_usd += fee;
        self.evidence.fee_event_ids.push(event.event_id.clone());
        self.evidence.record_ledger_event(event);
    }

    fn add_funding(&mut self, event: &ExecutionLedgerEvent, payment: &FundingPaymentLedgerRecord) {
        let Some(amount) = funding_amount(payment) else {
            return;
        };
        self.funding_usd += amount;
        self.evidence.funding_event_ids.push(event.event_id.clone());
        self.evidence.record_ledger_event(event);
    }

    fn add_slippage(&mut self, event: &ExecutionLedgerEvent, record: &SlippageLedgerRecord) {
        let Some(amount) = durable_slippage_amount(record) else {
            return;
        };
        self.slippage_usd += amount;
        self.evidence
            .slippage_event_ids
            .push(event.event_id.clone());
        self.evidence.record_ledger_event(event);
        self.durable_slippage_order_ids
            .insert(event.order.identity.internal_order_id.clone());
    }

    fn add_orderbook(&mut self, event: &ExecutionLedgerEvent) {
        if !self.evidence.orderbook_event_ids.contains(&event.event_id) {
            self.evidence
                .orderbook_event_ids
                .push(event.event_id.clone());
            self.evidence.record_ledger_event(event);
        }
    }

    pub(super) fn into_row(
        self,
        group_id: String,
        from_ms: i64,
        to_ms: i64,
    ) -> Option<RealizedPnlRow> {
        if !self.is_complete() || self.realized_at_ms < from_ms || self.realized_at_ms >= to_ms {
            return None;
        }
        let price_pnl_usd = self.sell_notional_usd - self.buy_notional_usd;
        Some(RealizedPnlRow {
            group_id,
            realized_at_ms: self.realized_at_ms,
            realized_day_ms: day_start_ms(self.realized_at_ms),
            buy_notional_usd: self.buy_notional_usd,
            sell_notional_usd: self.sell_notional_usd,
            price_pnl_usd,
            fee_usd: self.fee_usd,
            funding_usd: self.funding_usd,
            slippage_usd: self.slippage_usd,
            net_pnl_usd: price_pnl_usd + self.funding_usd - self.fee_usd,
            closed_at_ms: None,
            close_price_quality: None,
            order_ids: self.order_ids,
            evidence: self.evidence,
        })
    }

    fn is_complete(&self) -> bool {
        self.buy_notional_usd > 0.0 && self.sell_notional_usd > 0.0
    }
}
