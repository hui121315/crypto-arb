#[test]
fn apply_order_info_by_client_order_id_reuses_client_index() {
    let journal = OrderJournal::default_without_audit();
    let intent = intent("o1", "c1");
    journal.insert_created(intent.clone(), 1);
    journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(10.0), 2)
        .expect("risk checked");
    journal.mark_submitted(&intent.id, 3).expect("submitted");

    let updated = journal
        .apply_order_info_by_client_order_id("c1", &order_info(OrderStatus::Filled), 5)
        .expect("client id indexed");

    assert_eq!(updated.intent.id, "o1");
    assert_eq!(updated.state, LiveOrderState::Filled);
    assert_eq!(journal.open_order_count(), 0);
    assert!(journal
        .apply_order_info_by_client_order_id("missing", &order_info(OrderStatus::Filled), 6)
        .is_none());
}

#[test]
fn claim_created_grants_single_submission_right_per_client_order_id() {
    let journal = OrderJournal::default_without_audit();
    let first = journal.claim_created(intent("i1", "client-1"), 1);
    assert!(matches!(first, CreatedClaim::New(_)));

    let replay = journal.claim_created(intent("i2", "client-1"), 2);
    match replay {
        CreatedClaim::Existing(existing) => assert_eq!(existing.intent.id, "i1"),
        other => panic!("expected existing record, got {other:?}"),
    }
    assert!(journal.get("i2").is_none());
    assert!(matches!(
        journal.claim_created(intent("i3", "client-2"), 3),
        CreatedClaim::New(_)
    ));
}

fn intent(id: &str, client_order_id: &str) -> OrderIntent {
    OrderIntent {
        id: id.into(),
        source: OrderSource::Manual,
        strategy: None,
        mode: ExecutionMode::DryRun,
        exchange: "mock".into(),
        symbol: "BTC".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        quantity: 1.0,
        price: Some(10.0),
        slippage_tolerance_bps: None,
        reduce_only: false,
        time_in_force: shared_types::TimeInForce::Ioc,
        post_only: false,
        margin_mode: shared_types::MarginMode::Cross,
        leverage: 1.0,
        client_order_id: client_order_id.into(),
        client_order_id_policy: None,
        created_at_ms: 1,
    }
}

fn arbitrage_intent(id: &str, client_order_id: &str, exchange: &str, symbol: &str) -> OrderIntent {
    let mut intent = intent(id, client_order_id);
    intent.source = OrderSource::ArbitragePreview;
    intent.strategy = Some(shared_types::StrategyKind::PerpCross);
    intent.exchange = exchange.to_owned();
    intent.symbol = symbol.to_owned();
    intent
}

fn submit_intent(journal: &OrderJournal, intent: &OrderIntent) {
    journal.insert_created(intent.clone(), 1);
    journal
        .mark_risk_checked(&intent.id, RiskDecision::allow(10.0), 2)
        .expect("risk checked");
    journal.mark_submitted(&intent.id, 3).expect("submitted");
}

fn order_info(status: OrderStatus) -> OrderInfo {
    OrderInfo {
        execution_style: None,
        venue_time_in_force: None,
        client_order_id: None,
        reduce_only: None,
        order_id: "x1".into(),
        symbol: "BTC".into(),
        exchange: "mock".into(),
        side: OrderSide::Buy,
        order_type: OrderType::Limit,
        status,
        quantity: 1.0,
        price: 10.0,
        filled_quantity: 0.4,
        filled_price: 10.5,
        fees: 0.08,
        created_at: chrono::Utc::now(),
    }
}

fn replay_order_record(
    id: &str,
    client_order_id: &str,
    state: LiveOrderState,
    updated_at_ms: i64,
) -> OrderRecord {
    let intent = intent(id, client_order_id);
    OrderRecord {
        identity: VenueOrderIdentity::from_intent(&intent),
        intent,
        state,
        risk: None,
        last_update_source: OrderUpdateSource::PrivateWs,
        exchange_order_id: Some("x1".to_owned()),
        message: None,
        filled_quantity: matches!(state, LiveOrderState::Filled).then_some(1.0),
        filled_price: matches!(state, LiveOrderState::Filled).then_some(10.0),
        filled_fee: None,
        updated_at_ms,
    }
}

fn replay_execution_event(record: &OrderRecord, at_ms: i64) -> ExecutionLedgerEvent {
    ExecutionLedgerEvent {
        event_id: format!("replay-state:{}", record.intent.id),
        event_type: shared_types::ExecutionLedgerEventType::OrderState,
        source: OrderUpdateSource::PrivateWs,
        order: shared_types::ExecutionLedgerOrderRef {
            run_id: None,
            ticket_id: None,
            leg_role: None,
            reduce_only: None,
            exchange: record.intent.exchange.clone(),
            symbol: record.intent.symbol.clone(),
            side: record.intent.side,
            identity: record.identity_snapshot(),
        },
        payload: ExecutionLedgerPayload::OrderState {
            state: record.state,
            message: None,
        },
        occurred_at_ms: at_ms,
        captured_at_ms: at_ms,
    }
}

fn sql_migration_health() -> crate::sql_ledger::SqlLedgerMigrationHealth {
    crate::sql_ledger::SqlLedgerMigrationHealth {
        configured: true,
        migration_id: crate::sql_ledger::SQL_LEDGER_MIGRATION_ID,
        migration_path: crate::sql_ledger::SQL_LEDGER_MIGRATION_PATH,
        migration_checksum: crate::sql_ledger::sql_ledger_schema_hash(),
        schema_version: Some(crate::sql_ledger::SQL_LEDGER_MIGRATION_VERSION),
        applied: true,
        degraded_reason: None,
        last_success_at_ms: Some(9),
        last_error_at_ms: None,
        last_error: None,
        observed_at_ms: 9,
    }
}

fn temp_ledger_path(label: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system time")
        .as_nanos();
    let mut path = std::env::temp_dir();
    path.push(format!(
        "crossline-{label}-{}-{nanos}.jsonl",
        std::process::id()
    ));
    path
}

fn assert_execution_ledger_unconfigured(journal: &OrderJournal) {
    let storage = journal.execution_ledger_storage_snapshot();
    assert!(!storage.configured);
    assert_eq!(storage.replay_failures, 0);
    assert_eq!(storage.append_successes, 0);
    assert_eq!(storage.append_failures, 0);
}

fn assert_order_snapshot_writer_health(journal: &OrderJournal) {
    let snapshot = journal.order_snapshot_storage_snapshot();
    assert!(snapshot.configured);
    assert_eq!(snapshot.record_count, 1);
    assert!(snapshot.append_successes >= 1);
    assert_eq!(snapshot.append_failures, 0);
    assert!(snapshot.last_append_at_ms.is_some());
}

fn assert_restored_accepted_order_snapshot(journal: &OrderJournal) {
    assert_eq!(journal.list().len(), 1);
    assert_eq!(
        journal
            .get_by_client_order_id("c1")
            .map(|order| order.intent.id),
        Some("o1".to_owned())
    );
    assert_eq!(
        journal
            .get_by_exchange_order_id("x1")
            .map(|order| order.state),
        Some(LiveOrderState::Accepted)
    );
    assert_eq!(journal.open_order_count(), 1);
    assert_restored_order_snapshot_health(journal);
}
