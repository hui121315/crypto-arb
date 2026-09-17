#[test]
fn order_snapshot_jsonl_replays_order_records_after_restart() {
    let path = temp_ledger_path("order-snapshot-replay");
    {
        let journal = OrderJournal::new_with_storage_paths(None, Some(path.clone()));
        let intent = intent("o1", "c1");
        submit_intent(&journal, &intent);
        journal
            .apply_ack(&OrderAck {
                internal_order_id: intent.id.clone(),
                exchange_order_id: Some("x1".into()),
                client_order_id: intent.client_order_id,
                identity_update: Default::default(),
                state: LiveOrderState::Accepted,
                accepted_at_ms: 4,
                message: None,
                filled_quantity: None,
                filled_price: None,
                filled_fee: None,
            })
            .expect("accepted");
        assert_execution_ledger_unconfigured(&journal);
        assert_order_snapshot_writer_health(&journal);
    }

    let restored = OrderJournal::new_with_storage_paths(None, Some(path.clone()));

    assert_restored_accepted_order_snapshot(&restored);
    let _ = std::fs::remove_file(path);
}

#[test]
fn order_snapshot_replay_keeps_latest_record_for_same_order() {
    let path = temp_ledger_path("order-snapshot-terminal");
    {
        let journal = OrderJournal::new_with_storage_paths(None, Some(path.clone()));
        let intent = intent("o1", "c1");
        submit_intent(&journal, &intent);
        journal
            .apply_ack(&OrderAck {
                internal_order_id: intent.id,
                exchange_order_id: Some("x1".into()),
                client_order_id: intent.client_order_id,
                identity_update: Default::default(),
                state: LiveOrderState::Filled,
                accepted_at_ms: 4,
                message: None,
                filled_quantity: Some(0.6),
                filled_price: Some(10.25),
                filled_fee: Some(0.03),
            })
            .expect("filled");
    }

    let restored = OrderJournal::new_with_storage_paths(None, Some(path.clone()));

    assert_eq!(restored.list().len(), 1);
    assert_eq!(restored.open_order_count(), 0);
    assert_eq!(
        restored.get("o1").map(|order| order.state),
        Some(LiveOrderState::Filled)
    );
    let _ = std::fs::remove_file(path);
}

#[test]
fn order_snapshot_replay_failures_do_not_pollute_ledger_health() {
    let path = temp_ledger_path("order-snapshot-bad-line");
    std::fs::write(&path, "{not-json}\n").expect("write bad snapshot");

    let journal = OrderJournal::new_with_storage_paths(None, Some(path.clone()));
    let order_snapshot = journal.order_snapshot_storage_snapshot();
    let execution_ledger = journal.execution_ledger_storage_snapshot();

    assert_eq!(order_snapshot.replay_failures, 1);
    assert_eq!(order_snapshot.record_count, 0);
    assert_eq!(execution_ledger.replay_failures, 0);
    let _ = std::fs::remove_file(path);
}

#[test]
fn order_snapshot_jsonl_serializes_concurrent_appends() -> Result<(), String> {
    let path = temp_ledger_path("order-snapshot-concurrent");
    let journal = Arc::new(OrderJournal::new_with_storage_paths(
        None,
        Some(path.clone()),
    ));
    std::thread::scope(|scope| {
        for index in 0..32 {
            let journal = Arc::clone(&journal);
            scope.spawn(move || {
                let id = format!("o{index}");
                let client_order_id = format!("c{index}");
                journal.insert_created(intent(&id, &client_order_id), index);
            });
        }
    });

    let replay = order_store::read_jsonl(&path)
        .map_err(|error| format!("order snapshot replay failed: {error}"))?;
    let restored = OrderJournal::new_with_storage_paths(None, Some(path.clone()));
    let snapshot = journal.order_snapshot_storage_snapshot();

    assert_eq!(replay.failed_lines, 0);
    assert_eq!(snapshot.record_count, 32);
    assert_eq!(snapshot.append_successes, 32);
    assert_eq!(snapshot.append_failures, 0);
    assert_eq!(restored.list().len(), 32);
    let _ = std::fs::remove_file(path);
    Ok(())
}

#[test]
fn execution_ledger_storage_snapshot_tracks_append_and_replay() {
    let path = temp_ledger_path("storage-health");
    let event_count = {
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

        let snapshot = journal.execution_ledger_storage_snapshot();
        assert!(snapshot.configured);
        assert_eq!(snapshot.append_failures, 0);
        assert!(snapshot.append_successes >= 3);
        assert!(snapshot.last_append_at_ms.is_some());
        assert_eq!(snapshot.query_successes, 0);
        assert_eq!(snapshot.query_failures, 0);
        snapshot.event_count
    };

    let restored = OrderJournal::new_with_ledger_path(path.clone());
    let snapshot = restored.execution_ledger_storage_snapshot();

    assert_eq!(snapshot.replay_failures, 0);
    assert_eq!(snapshot.replayed_events, event_count);
    assert_eq!(snapshot.event_count, event_count);
    let _ = std::fs::remove_file(path);
}

#[test]
fn ledger_query_records_query_health() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("hedge-1-long", "c1");
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
            filled_quantity: Some(0.6),
            filled_price: Some(10.25),
            filled_fee: Some(0.03),
        })
        .expect("ack filled");

    let rows = journal.ledger_events_by_query(&ExecutionLedgerQuery {
        internal_order_id: Some("hedge-1-long".to_owned()),
        limit: 10,
        ..ExecutionLedgerQuery::default()
    });
    let snapshot = journal.execution_ledger_storage_snapshot();

    assert!(!rows.is_empty());
    assert_eq!(snapshot.query_successes, 1);
    assert_eq!(snapshot.query_failures, 0);
    assert!(snapshot.last_query_at_ms.is_some());
}

#[test]
fn invalid_ledger_query_records_failure() {
    let journal = OrderJournal::default_without_audit();

    let rows = journal.ledger_events_by_query(&ExecutionLedgerQuery {
        limit: 0,
        ..ExecutionLedgerQuery::default()
    });
    let snapshot = journal.execution_ledger_storage_snapshot();

    assert!(rows.is_empty());
    assert_eq!(snapshot.query_successes, 0);
    assert_eq!(snapshot.query_failures, 1);
    assert!(snapshot.last_query_at_ms.is_some());
}

#[test]
fn sql_ledger_storage_snapshot_reports_unconfigured_writer() {
    let journal = OrderJournal::default_without_audit();
    let snapshot = journal.sql_ledger_storage_snapshot();

    assert!(!snapshot.migration.configured);
    assert!(!snapshot.writer_configured);
    assert_eq!(snapshot.event_append_successes, 0);
    assert_eq!(snapshot.snapshot_append_successes, 0);
}

#[test]
fn sql_ledger_replay_seeds_memory_projection() {
    let record = replay_order_record("o1", "c1", LiveOrderState::Accepted, 10);
    let event = replay_execution_event(&record, 11);
    let replay = SqlLedgerReplay {
        events: vec![event.clone()],
        order_snapshots: vec![record.clone()],
        balance_events: Vec::new(),
        run_finality_events: Vec::new(),
        health: SqlLedgerReplayHealth {
            query_successes: 2,
            event_rows: 1,
            replayed_events: 1,
            snapshot_rows: 1,
            replayed_order_snapshots: 1,
            replay_limit: 50_000,
            last_query_at_ms: Some(12),
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

    assert_eq!(journal.get("o1"), Some(record));
    assert!(journal
        .ledger_events()
        .iter()
        .any(|row| row.event_id == event.event_id));
    let storage = journal.sql_ledger_storage_snapshot();
    assert_eq!(storage.replayed_events, 1);
    assert_eq!(storage.replayed_order_snapshots, 1);
    assert_eq!(storage.replay_query_successes, 2);
}
