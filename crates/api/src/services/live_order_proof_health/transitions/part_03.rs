use shared_types::{
    ExecutionLedgerEvent, ExecutionLedgerEventType, ExecutionLedgerPayload,
};
use std::collections::{BTreeMap, HashSet};

impl LiveOrderProofHealthStore {
    /// Restores persisted place acknowledgements as context only. Cancel proof is deliberately not
    /// replayed because the ledger does not bind historical events to the active credential version.
    pub(crate) fn replay_persisted_place_ack_evidence(
        &self,
        records: &[OrderRecord],
        events: &[ExecutionLedgerEvent],
    ) -> usize {
        let live_order_ids = records
            .iter()
            .filter(|record| record.intent.mode == ExecutionMode::Live)
            .map(|record| record.intent.id.as_str())
            .collect::<HashSet<_>>();
        let mut latest_by_order = BTreeMap::<&str, &ExecutionLedgerEvent>::new();

        for event in events {
            let internal_order_id = event.order.identity.internal_order_id.as_str();
            if !live_order_ids.contains(internal_order_id)
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
