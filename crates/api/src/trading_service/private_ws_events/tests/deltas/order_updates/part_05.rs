fn bitget_fixture_events(body: &str) -> Vec<PrivateWsEvent> {
    let event = exchange::adapters::bitget_uta_ws_user::parse_user_event(body)
        .expect("Bitget private WS fixture parses")
        .expect("Bitget private WS fixture contains a data event");
    crate::trading_service::private_ws_mapper::map_bitget_event(event)
}

async fn assert_single_bitget_order_state(
    service: &TradingService,
    body: &str,
    expected: shared_types::LiveOrderState,
) {
    let mut events = bitget_fixture_events(body).into_iter();
    let outcome = service
        .apply_private_ws_event(events.next().expect("one Bitget order event"))
        .await;
    assert!(events.next().is_none());
    assert_eq!(
        outcome.order.as_ref().map(|record| record.state),
        Some(expected)
    );
}

#[tokio::test]
async fn bitget_private_fill_and_order_finality_project_once() {
    let service = TradingService::new_mock();
    let mut fill_intent = arbitrage_intent_on(
        "bitget-fill-long",
        "crossline-10001",
        OrderSide::Buy,
        "bitget",
        "BTCPERP",
    );
    fill_intent.mode = ExecutionMode::Live;
    seed_accepted_order(&service, fill_intent, "10001");

    let fill = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/bitget/uta_ws_fill.json"
    ));
    let mut ledger_events = Vec::new();
    for event in bitget_fixture_events(fill) {
        ledger_events.extend(service.apply_private_ws_event(event).await.ledger_events);
    }
    let fill_event = ledger_events
        .iter()
        .find(|event| event.event_type == ExecutionLedgerEventType::FillEvent)
        .expect("Bitget fill ledger event");
    assert_eq!(fill_event.order.identity.internal_order_id, "bitget-fill-long");
    assert!(matches!(
        &fill_event.payload,
        ExecutionLedgerPayload::FillSnapshot(snapshot)
            if snapshot.quantity == 0.02
                && snapshot.average_price == 50_000.0
                && snapshot.fee.as_ref().is_some_and(|fee| {
                    fee.amount == -0.6 && fee.currency.as_deref() == Some("USDC")
                })
    ));

    let before_duplicate = service.list_execution_ledger_events().len();
    for event in bitget_fixture_events(fill) {
        assert!(!service.apply_private_ws_event(event).await.ledger_updated);
    }
    assert_eq!(service.list_execution_ledger_events().len(), before_duplicate);

    let filled = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/bitget/uta_ws_order_filled.json"
    ));
    assert_single_bitget_order_state(&service, filled, shared_types::LiveOrderState::Filled).await;

    let mut cancel_intent = arbitrage_intent_on(
        "bitget-cancel-short",
        "crossline-10002",
        OrderSide::Sell,
        "bitget",
        "ETHUSDT",
    );
    cancel_intent.mode = ExecutionMode::Live;
    seed_accepted_order(&service, cancel_intent, "10002");
    let cancelled = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/bitget/uta_ws_order_cancelled.json"
    ));
    assert_single_bitget_order_state(
        &service,
        cancelled,
        shared_types::LiveOrderState::Cancelled,
    )
    .await;

    let proof = service.live_order_proof_health.snapshot(1_740_000_020_000);
    let row = proof
        .iter()
        .find(|row| row.venue == "bitget")
        .expect("Bitget finality health row");
    assert_eq!(
        row.cancel_finality
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("private_ws_order")
    );
}
