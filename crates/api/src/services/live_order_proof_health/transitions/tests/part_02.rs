#[test]
fn private_ws_cancel_finality_records_distinct_source_for_live_only() {
    let store = LiveOrderProofHealthStore::default();
    store.record_submit_ack_from_record(&record(
        ExecutionMode::Live,
        LiveOrderState::Accepted,
        OrderUpdateSource::AdapterAck,
        1_000,
    ));
    store.record_private_ws_cancel_finality_from_record(
        &record(
            ExecutionMode::DryRun,
            LiveOrderState::Cancelled,
            OrderUpdateSource::PrivateWs,
            1_100,
        ),
        "private_ws_order",
    );
    store.record_private_ws_cancel_finality_from_record(
        &record(
            ExecutionMode::Live,
            LiveOrderState::Cancelled,
            OrderUpdateSource::OrderQuery,
            1_200,
        ),
        "private_ws_order",
    );
    store.record_private_ws_cancel_finality_from_record(
        &record(
            ExecutionMode::Live,
            LiveOrderState::Cancelled,
            OrderUpdateSource::PrivateWs,
            1_300,
        ),
        "private_ws_non_user_cancel",
    );

    let rows = store.snapshot(1_400);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, VenueOperationStatus::Ok);
    assert_eq!(rows[0].cancel_finality_count, 1);
    assert_eq!(
        rows[0]
            .cancel_finality
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("private_ws_non_user_cancel")
    );
}

#[test]
fn record_sample_preserves_native_transport_ids() {
    let store = LiveOrderProofHealthStore::default();
    let mut record = record(
        ExecutionMode::Live,
        LiveOrderState::Accepted,
        OrderUpdateSource::AdapterAck,
        1_000,
    );
    record.identity = record.identity_snapshot();
    record.identity.transport_metadata = shared_types::OrderTransportMetadata::default()
        .with_native_transport("hyperliquid_ws_post")
        .with_native_request_id("257")
        .with_native_response_id("257");

    store.record_submit_ack_from_record(&record);

    let rows = store.snapshot(1_100);
    assert_eq!(
        rows[0]
            .place_proof
            .as_ref()
            .and_then(|sample| sample.native_transport.as_deref()),
        Some("hyperliquid_ws_post")
    );
    assert_eq!(
        rows[0]
            .place_proof
            .as_ref()
            .and_then(|sample| sample.native_request_id.as_deref()),
        Some("257")
    );
    assert_eq!(
        rows[0]
            .place_proof
            .as_ref()
            .and_then(|sample| sample.native_response_id.as_deref()),
        Some("257")
    );
}

#[test]
fn persisted_replay_restores_only_live_place_ack_context() {
    let store = LiveOrderProofHealthStore::default();
    let mut live = record(
        ExecutionMode::Live,
        LiveOrderState::Filled,
        OrderUpdateSource::PrivateWs,
        1_200,
    );
    live.identity = live.identity_snapshot();
    let accounts = HashMap::from([(
        (live.intent.exchange.clone(), live.identity.product),
        live.identity
            .account_scope
            .clone()
            .expect("bound test record"),
    )]);
    let dry_run = record(
        ExecutionMode::DryRun,
        LiveOrderState::Filled,
        OrderUpdateSource::AdapterAck,
        1_300,
    );
    let events = vec![
        ledger_order_state(&live, LiveOrderState::Accepted, 1_000),
        ledger_order_state(&live, LiveOrderState::Filled, 1_100),
        ledger_order_state(&dry_run, LiveOrderState::Filled, 1_200),
    ];

    let mut old = live.clone();
    old.identity.account_scope = Some("old-account".into());
    let mut legacy = live.clone();
    legacy.identity.account_scope = None;
    assert_eq!(
        store.replay_persisted_place_ack_evidence(&[old.clone()], &events, &accounts),
        0
    );
    assert_eq!(
        store.replay_persisted_place_ack_evidence(&[legacy], &events, &accounts),
        0
    );
    assert_eq!(
        store.replay_persisted_place_ack_evidence(
            std::slice::from_ref(&live),
            &[ledger_order_state(&old, LiveOrderState::Accepted, 1_100)],
            &accounts
        ),
        0
    );
    let restored = store.replay_persisted_place_ack_evidence(&[live], &events, &accounts);
    let rows = store.snapshot(1_400);

    assert_eq!(restored, 1);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].status, VenueOperationStatus::Warn);
    assert_eq!(rows[0].place_ack_count, 1);
    assert_eq!(rows[0].cancel_requested_count, 0);
    assert_eq!(rows[0].cancel_finality_count, 0);
    assert_eq!(
        rows[0]
            .place_proof
            .as_ref()
            .map(|sample| sample.source.as_str()),
        Some("execution_ledger_replay:adapter_ack")
    );
}

fn ledger_order_state(
    record: &OrderRecord,
    state: LiveOrderState,
    occurred_at_ms: i64,
) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("{}-{occurred_at_ms}", record.intent.id),
        event_type: ExecutionLedgerEventType::OrderState,
        source: OrderUpdateSource::AdapterAck,
        order: shared_types::ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: None,
            reduce_only: Some(record.intent.reduce_only),
            exchange: record.intent.exchange.clone(),
            symbol: record.intent.symbol.clone(),
            side: record.intent.side,
            identity: record.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::OrderState {
            state,
            message: None,
        },
        occurred_at_ms,
        captured_at_ms: occurred_at_ms,
    }
}

fn sample(
    venue: &str,
    internal_order_id: &str,
    checked_at_ms: i64,
    request_id: Option<&str>,
) -> LiveOrderProofSample {
    LiveOrderProofSample {
        venue: venue.to_owned(),
        account_scope: credential_fingerprint_v1(venue, shared_types::FeeProduct::Perp),
        product: shared_types::FeeProduct::Perp,
        symbol: "BTCUSDT".to_owned(),
        internal_order_id: internal_order_id.to_owned(),
        exchange_order_id: Some(format!("exchange-{internal_order_id}")),
        client_order_id: Some(format!("client-{internal_order_id}")),
        source: "test".to_owned(),
        checked_at_ms,
        request_id: request_id.map(str::to_owned),
        native_transport: None,
        native_request_id: None,
        native_response_id: None,
    }
}

fn record(
    mode: ExecutionMode,
    state: LiveOrderState,
    source: OrderUpdateSource,
    updated_at_ms: i64,
) -> OrderRecord {
    OrderRecord {
        intent: shared_types::OrderIntent {
            id: "internal-1".to_owned(),
            source: shared_types::OrderSource::Manual,
            strategy: None,
            mode,
            exchange: "binance".to_owned(),
            symbol: "BTCUSDT".to_owned(),
            side: shared_types::OrderSide::Buy,
            order_type: shared_types::OrderType::Limit,
            quantity: 1.0,
            price: Some(10.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: "client-1".to_owned(),
            client_order_id_policy: None,
            created_at_ms: 900,
        },
        state,
        risk: None,
        identity: shared_types::VenueOrderIdentity {
            internal_order_id: "internal-1".into(),
            public_client_order_id: "client-1".into(),
            account_scope: if mode == ExecutionMode::Live {
                credential_fingerprint_v1("binance", shared_types::FeeProduct::Perp)
            } else {
                Some("paper-account".into())
            },
            product: shared_types::FeeProduct::Perp,
            ..Default::default()
        },
        last_update_source: source,
        exchange_order_id: Some("exchange-1".to_owned()),
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms,
    }
}

fn venue_keys(rows: Vec<LiveOrderProofRuntimeHealth>) -> Vec<String> {
    let mut venues = rows
        .into_iter()
        .map(|row| normalized_venue_name(&row.venue))
        .collect::<Vec<_>>();
    venues.sort();
    venues
}
