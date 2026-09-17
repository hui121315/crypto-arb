#[test]
fn snapshot_replay_does_not_overwrite_newer_record() {
    let older = replay_order_record("o1", "c1", LiveOrderState::Accepted, 10);
    let newer = replay_order_record("o1", "c1", LiveOrderState::Filled, 20);
    let replay = SqlLedgerReplay {
        events: Vec::new(),
        order_snapshots: vec![newer.clone(), older],
        balance_events: Vec::new(),
        run_finality_events: Vec::new(),
        health: SqlLedgerReplayHealth::default(),
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

    assert_eq!(journal.get("o1"), Some(newer));
    assert_eq!(journal.open_order_count(), 0);
}

#[test]
fn snapshot_replay_prefers_terminal_state_over_later_ack_timestamp() {
    let accepted = replay_order_record("o1", "c1", LiveOrderState::Accepted, 20);
    let filled = replay_order_record("o1", "c1", LiveOrderState::Filled, 10);
    let replay = SqlLedgerReplay {
        events: Vec::new(),
        order_snapshots: vec![accepted, filled.clone()],
        balance_events: Vec::new(),
        run_finality_events: Vec::new(),
        health: SqlLedgerReplayHealth::default(),
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

    assert_eq!(journal.get("o1"), Some(filled));
    assert_eq!(journal.open_order_count(), 0);
}

#[test]
fn snapshot_replay_never_regresses_terminal_state_to_later_ack() {
    let filled = replay_order_record("o1", "c1", LiveOrderState::Filled, 10);
    let accepted = replay_order_record("o1", "c1", LiveOrderState::Accepted, 20);
    let replay = SqlLedgerReplay {
        events: Vec::new(),
        order_snapshots: vec![filled.clone(), accepted],
        balance_events: Vec::new(),
        run_finality_events: Vec::new(),
        health: SqlLedgerReplayHealth::default(),
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

    assert_eq!(journal.get("o1"), Some(filled));
    assert_eq!(journal.open_order_count(), 0);
}

#[test]
fn order_info_reindexes_changed_exchange_order_id() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    submit_intent(&journal, &intent);
    journal
        .apply_ack(&OrderAck {
            internal_order_id: intent.id.clone(),
            exchange_order_id: Some("x-old".into()),
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

    let info = OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: None,
        reduce_only: None,
        order_id: "x-new".into(),
        ..order_info(OrderStatus::Filled)
    };
    journal
        .apply_order_info(&intent.id, &info, 3)
        .expect("order status backfill");

    assert!(journal.get_by_exchange_order_id("x-old").is_none());
    assert_eq!(
        journal
            .get_by_exchange_order_id("x-new")
            .expect("new exchange id indexed")
            .intent
            .id,
        "o1"
    );
}
