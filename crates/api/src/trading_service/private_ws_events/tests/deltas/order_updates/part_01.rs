use super::*;

#[tokio::test]
async fn order_delta_updates_journal_by_client_order_id() {
    let service = TradingService::new_mock();
    let intent = intent("i1", "c1");
    seed_account_order(&service, intent, 1);
    service
        .journal
        .mark_risk_checked("i1", shared_types::RiskDecision::allow(50_000.0), 2);
    service.journal.mark_submitted("i1", 3);

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Order(PrivateOrderDelta {
            client_order_id: "c1".into(),
            order: order_info(OrderStatus::Filled),
            received_at_ms: 4,
        }))
        .await;

    assert!(outcome.account_cache_dirty.is_some());
    assert_eq!(
        outcome.order.as_ref().map(|record| record.state),
        Some(shared_types::LiveOrderState::Filled)
    );
    assert_eq!(
        outcome
            .order
            .as_ref()
            .map(|record| record.last_update_source),
        Some(OrderUpdateSource::PrivateWs)
    );
    assert_eq!(
        outcome
            .order
            .as_ref()
            .and_then(|record| record.filled_quantity),
        Some(1.0)
    );
}

#[tokio::test]
async fn private_ws_cancelled_order_delta_records_live_order_proof() {
    let service = TradingService::new_mock();
    let mut intent = intent("i1", "c1");
    intent.mode = ExecutionMode::Live;
    seed_accepted_order(&service, intent, "e1");
    let accepted = service.get_order("i1").expect("accepted order");
    service
        .live_order_proof_health
        .record_submit_ack_from_record(&accepted);

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Order(PrivateOrderDelta {
            client_order_id: "c1".into(),
            order: order_info(OrderStatus::Canceled),
            received_at_ms: 12,
        }))
        .await;

    assert_eq!(
        outcome.order.as_ref().map(|record| record.state),
        Some(shared_types::LiveOrderState::Cancelled)
    );
    let proof = service.live_order_proof_health.snapshot(13);
    assert_eq!(proof.len(), 1);
    assert_eq!(proof[0].status, shared_types::VenueOperationStatus::Ok);
    assert_eq!(
        proof[0]
            .cancel_finality
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("private_ws_order")
    );
}

#[tokio::test]
async fn order_delta_falls_back_to_exchange_order_id() {
    let service = TradingService::new_mock();
    let intent = intent("i1", "c1");
    seed_account_order(&service, intent.clone(), 1);
    service
        .journal
        .mark_risk_checked("i1", shared_types::RiskDecision::allow(50_000.0), 2);
    service.journal.mark_submitted("i1", 3);
    service.journal.apply_ack(&shared_types::OrderAck {
        internal_order_id: intent.id,
        exchange_order_id: Some("e1".into()),
        client_order_id: intent.client_order_id,
        identity_update: Default::default(),
        state: shared_types::LiveOrderState::Accepted,
        accepted_at_ms: 4,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    });

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Order(PrivateOrderDelta {
            client_order_id: "venue-normalized-client-id".into(),
            order: order_info(OrderStatus::Filled),
            received_at_ms: 5,
        }))
        .await;

    assert!(outcome.account_cache_dirty.is_some());
    let record = outcome.order.expect("exchange order id fallback");
    assert_eq!(record.intent.id, "i1");
    assert_eq!(record.state, shared_types::LiveOrderState::Filled);
    assert_eq!(record.exchange_order_id.as_deref(), Some("e1"));
    assert_eq!(record.last_update_source, OrderUpdateSource::PrivateWs);
}

#[tokio::test]
async fn fill_delta_records_ledger_by_exchange_order_id() {
    let service = TradingService::new_mock();
    let intent = intent("i1", "c1");
    seed_account_order(&service, intent.clone(), 1);
    service
        .journal
        .mark_risk_checked("i1", shared_types::RiskDecision::allow(50_000.0), 2);
    service.journal.mark_submitted("i1", 3);
    service.journal.apply_ack(&shared_types::OrderAck {
        internal_order_id: intent.id,
        exchange_order_id: Some("e1".into()),
        client_order_id: intent.client_order_id,
        identity_update: Default::default(),
        state: shared_types::LiveOrderState::Accepted,
        accepted_at_ms: 4,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
    });

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Fill(PrivateFillDelta {
            venue: "mock".into(),
            exchange_order_id: "e1".into(),
            client_order_id: Some("c1".into()),
            symbol: Some("BTC".into()),
            side: Some(OrderSide::Buy),
            venue_event_id: "hl-hash-1".into(),
            quantity: 0.25,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 10,
        }))
        .await;

    assert!(outcome.ledger_updated);
    assert!(outcome.account_cache_dirty.is_some());
    assert_eq!(
        outcome
            .ledger_events
            .iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec![
            ExecutionLedgerEventType::FillEvent,
            ExecutionLedgerEventType::Slippage,
        ]
    );
    let events = service.list_execution_ledger_events();
    assert!(events.iter().any(|event| {
        event.source == OrderUpdateSource::PrivateWs
            && event.order.identity.exchange_order_id.as_deref() == Some("e1")
            && matches!(event.payload, ExecutionLedgerPayload::FillSnapshot(_))
    }));
}

#[tokio::test]
async fn unmatched_fill_delta_clears_cache_without_fabricating_ledger() {
    let service = TradingService::new_mock();

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Fill(PrivateFillDelta {
            venue: "mock".into(),
            exchange_order_id: "missing".into(),
            client_order_id: None,
            symbol: Some("BTC".into()),
            side: Some(OrderSide::Buy),
            venue_event_id: "hl-hash-1".into(),
            quantity: 0.25,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 10,
        }))
        .await;

    assert!(!outcome.ledger_updated);
    assert!(outcome.account_cache_dirty.is_some());
    assert!(service.list_execution_ledger_events().is_empty());
}

#[tokio::test]
async fn fill_delta_falls_back_to_guarded_client_id() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent_on(
            "hedge-client-long",
            "client-fill",
            OrderSide::Buy,
            "mock",
            "BTC",
        ),
        "exchange-fill",
    );

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Fill(PrivateFillDelta {
            venue: "mock".into(),
            exchange_order_id: "missing-exchange-id".into(),
            client_order_id: Some("client-fill".into()),
            symbol: Some("BTC-USDT".into()),
            side: Some(OrderSide::Buy),
            venue_event_id: "client-fill-event".into(),
            quantity: 0.5,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 10,
        }))
        .await;

    assert!(outcome.ledger_updated);
    let event = outcome.ledger_events.first().expect("fill ledger event");
    assert_eq!(event.order.identity.internal_order_id, "hedge-client-long");
    assert_eq!(event.order.symbol, "BTC");
}

#[tokio::test]
async fn private_fill_rejects_cross_venue_exchange_id() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent_on("gate-long", "gate-client", OrderSide::Buy, "gate", "BTC"),
        "shared-exchange-id",
    );

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Fill(PrivateFillDelta {
            venue: "bybit".into(),
            exchange_order_id: "shared-exchange-id".into(),
            client_order_id: None,
            symbol: Some("BTC".into()),
            side: Some(OrderSide::Buy),
            venue_event_id: "wrong-venue-fill".into(),
            quantity: 0.5,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 10,
        }))
        .await;

    assert!(!outcome.ledger_updated);
    assert!(
        service
            .list_execution_ledger_events()
            .iter()
            .all(|event| { event.event_type != ExecutionLedgerEventType::FillEvent })
    );
}
