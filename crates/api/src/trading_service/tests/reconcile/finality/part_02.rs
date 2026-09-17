#[tokio::test]
async fn reconcile_keeps_quantity_mismatch_observe_only() {
    let remote = order_info("x1", OrderStatus::Open, 0.02);
    let service = service_with_reconcile_adapter(
        vec![remote],
        Some(order_info("x1", OrderStatus::Filled, 0.02)),
    );
    seed_accepted_order(&service, "quantity-mismatch", "x1", 0.01);

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "quantity mismatch reconcile",
    );

    assert_eq!(outcome.diffs.len(), 1);
    assert_eq!(
        outcome.diffs[0].kind,
        trading::ReconcileDiffKind::QuantityMismatch
    );
    assert!(outcome.refreshed.is_empty());
    let local = must_some(
        service.get_order("quantity-mismatch"),
        "quantity mismatch must not mutate local order",
    );
    assert_eq!(local.state, LiveOrderState::Accepted);
    assert_eq!(local.intent.quantity, 0.01);
}

fn cancel_guard_record(mode: ExecutionMode, state: LiveOrderState) -> OrderRecord {
    let mut intent = limit_intent("cancel-guard");
    intent.mode = mode;
    OrderRecord {
        intent,
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: Some("x-guard".to_owned()),
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms: 10,
    }
}

fn audit_health_snapshot(
    configured: bool,
    opened: bool,
    last_error: Option<&str>,
) -> crate::middleware::audit::AuditSinkHealthSnapshot {
    crate::middleware::audit::AuditSinkHealthSnapshot {
        initialized: true,
        configured,
        opened,
        path: configured.then(|| "/tmp/audit.jsonl".to_owned()),
        queue_capacity: if configured && opened { 1024 } else { 0 },
        pending_writes: 0,
        writer_alive: configured && opened,
        write_attempts: 0,
        write_successes: 0,
        write_failures: 0,
        last_write_at_ms: None,
        last_error_at_ms: last_error.map(|_| 1),
        last_error: last_error.map(ToOwned::to_owned),
        observed_at_ms: 10,
    }
}

#[tokio::test]
async fn reconcile_keeps_local_missing_observe_only() {
    let service =
        service_with_reconcile_adapter(vec![order_info("x1", OrderStatus::Open, 0.01)], None);

    let outcome = must_ok(
        service.reconcile_and_refresh_missing_orders().await,
        "local missing reconcile",
    );

    assert_eq!(outcome.diffs.len(), 1);
    assert_eq!(
        outcome.diffs[0].kind,
        trading::ReconcileDiffKind::LocalMissing
    );
    assert!(outcome.refreshed.is_empty());
}

#[tokio::test]
async fn mock_dry_run_submit_fills_without_open_order_debt() {
    let service = TradingService::new_mock();
    let mut intent = limit_intent("dry-run-fill");
    intent.mode = ExecutionMode::DryRun;

    let record = must_ok(service.submit(intent).await, "dry-run submit");

    assert_eq!(record.state, shared_types::LiveOrderState::Filled);
    assert_eq!(service.open_order_count(), 0);
}

#[test]
fn execution_ledger_query_facade_records_query_health() {
    let service = TradingService::new_mock();
    seed_filled_order(&service, "ledger-query-fill", "x-ledger-query");

    let events = service.list_execution_ledger_events_by_query(&trading::ExecutionLedgerQuery {
        internal_order_id: Some("ledger-query-fill".to_owned()),
        limit: 16,
        ..trading::ExecutionLedgerQuery::default()
    });

    assert!(
        events.iter().any(
            |event| event.order.identity.exchange_order_id.as_deref() == Some("x-ledger-query")
        )
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == shared_types::ExecutionLedgerEventType::FillSnapshot)
    );
    let snapshot = service.execution_ledger_storage_snapshot();
    assert_eq!(snapshot.query_successes, 1);
    assert_eq!(snapshot.query_failures, 0);
}
