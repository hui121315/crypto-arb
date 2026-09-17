use super::*;

impl TradingService {
    pub(super) async fn apply_binance_order_trade(
        &self,
        delta: BinanceOrderTradeDelta,
    ) -> PrivateWsApplyOutcome {
        let open_order_cache_updated = self.apply_open_order_cache(&delta.order.order);
        let prior_event_ids = self.binance_order_ledger_event_ids(&delta.order);
        if delta.terminal {
            self.apply_binance_terminal_state(&delta);
        }
        let updated = self.apply_private_order_delta(&delta.order);
        let mut ledger_events = self
            .binance_order_ledger_events(&delta.order)
            .into_iter()
            .filter(|event| !prior_event_ids.contains(&event.event_id))
            .collect::<Vec<_>>();
        if let Some(fill) = delta.fill.as_ref() {
            ledger_events.extend(self.record_private_fill_delta(fill, None));
        }
        if let Some(finality) = binance_reduce_only_fill_finality(&delta, &ledger_events) {
            ledger_events.push(finality);
        }
        let ledger_updated = !ledger_events.is_empty();
        let duplicate_finality = (delta.terminal || delta.fill.is_some()) && !ledger_updated;
        let account_changed = delta.fill.is_some()
            || updated.as_ref().is_some_and(|record| {
                crate::trading_service::filled_quantity_changes_account_state(
                    record.filled_quantity,
                )
            })
            || crate::trading_service::order_info_changes_account_state(&delta.order.order);
        let account_cache_dirty = account_changed.then(|| {
            self.mark_private_event_account_dirty(PrivateAccountDirty::new(
                "binance",
                PrivateAccountScope::All,
                "fill_event",
            ))
        });
        if let Some(record) = updated.as_ref() {
            self.live_order_proof_health
                .record_private_ws_cancel_finality_from_record(record, "private_ws_order");
        }
        PrivateWsApplyOutcome {
            order: (!duplicate_finality).then_some(updated).flatten(),
            ledger_events,
            ledger_updated,
            order_projection_handled_by_ledger: ledger_updated,
            open_order_cache_updated,
            account_cache_dirty,
            ..PrivateWsApplyOutcome::default()
        }
    }

    fn apply_binance_terminal_state(&self, delta: &BinanceOrderTradeDelta) {
        let _ = self.journal.update_state_by_exchange_order_id_from_source(
            &delta.order.order.order_id,
            live_state_from_order_status(delta.order.order.status),
            Some(binance_finality_message(delta)),
            delta.order.received_at_ms,
            OrderUpdateSource::PrivateWs,
        );
    }

    fn binance_order_ledger_event_ids(&self, delta: &PrivateOrderDelta) -> HashSet<String> {
        self.binance_order_ledger_events(delta)
            .into_iter()
            .map(|event| event.event_id)
            .collect()
    }

    fn binance_order_ledger_events(&self, delta: &PrivateOrderDelta) -> Vec<ExecutionLedgerEvent> {
        let record = self
            .journal
            .get_by_client_order_id(&delta.client_order_id)
            .or_else(|| self.journal.get_by_exchange_order_id(&delta.order.order_id));
        let Some(record) = record else {
            return Vec::new();
        };
        self.journal
            .ledger_events_by_query(&trading::ExecutionLedgerQuery {
                internal_order_id: Some(record.intent.id),
                limit: 128,
                ..trading::ExecutionLedgerQuery::default()
            })
            .into_iter()
            .filter(|event| {
                event.source == OrderUpdateSource::PrivateWs
                    && matches!(
                        event.event_type,
                        ExecutionLedgerEventType::OrderState | ExecutionLedgerEventType::Cancel
                    )
            })
            .collect()
    }
}

fn live_state_from_order_status(status: OrderStatus) -> shared_types::LiveOrderState {
    match status {
        OrderStatus::Pending | OrderStatus::Open => shared_types::LiveOrderState::Accepted,
        OrderStatus::PartiallyFilled => shared_types::LiveOrderState::PartiallyFilled,
        OrderStatus::Filled => shared_types::LiveOrderState::Filled,
        OrderStatus::Canceled => shared_types::LiveOrderState::Cancelled,
        OrderStatus::Rejected => shared_types::LiveOrderState::Rejected,
        OrderStatus::Expired => shared_types::LiveOrderState::Failed,
    }
}

fn binance_reduce_only_fill_finality(
    delta: &BinanceOrderTradeDelta,
    events: &[ExecutionLedgerEvent],
) -> Option<ExecutionLedgerEvent> {
    if !delta.terminal || delta.order.order.status != OrderStatus::Filled {
        return None;
    }
    let fill = events
        .iter()
        .find(|event| event.event_type == ExecutionLedgerEventType::FillEvent)?;
    if fill.order.reduce_only != Some(true) {
        return None;
    }
    Some(ExecutionLedgerEvent {
        event_id: format!("binance_order_finality:{}", fill.event_id),
        event_type: ExecutionLedgerEventType::OrderState,
        source: OrderUpdateSource::PrivateWs,
        order: fill.order.clone(),
        payload: ExecutionLedgerPayload::OrderState {
            state: LiveOrderState::Filled,
            message: Some(binance_finality_message(delta)),
        },
        occurred_at_ms: delta.order.received_at_ms,
        captured_at_ms: common::time::now_ms(),
    })
}

fn binance_finality_message(delta: &BinanceOrderTradeDelta) -> String {
    let base = format!(
        "binance ORDER_TRADE_UPDATE execution={} status={}",
        delta.execution_type, delta.order_status
    );
    match delta.reject_reason.as_deref() {
        Some(reason) => format!("{base} reject_reason={reason}"),
        None => base,
    }
}
