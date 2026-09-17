fn okx_fixture_events(body: &str) -> Vec<PrivateWsEvent> {
    let event = exchange::adapters::okx_ws_user::parse_user_event(body)
        .expect("OKX private WS fixture parses")
        .expect("OKX private WS fixture contains a data event");
    crate::trading_service::private_ws_mapper::map_okx_event(event)
}

#[tokio::test]
async fn okx_partial_fill_fixture_preserves_identity_fee_and_deduplicates() {
    let service = TradingService::new_mock();
    let mut intent = arbitrage_intent_on(
        "okx-fill-long",
        "okx-client-fill",
        OrderSide::Buy,
        "okx",
        "BTC",
    );
    intent.mode = ExecutionMode::Live;
    seed_accepted_order(
        &service,
        intent,
        "680800019749904384",
    );
    let body = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/okx/ws_user_orders_partial_fill.json"
    ));

    let mut first_events = Vec::new();
    for event in okx_fixture_events(body) {
        first_events.extend(service.apply_private_ws_event(event).await.ledger_events);
    }
    let fill = first_events
        .iter()
        .find(|event| event.event_type == ExecutionLedgerEventType::FillEvent)
        .expect("OKX fill ledger event");
    assert_eq!(fill.order.identity.internal_order_id, "okx-fill-long");
    assert_eq!(
        fill.order.identity.public_client_order_id,
        "okx-client-fill"
    );
    assert_eq!(
        fill.order.identity.exchange_order_id.as_deref(),
        Some("680800019749904384")
    );
    assert!(matches!(
        &fill.payload,
        ExecutionLedgerPayload::FillSnapshot(snapshot)
            if snapshot.quantity == 0.2
                && snapshot.average_price == 51_858.0
                && snapshot.fee.as_ref().is_some_and(|fee| {
                    fee.amount == 0.004 && fee.currency.as_deref() == Some("USDT")
                })
    ));

    let before_duplicate = service.list_execution_ledger_events().len();
    for event in okx_fixture_events(body) {
        let duplicate = service.apply_private_ws_event(event).await;
        assert!(!duplicate.ledger_updated);
    }
    assert_eq!(
        service.list_execution_ledger_events().len(),
        before_duplicate
    );
}

#[tokio::test]
async fn okx_cancel_fixture_is_final_only_after_private_order_event() {
    let service = TradingService::new_mock();
    let mut intent = arbitrage_intent_on(
        "okx-cancel-long",
        "okx-client-cancel",
        OrderSide::Sell,
        "okx",
        "BTC",
    );
    intent.mode = ExecutionMode::Live;
    seed_accepted_order(
        &service,
        intent,
        "680800019749904385",
    );
    let body = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../exchange/fixtures/okx/ws_user_orders_canceled.json"
    ));
    let mut events = okx_fixture_events(body).into_iter();
    let event = events.next().expect("one OKX cancel event");
    assert!(events.next().is_none());

    let outcome = service.apply_private_ws_event(event).await;
    assert_eq!(
        outcome.order.as_ref().map(|record| record.state),
        Some(shared_types::LiveOrderState::Cancelled)
    );
    assert!(outcome.account_cache_dirty.is_none());
    let cancel = service
        .list_execution_ledger_events()
        .into_iter()
        .find(|event| event.event_type == ExecutionLedgerEventType::Cancel)
        .expect("OKX cancel ledger event");
    assert!(matches!(
        cancel.payload,
        ExecutionLedgerPayload::OrderState {
            state: shared_types::LiveOrderState::Cancelled,
            ..
        }
    ));
    let proof = service
        .live_order_proof_health
        .snapshot(1_708_587_373_400);
    let row = proof
        .iter()
        .find(|row| row.venue == "okx")
        .expect("OKX finality health row");
    assert_eq!(row.status, shared_types::VenueOperationStatus::Warn);
    assert_eq!(row.rows, Some(1));
    assert!(row.message.contains("live 下单 ack"));
    assert_eq!(
        row.cancel_finality.as_ref().map(|sample| sample.source.as_str()),
        Some("private_ws_order")
    );
}
