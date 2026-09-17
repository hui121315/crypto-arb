#[test]
fn adapter_ack_filled_with_fill_evidence_creates_fill_ledger() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);

    let updated = journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id,
            exchange_order_id: Some("x1".into()),
            client_order_id: intent.client_order_id,
            identity_update: Default::default(),
            state: LiveOrderState::Filled,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: Some(0.6),
            filled_price: Some(10.25),
            filled_fee: Some(0.03),
        })
        .expect("ack filled");

    assert_eq!(updated.filled_quantity, Some(0.6));
    assert_eq!(updated.filled_price, Some(10.25));
    let fill = journal
        .ledger_events()
        .into_iter()
        .find(|event| matches!(event.payload, ExecutionLedgerPayload::FillSnapshot(_)))
        .expect("fill snapshot");
    assert_eq!(fill.source, OrderUpdateSource::AdapterAck);
    let slippage = journal
        .ledger_events()
        .into_iter()
        .find(|event| matches!(event.payload, ExecutionLedgerPayload::Slippage(_)))
        .expect("slippage event");
    let ExecutionLedgerPayload::Slippage(record) = &slippage.payload else {
        unreachable!("filtered to slippage payload")
    };
    assert_eq!(slippage.source, OrderUpdateSource::AdapterAck);
    assert_eq!(record.amount_usd, 0.15);
    assert_eq!(record.reference_price, 10.0);
    assert_eq!(record.fill_price, 10.25);
}

#[test]
fn adapter_ack_accepted_creates_order_state_ledger_event() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);

    journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id,
            exchange_order_id: Some("x1".into()),
            client_order_id: intent.client_order_id,
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 4,
            message: Some("accepted".to_owned()),
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("ack accepted");

    assert!(journal.ledger_events().iter().any(|event| {
        event.event_type == shared_types::ExecutionLedgerEventType::OrderState
            && event.source == OrderUpdateSource::AdapterAck
            && matches!(
                event.payload,
                ExecutionLedgerPayload::OrderState {
                    state: LiveOrderState::Accepted,
                    ..
                }
            )
    }));
}

#[test]
fn cancel_ack_creates_cancel_ledger_event() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);
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
        .expect("ack accepted");
    journal
        .update_state(&intent.id, LiveOrderState::CancelRequested, None, 5)
        .expect("cancel requested");
    journal
        .update_state(&intent.id, LiveOrderState::Cancelled, None, 6)
        .expect("cancelled");

    assert!(journal.ledger_events().iter().any(|event| {
        event.event_type == shared_types::ExecutionLedgerEventType::Cancel
            && matches!(
                event.payload,
                ExecutionLedgerPayload::OrderState {
                    state: LiveOrderState::Cancelled,
                    ..
                }
            )
    }));
}

#[test]
fn cancel_request_ack_stays_open_until_finality_backfill() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);
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

    let pending = journal
        .apply_cancel_request_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("x1".into()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: Default::default(),
            state: LiveOrderState::Cancelled,
            accepted_at_ms: 5,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("cancel request accepted");

    assert_eq!(pending.state, LiveOrderState::CancelRequested);
    assert_eq!(pending.last_update_source, OrderUpdateSource::AdapterAck);
    assert_eq!(journal.open_order_count(), 1);

    let finality = journal
        .apply_order_info(&intent.id, &order_info(OrderStatus::Canceled), 6)
        .expect("cancel finality");

    assert_eq!(finality.state, LiveOrderState::Cancelled);
    assert_eq!(finality.last_update_source, OrderUpdateSource::OrderQuery);
    assert_eq!(journal.open_order_count(), 0);
}

#[test]
fn ack_indexes_exchange_order_id() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);

    journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("x1".into()),
            client_order_id: intent.client_order_id,
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 2,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("accepted");

    let indexed = journal
        .get_by_exchange_order_id("x1")
        .expect("exchange order id indexed");
    assert_eq!(indexed.intent.id, "o1");
}

#[test]
fn ack_records_venue_client_order_id_without_losing_public_id() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "public-c1");
    submit_intent(&journal, &intent);

    let updated = journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id,
            exchange_order_id: Some("x1".into()),
            client_order_id: "venue-c1".into(),
            identity_update: Default::default(),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 2,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("accepted");

    assert_eq!(updated.identity.public_client_order_id, "public-c1");
    assert_eq!(updated.last_update_source, OrderUpdateSource::AdapterAck);
    assert_eq!(
        updated.identity.venue_client_order_id.as_deref(),
        Some("venue-c1")
    );
    assert!(updated.identity.client_order_id_policy.is_some());
    assert_eq!(
        journal
            .get_by_client_order_id("venue-c1")
            .expect("venue client id indexed")
            .intent
            .id,
        "o1"
    );
    assert_eq!(
        journal
            .get_by_client_order_id("public-c1")
            .expect("public client id remains indexed")
            .intent
            .id,
        "o1"
    );
    assert!(journal.events().iter().any(|event| {
        event.source == OrderUpdateSource::AdapterAck
            && event.identity.public_client_order_id == "public-c1"
            && event.identity.venue_client_order_id.as_deref() == Some("venue-c1")
            && event.identity.exchange_order_id.as_deref() == Some("x1")
            && event.identity.client_order_id_policy.is_some()
    }));
}

#[test]
fn insert_created_persists_client_order_id_policy() {
    let journal = OrderJournal::default_without_audit();
    let intent = arbitrage_intent("o1", "xlstableclient001", "hyperliquid:xyz", "BTC-USDC");

    let record = journal.insert_created(intent, 1);
    let policy = record
        .identity
        .client_order_id_policy
        .as_ref()
        .expect("identity policy");

    assert_eq!(policy.venue_family, "hyperliquid");
    assert_eq!(policy.venue_field, "c/cloid");
    assert_eq!(policy.derivation, ClientOrderIdDerivation::StableHash);
    assert!(
        policy
            .venue_client_order_id
            .as_deref()
            .is_some_and(|id| { id.starts_with("0x") && id.len() == 34 })
    );
    assert!(record.intent.client_order_id_policy.is_some());
    assert!(journal.events().iter().any(|event| {
        event.internal_order_id == "o1"
            && event
                .identity
                .client_order_id_policy
                .as_ref()
                .is_some_and(|event_policy| {
                    event_policy.venue_family == "hyperliquid"
                        && event_policy.derivation == ClientOrderIdDerivation::StableHash
                })
    }));
}
