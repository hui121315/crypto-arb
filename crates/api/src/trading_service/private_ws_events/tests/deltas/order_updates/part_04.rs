fn bybit_fixture_events(body: &str) -> Vec<PrivateWsEvent> {
    let event = exchange::adapters::bybit_ws_user::parse_user_event(body)
        .expect("Bybit private WS fixture parses")
        .expect("Bybit private WS fixture contains a data event");
    crate::trading_service::private_ws_mapper::map_bybit_event(event)
}

async fn assert_single_bybit_order_state(
    service: &TradingService,
    body: &str,
    expected: shared_types::LiveOrderState,
) {
    let mut events = bybit_fixture_events(body).into_iter();
    let outcome = service
        .apply_private_ws_event(events.next().expect("one Bybit order event"))
        .await;
    assert!(events.next().is_none());
    assert_eq!(
        outcome.order.as_ref().map(|record| record.state),
        Some(expected)
    );
}

#[tokio::test]
async fn bybit_private_fill_and_order_finality_project_once() {
    let service = TradingService::new_mock();
    let mut fill_intent = arbitrage_intent_on(
        "bybit-fill-long",
        "bybit-client-fill-1",
        OrderSide::Buy,
        "bybit",
        "BTCPERP",
    );
    fill_intent.mode = ExecutionMode::Live;
    seed_accepted_order(&service, fill_intent, "bybit-exchange-fill-1");

    let execution = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/bybit/ws_user_execution_fill.json"
    ));
    let mut first_events = Vec::new();
    for event in bybit_fixture_events(execution) {
        first_events.extend(service.apply_private_ws_event(event).await.ledger_events);
    }
    let fill = first_events
        .iter()
        .find(|event| event.event_type == ExecutionLedgerEventType::FillEvent)
        .expect("Bybit fill ledger event");
    assert_eq!(fill.order.identity.internal_order_id, "bybit-fill-long");
    assert_eq!(
        fill.order.identity.public_client_order_id,
        "bybit-client-fill-1"
    );
    assert_eq!(
        fill.order.identity.exchange_order_id.as_deref(),
        Some("bybit-exchange-fill-1")
    );
    assert!(matches!(
        &fill.payload,
        ExecutionLedgerPayload::FillSnapshot(snapshot)
            if snapshot.quantity == 0.5
                && snapshot.average_price == 95_900.1
                && snapshot.fee.as_ref().is_some_and(|fee| {
                    fee.amount == 26.372_527_5
                        && fee.currency.as_deref() == Some("USDC")
                })
    ));

    let before_duplicate = service.list_execution_ledger_events().len();
    for event in bybit_fixture_events(execution) {
        let duplicate = service.apply_private_ws_event(event).await;
        assert!(!duplicate.ledger_updated);
    }
    assert_eq!(
        service.list_execution_ledger_events().len(),
        before_duplicate
    );

    let filled = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/bybit/ws_user_order_filled.json"
    ));
    assert_single_bybit_order_state(&service, filled, shared_types::LiveOrderState::Filled).await;

    let mut cancel_intent = arbitrage_intent_on(
        "bybit-cancel-short",
        "bybit-client-cancel-1",
        OrderSide::Sell,
        "bybit",
        "AAPLUSDT",
    );
    cancel_intent.mode = ExecutionMode::Live;
    seed_accepted_order(
        &service,
        cancel_intent,
        "bybit-exchange-cancel-1",
    );
    let canceled = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/bybit/ws_user_order_canceled.json"
    ));
    assert_single_bybit_order_state(
        &service,
        canceled,
        shared_types::LiveOrderState::Cancelled,
    )
    .await;
    let proof = service
        .live_order_proof_health
        .snapshot(1_783_913_200_500);
    let row = proof
        .iter()
        .find(|row| row.venue == "bybit")
        .expect("Bybit finality health row");
    assert_eq!(
        row.cancel_finality
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("private_ws_order")
    );
}
