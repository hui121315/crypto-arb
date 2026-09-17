#[test]
fn apply_order_info_patches_same_state_fill_fields() {
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
            state: LiveOrderState::PartiallyFilled,
            accepted_at_ms: 4,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("partial");

    let updated = journal
        .apply_order_info(&intent.id, &order_info(OrderStatus::PartiallyFilled), 5)
        .expect("same-state patch");

    assert_eq!(updated.state, LiveOrderState::PartiallyFilled);
    assert_eq!(updated.filled_quantity, Some(0.4));
    assert_eq!(updated.last_update_source, OrderUpdateSource::OrderQuery);
    assert!(
        journal
            .ledger_events()
            .iter()
            .any(|event| { matches!(event.payload, ExecutionLedgerPayload::FillSnapshot(_)) })
    );
}

#[test]
fn apply_order_info_by_exchange_order_id_reuses_exchange_index() {
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

    let updated = journal
        .apply_order_info_by_exchange_order_id("x1", &order_info(OrderStatus::Filled), 3)
        .expect("exchange id indexed");

    assert_eq!(updated.intent.id, "o1");
    assert_eq!(updated.state, LiveOrderState::Filled);
    assert_eq!(journal.open_order_count(), 0);
}

#[test]
fn update_state_by_exchange_order_id_uses_requested_source() {
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

    let updated = journal
        .update_state_by_exchange_order_id_from_source(
            "x1",
            LiveOrderState::Cancelled,
            Some("non-user cancel".to_owned()),
            3,
            OrderUpdateSource::PrivateWs,
        )
        .expect("cancel by exchange id");

    assert_eq!(updated.intent.id, "o1");
    assert_eq!(updated.state, LiveOrderState::Cancelled);
    assert_eq!(updated.last_update_source, OrderUpdateSource::PrivateWs);
    assert_eq!(journal.open_order_count(), 0);
    assert!(journal.ledger_events().iter().any(|event| {
        event.event_type == shared_types::ExecutionLedgerEventType::Cancel
            && event.source == OrderUpdateSource::PrivateWs
    }));
}

#[test]
fn record_fill_by_exchange_order_id_writes_actual_ledger_event() {
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

    let event = journal
        .record_fill_by_exchange_order_id(
            "x1",
            &FillLedgerInput {
                venue_event_id: "fill-1".into(),
                quantity: 0.25,
                price: 11.0,
                fee_amount: Some(0.02),
                fee_currency: Some("USDC".into()),
                occurred_at_ms: 8,
            },
            OrderUpdateSource::PrivateWs,
            9,
        )
        .expect("fill ledger event");

    assert_eq!(event.source, OrderUpdateSource::PrivateWs);
    assert_eq!(
        event.order.identity.exchange_order_id.as_deref(),
        Some("x1")
    );
    assert!(matches!(
        event.payload,
        ExecutionLedgerPayload::FillSnapshot(_)
    ));
    assert!(journal.ledger_events().iter().any(|event| {
        event.event_id == "slippage:fill_event:o1:private_ws:fill-1"
            && event.event_type == shared_types::ExecutionLedgerEventType::Slippage
            && matches!(
                event.payload,
                ExecutionLedgerPayload::Slippage(shared_types::SlippageLedgerRecord {
                    amount_usd: 0.25,
                    reference_price: 10.0,
                    fill_price: 11.0,
                    quantity: 0.25,
                    quality: shared_types::ExecutionLedgerQuality::Actual,
                })
            )
    }));
}

#[test]
fn record_fill_by_order_identity_falls_back_to_client_id() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);

    let events = journal
        .record_fill_by_order_identity(
            FillOrderIdentity {
                venue: Some("mock"),
                exchange_order_id: Some("missing"),
                client_order_id: Some("c1"),
                symbol: Some("BTC-USDT"),
                side: Some(OrderSide::Buy),
            },
            &FillLedgerInput {
                venue_event_id: "fill-1".into(),
                quantity: 0.25,
                price: 11.0,
                fee_amount: Some(0.02),
                fee_currency: Some("USDC".into()),
                occurred_at_ms: 8,
            },
            OrderUpdateSource::PrivateWs,
            9,
        );
    let event = events.first().expect("client id fill ledger event");

    assert_eq!(event.order.identity.internal_order_id, "o1");
    assert_eq!(event.order.symbol, "BTC");
}

#[test]
fn record_fill_by_order_identity_rejects_symbol_mismatch() {
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

    let event = journal.record_fill_by_order_identity(
        FillOrderIdentity {
            venue: Some("mock"),
            exchange_order_id: Some("x1"),
            client_order_id: None,
            symbol: Some("ETH-USDT"),
            side: Some(OrderSide::Buy),
        },
        &FillLedgerInput {
            venue_event_id: "wrong-symbol-fill".into(),
            quantity: 0.25,
            price: 11.0,
            fee_amount: Some(0.02),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 8,
        },
        OrderUpdateSource::PrivateWs,
        9,
    );

    assert!(event.is_empty());
    assert!(
        journal
            .ledger_events()
            .iter()
            .all(|event| { event.event_type != shared_types::ExecutionLedgerEventType::FillEvent })
    );
}
