#[test]
fn late_adapter_ack_merges_identity_after_private_ws_fill() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("late-ack", "late-client");
    submit_intent(&journal, &intent);

    let mut fill = order_info(OrderStatus::Filled);
    fill.order_id = "230091157354".to_owned();
    fill.client_order_id = Some(intent.client_order_id.clone());
    let filled = journal
        .apply_order_info_from_source(&intent.id, &fill, 4, OrderUpdateSource::PrivateWs)
        .expect("private stream fill applies before ACK");
    assert_eq!(filled.state, LiveOrderState::Filled);

    let merged = journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("230091157354".to_owned()),
            client_order_id: intent.client_order_id.clone(),
            identity_update: VenueOrderIdentityUpdate::from_ids(
                intent.client_order_id.clone(),
                intent.client_order_id,
                Some("230091157354".to_owned()),
            ),
            state: LiveOrderState::Accepted,
            accepted_at_ms: 5,
            message: None,
            filled_quantity: None,
            filled_price: None,
            filled_fee: None,
        })
        .expect("late ACK merges without reporting order not found");

    assert_eq!(merged.state, LiveOrderState::Filled);
    assert_eq!(merged.filled_quantity, Some(0.4));
    assert_eq!(merged.filled_price, Some(10.5));
    assert_eq!(merged.exchange_order_id.as_deref(), Some("230091157354"));
    assert_eq!(journal.open_order_count(), 0);
}

#[test]
fn sql_ledger_replay_restores_funding_and_slippage_facts() {
    let record = replay_order_record("hedge-1-long", "c1", LiveOrderState::Filled, 10);
    let mut funding = replay_execution_event(&record, 11);
    funding.event_id = "funding:hedge-1-long:11".to_owned();
    funding.event_type = shared_types::ExecutionLedgerEventType::FundingPayment;
    funding.payload =
        ExecutionLedgerPayload::FundingPayment(shared_types::FundingPaymentLedgerRecord {
            amount: -0.12,
            currency: "USDT".to_owned(),
            funding_time_ms: 11,
            quality: shared_types::ExecutionLedgerQuality::Actual,
        });
    let mut slippage = replay_execution_event(&record, 12);
    slippage.event_id = "slippage:hedge-1-long:12".to_owned();
    slippage.event_type = shared_types::ExecutionLedgerEventType::Slippage;
    slippage.payload = ExecutionLedgerPayload::Slippage(shared_types::SlippageLedgerRecord {
        amount_usd: 0.4,
        reference_price: 100.0,
        fill_price: 100.4,
        quantity: 1.0,
        quality: shared_types::ExecutionLedgerQuality::Actual,
    });
    let replay = SqlLedgerReplay {
        events: vec![funding.clone(), slippage.clone()],
        order_snapshots: vec![record],
        balance_events: Vec::new(),
        run_finality_events: Vec::new(),
        health: SqlLedgerReplayHealth {
            query_successes: 2,
            event_rows: 2,
            replayed_events: 2,
            snapshot_rows: 1,
            replayed_order_snapshots: 1,
            replay_limit: 50_000,
            last_query_at_ms: Some(13),
            ..SqlLedgerReplayHealth::default()
        },
    };
    let journal = OrderJournal::new_with_storage_paths_and_sql(
        None,
        None,
        SqlLedgerInit {
            migration_health: sql_migration_health(),
            replay,
            store: None,
        },
    );

    let events = journal.ledger_events();
    assert!(events.iter().any(|event| event == &funding));
    assert!(events.iter().any(|event| event == &slippage));
    let storage = journal.sql_ledger_storage_snapshot();
    assert_eq!(storage.replayed_events, 2);
    assert_eq!(storage.replayed_order_snapshots, 1);
    assert_eq!(storage.replay_query_successes, 2);
}
