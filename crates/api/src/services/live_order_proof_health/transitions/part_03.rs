use shared_types::{ExecutionLedgerEvent, ExecutionLedgerEventType, ExecutionLedgerPayload};
use std::collections::{BTreeMap, HashMap};

impl LiveOrderProofHealthStore {
    /// Restore ACK context only after the active reader's credential scopes are known.
    pub(crate) fn replay_persisted_place_ack_evidence(
        &self,
        records: &[OrderRecord],
        events: &[ExecutionLedgerEvent],
        accounts: &HashMap<(String, shared_types::FeeProduct), String>,
    ) -> usize {
        let live_orders = records
            .iter()
            .filter(|record| {
                let identity = record.identity_snapshot();
                record.intent.mode == ExecutionMode::Live
                    && identity.account_scope.as_ref().is_some_and(|scope| {
                        accounts.get(&(
                            normalized_venue_name(&record.intent.exchange),
                            identity.product,
                        )) == Some(scope)
                    })
            })
            .map(|record| (record.intent.id.as_str(), record))
            .collect::<HashMap<_, _>>();
        let mut latest_by_order = BTreeMap::<&str, &ExecutionLedgerEvent>::new();

        for event in events {
            let internal_order_id = event.order.identity.internal_order_id.as_str();
            let Some(record) = live_orders.get(internal_order_id) else {
                continue;
            };
            let identity = record.identity_snapshot();
            if event.order.identity.account_scope != identity.account_scope
                || event.order.identity.product != identity.product
                || normalized_venue_name(&event.order.exchange)
                    != normalized_venue_name(&record.intent.exchange)
                || event.order.symbol != record.intent.symbol
                || event.event_type != ExecutionLedgerEventType::OrderState
                || event.source != OrderUpdateSource::AdapterAck
                || !event_has_place_ack_state(event)
            {
                continue;
            }
            let replace = latest_by_order
                .get(internal_order_id)
                .is_none_or(|current| event.occurred_at_ms > current.occurred_at_ms);
            if replace {
                latest_by_order.insert(internal_order_id, event);
            }
        }

        for event in latest_by_order.values() {
            self.record_place_ack(ledger_event_sample(
                event,
                "execution_ledger_replay:adapter_ack",
            ));
        }
        latest_by_order.len()
    }
}

fn event_has_place_ack_state(event: &ExecutionLedgerEvent) -> bool {
    matches!(
        &event.payload,
        ExecutionLedgerPayload::OrderState { state, .. } if place_ack_state(*state)
    )
}

fn ledger_event_sample(event: &ExecutionLedgerEvent, source: &str) -> LiveOrderProofSample {
    let identity = &event.order.identity;
    let transport = &identity.transport_metadata;
    LiveOrderProofSample {
        venue: event.order.exchange.clone(),
        account_scope: identity.account_scope.clone(),
        product: identity.product,
        symbol: event.order.symbol.clone(),
        internal_order_id: identity.internal_order_id.clone(),
        exchange_order_id: identity.exchange_order_id.clone(),
        client_order_id: non_empty_identity(&identity.public_client_order_id)
            .or_else(|| identity.venue_client_order_id.clone()),
        source: source.to_owned(),
        checked_at_ms: event.occurred_at_ms,
        request_id: None,
        native_transport: transport.native_transport.clone(),
        native_request_id: transport.native_request_id.clone(),
        native_response_id: transport.native_response_id.clone(),
    }
}

fn non_empty_identity(value: &str) -> Option<String> {
    (!value.trim().is_empty()).then(|| value.to_owned())
}
