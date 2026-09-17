use super::*;
use shared_types::{
    ClientOrderIdDerivation, ExecutionLedgerPayload, ExecutionMode, HedgeLegRole, OrderSide,
    OrderSource, OrderStatus, OrderTransportMetadata, OrderType, VenueOrderIdentityUpdate,
};
use std::sync::Arc;

#[test]
fn relative_order_audit_path_uses_runtime_data_dir() {
    let path =
        resolve_order_audit_path("order_events.jsonl", Some("/tmp/crossline-data".to_owned()))
            .expect("path");

    assert_eq!(
        path,
        PathBuf::from("/tmp/crossline-data/order_events.jsonl")
    );
}

#[test]
fn blank_order_audit_path_disables_writer() {
    assert!(resolve_order_audit_path("   ", Some("/tmp/crossline-data".to_owned())).is_none());
}

#[test]
fn relative_execution_ledger_path_uses_runtime_data_dir() {
    let path = resolve_execution_ledger_path(
        "execution_ledger.jsonl",
        Some("/tmp/crossline-data".to_owned()),
    )
    .expect("path");

    assert_eq!(
        path,
        PathBuf::from("/tmp/crossline-data/execution_ledger.jsonl")
    );
}

#[test]
fn open_order_count_updates_on_state_transitions() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");

    journal.insert_created(intent.clone(), 1);
    assert_eq!(journal.open_order_count(), 1);

    assert!(
        journal
            .mark_risk_checked(&intent.id, RiskDecision::allow(10.0), 2)
            .is_some()
    );
    assert_eq!(journal.open_order_count(), 1);
    assert!(journal.mark_submitted(&intent.id, 3).is_some());

    assert!(
        journal
            .apply_ack(&OrderAck {
                internal_order_id: intent.id,
                exchange_order_id: Some("x1".into()),
                client_order_id: intent.client_order_id,
                identity_update: Default::default(),
                state: LiveOrderState::Filled,
                accepted_at_ms: 4,
                message: None,
                filled_quantity: None,
                filled_price: None,
                filled_fee: None,
            })
            .is_some()
    );
    assert_eq!(journal.open_order_count(), 0);
}

#[test]
fn adapter_ack_persists_native_transport_metadata_without_identity_matching() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);
    let client_order_id = intent.client_order_id.clone();

    let updated = journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id,
            exchange_order_id: Some("x1".into()),
            client_order_id: client_order_id.clone(),
            identity_update: VenueOrderIdentityUpdate::from_ids(
                client_order_id,
                "venue-c1",
                Some("x1".to_owned()),
            )
            .with_transport_metadata(
                OrderTransportMetadata::default()
                    .with_native_transport("hyperliquid_ws_post")
                    .with_native_request_id("257")
                    .with_native_response_id("257"),
            ),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("ack accepted");

    let identity = updated.identity_snapshot();
    assert_eq!(
        identity.transport_metadata.native_transport.as_deref(),
        Some("hyperliquid_ws_post")
    );
    assert_eq!(
        identity.transport_metadata.native_request_id.as_deref(),
        Some("257")
    );
    assert_eq!(
        identity.transport_metadata.native_response_id.as_deref(),
        Some("257")
    );
    assert!(!identity.matches_order_id("257"));
}

#[test]
fn list_page_by_updated_at_desc_returns_bounded_sorted_rows() {
    let journal = OrderJournal::default_without_audit();
    for at_ms in 1..=5 {
        let id = format!("o{at_ms}");
        let client_order_id = format!("c{at_ms}");
        journal.insert_created(intent(&id, &client_order_id), at_ms);
    }

    let (rows, total_rows) = journal.list_page_by_updated_at_desc(1, 2);

    assert_eq!(total_rows, 5);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].intent.id, "o4");
    assert_eq!(rows[1].intent.id, "o3");
}

#[test]
fn list_page_by_updated_at_desc_filters_before_pagination() {
    let journal = OrderJournal::default_without_audit();
    for at_ms in 1..=5 {
        let id = format!("o{at_ms}");
        let client_order_id = format!("c{at_ms}");
        journal.insert_created(intent(&id, &client_order_id), at_ms);
    }
    for id in ["o4", "o5"] {
        journal
            .mark_risk_checked(id, RiskDecision::allow(10.0), 6)
            .unwrap_or_else(|| panic!("missing risk transition for {id}"));
        journal
            .mark_submitted(id, 7)
            .unwrap_or_else(|| panic!("missing submitted transition for {id}"));
    }

    let (rows, total_rows) = journal.list_page_by_updated_at_desc_filtered(
        0,
        10,
        Some(LiveOrderState::Submitted),
        Some(7),
    );

    assert_eq!(total_rows, 2);
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|row| row.state == LiveOrderState::Submitted)
    );
    assert!(rows.iter().all(|row| row.updated_at_ms >= 7));
}

#[test]
fn replacing_terminal_with_created_reopens_count_once() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");

    journal.insert_created(intent.clone(), 1);
    assert!(
        journal
            .mark_risk_checked(&intent.id, RiskDecision::allow(10.0), 2)
            .is_some()
    );
    assert!(journal.mark_submitted(&intent.id, 3).is_some());
    assert!(
        journal
            .update_state(&intent.id, LiveOrderState::Failed, None, 4)
            .is_some()
    );
    assert_eq!(journal.open_order_count(), 0);

    journal.insert_created(intent, 3);
    assert_eq!(journal.open_order_count(), 1);
}

#[test]
fn apply_order_info_backfills_terminal_state_and_fills() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    journal.insert_created(intent.clone(), 1);
    journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(10.0), 2)
        .expect("risk checked");
    journal.mark_submitted(&intent.id, 3).expect("submitted");
    journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("x1".into()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("accepted");

    let updated = journal
        .apply_order_info(&intent.id, &order_info(OrderStatus::Filled), 5)
        .expect("order status backfill");

    assert_eq!(updated.state, LiveOrderState::Filled);
    assert_eq!(updated.filled_quantity, Some(0.4));
    assert_eq!(updated.filled_price, Some(10.5));
    assert_eq!(updated.filled_fee, Some(0.08));
    assert_eq!(updated.last_update_source, OrderUpdateSource::OrderQuery);
    let ledger = journal.ledger_events();
    let fill = ledger
        .iter()
        .find(|event| matches!(event.payload, ExecutionLedgerPayload::FillSnapshot(_)))
        .expect("fill ledger event");
    assert_eq!(fill.source, OrderUpdateSource::OrderQuery);
    assert_eq!(fill.order.identity.exchange_order_id.as_deref(), Some("x1"));
    assert_eq!(journal.open_order_count(), 0);
}

#[test]
fn adapter_ack_filled_without_fill_evidence_does_not_create_fill_ledger() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);

    let updated = journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("x1".into()),
            client_order_id: intent.client_order_id,
            identity_update: Default::default(),
            state: LiveOrderState::Filled,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("ack filled");

    assert_eq!(updated.filled_quantity, None);
    assert_eq!(updated.filled_price, None);
    assert!(
        journal
            .ledger_events()
            .iter()
            .all(|event| !matches!(event.payload, ExecutionLedgerPayload::FillSnapshot(_)))
    );
}
