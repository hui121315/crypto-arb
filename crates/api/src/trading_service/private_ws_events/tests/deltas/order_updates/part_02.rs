#[tokio::test]
async fn private_fill_rejects_side_mismatch() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent_on("buy-order", "buy-client", OrderSide::Buy, "mock", "BTC"),
        "buy-exchange",
    );

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Fill(PrivateFillDelta {
            venue: "mock".into(),
            exchange_order_id: "buy-exchange".into(),
            client_order_id: Some("buy-client".into()),
            symbol: Some("BTC".into()),
            side: Some(OrderSide::Sell),
            venue_event_id: "wrong-side-fill".into(),
            quantity: 0.5,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 10,
        }))
        .await;

    assert!(!outcome.ledger_updated);
    assert!(service
        .list_execution_ledger_events()
        .iter()
        .all(|event| { event.event_type != ExecutionLedgerEventType::FillEvent }));
}

#[tokio::test]
async fn private_fill_rejects_conflicting_exchange_and_client_identity() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent_on("order-a", "client-a", OrderSide::Buy, "mock", "BTC"),
        "exchange-a",
    );
    seed_accepted_order(
        &service,
        arbitrage_intent_on("order-b", "client-b", OrderSide::Buy, "mock", "BTC"),
        "exchange-b",
    );

    let outcome = service
        .apply_private_ws_event(PrivateWsEvent::Fill(PrivateFillDelta {
            venue: "mock".into(),
            exchange_order_id: "exchange-a".into(),
            client_order_id: Some("client-b".into()),
            symbol: Some("BTC".into()),
            side: Some(OrderSide::Buy),
            venue_event_id: "conflicting-fill".into(),
            quantity: 0.5,
            price: 100.0,
            fee_amount: Some(0.01),
            fee_currency: Some("USDC".into()),
            occurred_at_ms: 10,
        }))
        .await;

    assert!(!outcome.ledger_updated);
    assert!(service
        .list_execution_ledger_events()
        .iter()
        .all(|event| { event.event_type != ExecutionLedgerEventType::FillEvent }));
}
#[tokio::test]
async fn duplicate_private_fill_returns_no_events_and_does_not_extend_ledger() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent_on("fill-once", "client-once", OrderSide::Buy, "mock", "BTC"),
        "exchange-once",
    );
    let fill = PrivateFillDelta {
        venue: "mock".into(),
        exchange_order_id: "exchange-once".into(),
        client_order_id: Some("client-once".into()),
        symbol: Some("BTC".into()),
        side: Some(OrderSide::Buy),
        venue_event_id: "venue-event-once".into(),
        quantity: 0.5,
        price: 100.0,
        fee_amount: Some(0.01),
        fee_currency: Some("USDC".into()),
        occurred_at_ms: 10,
    };

    let first = service
        .apply_private_ws_event(PrivateWsEvent::Fill(fill.clone()))
        .await;
    let first_ledger_len = service.list_execution_ledger_events().len();
    let duplicate = service
        .apply_private_ws_event(PrivateWsEvent::Fill(fill))
        .await;

    assert_eq!(first.ledger_events.len(), 2);
    assert!(first.ledger_updated);
    assert!(duplicate.ledger_events.is_empty());
    assert!(!duplicate.ledger_updated);
    assert_eq!(
        service.list_execution_ledger_events().len(),
        first_ledger_len
    );
}

#[tokio::test]
async fn binance_order_trade_fill_is_one_identity_preserving_ledger_outcome() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent_on(
            "binance-fill-long",
            "binance-client-1",
            OrderSide::Buy,
            "binance",
            "BTC",
        ),
        "8886774",
    );
    let mut order = order_info(OrderStatus::Filled);
    order.exchange = "binance".into();
    order.order_id = "8886774".into();
    order.client_order_id = Some("binance-client-1".into());
    order.quantity = 0.003;
    order.filled_quantity = 0.003;
    order.filled_price = 50_010.0;
    order.fees = 0.12;
    let event = BinanceOrderTradeDelta {
        order: PrivateOrderDelta {
            client_order_id: "binance-client-1".into(),
            order,
            received_at_ms: 1_568_879_465_650,
        },
        fill: Some(PrivateFillDelta {
            venue: "binance".into(),
            exchange_order_id: "8886774".into(),
            client_order_id: Some("binance-client-1".into()),
            symbol: Some("BTC".into()),
            side: Some(OrderSide::Buy),
            venue_event_id: "binance_trade:8886774:1234567".into(),
            quantity: 0.003,
            price: 50_010.0,
            fee_amount: Some(0.12),
            fee_currency: Some("USDT".into()),
            occurred_at_ms: 1_568_879_465_652,
        }),
        execution_type: "TRADE".into(),
        order_status: "FILLED".into(),
        reject_reason: Some("NONE".into()),
        terminal: true,
    };

    let first = service
        .apply_private_ws_event(PrivateWsEvent::BinanceOrderTrade(Box::new(event.clone())))
        .await;
    assert!(first.ledger_updated);
    assert!(first.order_projection_handled_by_ledger);
    assert_eq!(first.ledger_events.len(), 2);
    let fill = first
        .ledger_events
        .iter()
        .find(|event| event.event_type == ExecutionLedgerEventType::FillEvent)
        .expect("Binance fill ledger event");
    assert_eq!(fill.order.exchange, "binance");
    assert_eq!(fill.order.identity.internal_order_id, "binance-fill-long");
    assert_eq!(
        fill.order.identity.public_client_order_id,
        "binance-client-1"
    );
    assert_eq!(
        fill.order.identity.exchange_order_id.as_deref(),
        Some("8886774")
    );
    assert!(matches!(
        fill.payload,
        ExecutionLedgerPayload::FillSnapshot(_)
    ));
    if let ExecutionLedgerPayload::FillSnapshot(snapshot) = &fill.payload {
        assert_eq!(snapshot.quantity, 0.003);
        assert_eq!(snapshot.average_price, 50_010.0);
        assert_eq!(
            snapshot
                .fee
                .as_ref()
                .map(|fee| (fee.amount, fee.currency.as_deref())),
            Some((0.12, Some("USDT")))
        );
    }

    let duplicate = service
        .apply_private_ws_event(PrivateWsEvent::BinanceOrderTrade(Box::new(event)))
        .await;
    assert!(!duplicate.ledger_updated);
    assert!(duplicate.ledger_events.is_empty());
    assert!(duplicate.order.is_none());
}

#[tokio::test]
async fn binance_terminal_cancel_preserves_reason_in_order_state_ledger_once() {
    let service = TradingService::new_mock();
    seed_accepted_order(
        &service,
        arbitrage_intent_on(
            "binance-cancel-long",
            "binance-client-cancel",
            OrderSide::Buy,
            "binance",
            "BTC",
        ),
        "8886888",
    );
    let mut order = order_info(OrderStatus::Canceled);
    order.exchange = "binance".into();
    order.order_id = "8886888".into();
    order.client_order_id = Some("binance-client-cancel".into());
    order.filled_quantity = 0.0;
    order.filled_price = 0.0;
    order.fees = 0.0;
    let event = BinanceOrderTradeDelta {
        order: PrivateOrderDelta {
            client_order_id: "binance-client-cancel".into(),
            order,
            received_at_ms: 1_568_879_465_700,
        },
        fill: None,
        execution_type: "CANCELED".into(),
        order_status: "CANCELED".into(),
        reject_reason: Some("8".into()),
        terminal: true,
    };

    let first = service
        .apply_private_ws_event(PrivateWsEvent::BinanceOrderTrade(Box::new(event.clone())))
        .await;
    assert!(first.ledger_updated);
    assert!(first.order_projection_handled_by_ledger);
    assert_eq!(first.ledger_events.len(), 1);
    let state = &first.ledger_events[0];
    assert_eq!(state.event_type, ExecutionLedgerEventType::Cancel);
    assert_eq!(
        state.order.identity.internal_order_id,
        "binance-cancel-long"
    );
    assert!(matches!(
        &state.payload,
        ExecutionLedgerPayload::OrderState {
            state: shared_types::LiveOrderState::Cancelled,
            message: Some(message),
        } if message.contains("status=CANCELED") && message.contains("reject_reason=8")
    ));

    let duplicate = service
        .apply_private_ws_event(PrivateWsEvent::BinanceOrderTrade(Box::new(event)))
        .await;
    assert!(!duplicate.ledger_updated);
    assert!(duplicate.order.is_none());
}
