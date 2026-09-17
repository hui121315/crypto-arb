#[test]
fn record_fill_by_order_identity_rejects_side_mismatch() {
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
            symbol: Some("BTC-USDT"),
            side: Some(OrderSide::Sell),
        },
        &FillLedgerInput {
            venue_event_id: "wrong-side-fill".into(),
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

#[test]
fn record_fill_by_order_identity_allows_hyperliquid_family_fill_with_exact_symbol_guard() {
    let journal = OrderJournal::default_without_audit();
    let intent = arbitrage_intent("o1", "c1", "hyperliquid:xyz", "BTC-USDC");
    submit_intent(&journal, &intent);
    journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("hl-oid-1".into()),
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

    let events = journal
        .record_fill_by_order_identity(
            FillOrderIdentity {
                venue: Some("hyperliquid"),
                exchange_order_id: Some("hl-oid-1"),
                client_order_id: None,
                symbol: Some("BTC"),
                side: Some(OrderSide::Buy),
            },
            &FillLedgerInput {
                venue_event_id: "hl-family-fill".into(),
                quantity: 0.25,
                price: 11.0,
                fee_amount: Some(0.02),
                fee_currency: Some("USDC".into()),
                occurred_at_ms: 8,
            },
            OrderUpdateSource::PrivateWs,
            9,
        );
    let event = events
        .first()
        .expect("hyperliquid family fill ledger event");

    assert_eq!(event.order.identity.internal_order_id, "o1");
    assert_eq!(event.order.exchange, "hyperliquid:xyz");
}

#[test]
fn execution_ledger_context_follows_exchange_fill_events() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    journal.attach_execution_ledger_context(
        &intent.id,
        ExecutionLedgerOrderContext::new(
            "run-1".to_owned(),
            "ticket-1".to_owned(),
            HedgeLegRole::Long,
        ),
    );
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
    let rows = journal.ledger_events_by_query(&ExecutionLedgerQuery {
        run_id: Some("run-1".to_owned()),
        ticket_id: Some("ticket-1".to_owned()),
        leg_role: Some(HedgeLegRole::Long),
        limit: 10,
        ..ExecutionLedgerQuery::default()
    });
    assert_eq!(event.order.run_id.as_deref(), Some("run-1"));
    assert_eq!(event.order.ticket_id.as_deref(), Some("ticket-1"));
    assert_eq!(event.order.leg_role, Some(HedgeLegRole::Long));
    assert!(rows.iter().any(|row| row.event_id == event.event_id));
}

#[test]
fn execution_ledger_jsonl_replays_fill_events_after_restart() {
    let path = temp_ledger_path("fill-replay");
    {
        let journal = OrderJournal::new_with_ledger_path(path.clone());
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
        journal
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
    }

    let restored = OrderJournal::new_with_ledger_path(path.clone());
    let events = restored.ledger_events();

    assert!(events.iter().any(|event| {
        event.event_id == "fill_event:o1:private_ws:fill-1"
            && event.event_type == shared_types::ExecutionLedgerEventType::FillEvent
            && event.order.identity.exchange_order_id.as_deref() == Some("x1")
    }));
    let _ = std::fs::remove_file(path);
}

#[test]
fn execution_ledger_jsonl_replays_funding_payments_after_restart() {
    let path = temp_ledger_path("funding-replay");
    {
        let journal = OrderJournal::new_with_ledger_path(path.clone());
        let intent = arbitrage_intent("hedge-1-long", "c1", "hyperliquid", "BTC-USDC");
        submit_intent(&journal, &intent);
        journal
            .apply_ack(&OrderAck {
                internal_order_id: intent.id.clone(),
                exchange_order_id: Some("x1".into()),
                client_order_id: intent.client_order_id,
                identity_update: Default::default(),
                state: LiveOrderState::Filled,
                accepted_at_ms: 4,
                message: None,
                filled_quantity: Some(1.0),
                filled_price: Some(100.0),
                filled_fee: Some(0.01),
            })
            .expect("filled order");
        journal
            .record_funding_by_venue_symbol_reported(
                "hyperliquid",
                "BTC",
                &FundingLedgerInput {
                    venue_event_id: "hl-funding:BTC:10".into(),
                    amount: -0.12,
                    currency: "USDC".into(),
                    funding_time_ms: 10,
                },
                OrderUpdateSource::PrivateWs,
                11,
            )
            .expect("funding ledger event");
    }

    let restored = OrderJournal::new_with_ledger_path(path.clone());
    let events = restored.ledger_events();

    assert!(events.iter().any(|event| {
        event.event_id == "funding_payment:hedge-1-long:private_ws:hl-funding:BTC:10"
            && event.event_type == shared_types::ExecutionLedgerEventType::FundingPayment
            && matches!(
                event.payload,
                ExecutionLedgerPayload::FundingPayment(shared_types::FundingPaymentLedgerRecord {
                    amount: -0.12,
                    ref currency,
                    ..
                }) if currency == "USDC"
            )
    }));
    let _ = std::fs::remove_file(path);
}
