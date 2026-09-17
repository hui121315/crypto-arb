#[test]
fn fill_projection_returns_fill_then_slippage_and_suppresses_duplicate_event() {
    let journal = OrderJournal::default_without_audit();
    let intent = arbitrage_intent("o1", "c1", "mock", "BTC");
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
    let input = FillLedgerInput {
        venue_event_id: "venue-fill-1".into(),
        quantity: 0.25,
        price: 11.0,
        fee_amount: Some(0.02),
        fee_currency: Some("USDC".into()),
        occurred_at_ms: 8,
    };
    let identity = FillOrderIdentity {
        venue: Some("mock"),
        exchange_order_id: Some("x1"),
        client_order_id: Some("c1"),
        symbol: Some("BTC"),
        side: Some(OrderSide::Buy),
    };

    let events = journal.record_fill_by_order_identity(
        identity,
        &input,
        OrderUpdateSource::PrivateWs,
        9,
    );

    assert_eq!(events.len(), 2);
    assert_eq!(
        events[0].event_type,
        shared_types::ExecutionLedgerEventType::FillEvent
    );
    assert_eq!(
        events[1].event_type,
        shared_types::ExecutionLedgerEventType::Slippage
    );
    assert_eq!(
        events[1].event_id,
        format!("slippage:{}", events[0].event_id)
    );
    assert!(matches!(
        events[1].payload,
        ExecutionLedgerPayload::Slippage(shared_types::SlippageLedgerRecord {
            amount_usd: 0.25,
            ..
        })
    ));

    let duplicate = journal.record_fill_by_order_identity(
        identity,
        &input,
        OrderUpdateSource::PrivateWs,
        10,
    );
    assert!(duplicate.is_empty());
    assert_eq!(
        journal
            .ledger_events()
            .iter()
            .filter(|event| {
                matches!(
                    event.event_type,
                    shared_types::ExecutionLedgerEventType::FillEvent
                        | shared_types::ExecutionLedgerEventType::Slippage
                )
            })
            .count(),
        2
    );

    let fill_event = events[0].clone();
    let mut restarted = journal;
    restarted.execution_ledger = ExecutionLedger::from_events([fill_event.clone()]);

    let repaired = restarted.record_fill_by_order_identity(
        identity,
        &FillLedgerInput {
            quantity: 99.0,
            price: 999.0,
            ..input
        },
        OrderUpdateSource::PrivateWs,
        11,
    );

    assert_eq!(repaired.len(), 1);
    assert_eq!(repaired[0].event_id, format!("slippage:{}", fill_event.event_id));
    assert!(matches!(
        repaired[0].payload,
        ExecutionLedgerPayload::Slippage(shared_types::SlippageLedgerRecord {
            amount_usd: 0.25,
            reference_price: 10.0,
            fill_price: 11.0,
            quantity: 0.25,
            ..
        })
    ));
    assert!(restarted
        .record_fill_by_order_identity(
            identity,
            &FillLedgerInput {
                venue_event_id: "venue-fill-1".into(),
                quantity: 0.25,
                price: 11.0,
                fee_amount: Some(0.02),
                fee_currency: Some("USDC".into()),
                occurred_at_ms: 8,
            },
            OrderUpdateSource::PrivateWs,
            12,
        )
        .is_empty());
    assert_eq!(restarted.ledger_events().len(), 2);
}

#[test]
fn concurrent_funding_paths_emit_identical_event_once() {
    const CALLERS: usize = 32;

    let journal = Arc::new(OrderJournal::default_without_audit());
    let intent = arbitrage_intent("funding-order", "funding-client", "hyperliquid", "BTC-USDC");
    submit_intent(&journal, &intent);
    journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("funding-exchange".into()),
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
    let input = FundingLedgerInput {
        venue_event_id: "hl-funding:BTC:10".into(),
        amount: -0.12,
        currency: "USDC".into(),
        funding_time_ms: 10,
    };
    let barrier = Arc::new(std::sync::Barrier::new(CALLERS));
    let handles = (0..CALLERS)
        .map(|index| {
            let journal = Arc::clone(&journal);
            let barrier = Arc::clone(&barrier);
            let input = input.clone();
            std::thread::spawn(move || {
                barrier.wait();
                if index % 2 == 0 {
                    journal.record_funding_by_venue_symbol_reported_deferred_sql(
                        "hyperliquid",
                        "BTC",
                        &input,
                        OrderUpdateSource::PrivateWs,
                        11,
                    )
                } else {
                    journal.record_funding_by_venue_symbol_reported(
                        "hyperliquid",
                        "BTC",
                        &input,
                        OrderUpdateSource::PrivateWs,
                        11,
                    )
                }
            })
        })
        .collect::<Vec<_>>();
    let emitted = handles
        .into_iter()
        .filter_map(|handle| handle.join().expect("funding recorder thread").ok())
        .collect::<Vec<_>>();

    assert_eq!(emitted.len(), 1, "only one event may be enqueued");
    assert_eq!(
        journal
            .ledger_events()
            .iter()
            .filter(|event| {
                event.event_type == shared_types::ExecutionLedgerEventType::FundingPayment
            })
            .count(),
        1
    );
}
