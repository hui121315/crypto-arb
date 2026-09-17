#[test]
fn history_storage_row_blocks_unapplied_postgres_migration() {
    let mut health = realtime::HistoryStore::new(10).health_snapshot(10);
    health.backend = "postgres";
    health.durable = true;
    health.ephemeral = false;
    health.migration_status = Some(HistoryMigrationStatus {
        migration_id: "20260701_history".to_owned(),
        schema_name: "realtime_history".to_owned(),
        migration_path: "crates/realtime/migrations/20260701_history.sql".to_owned(),
        schema_version: None,
        migration_checksum: health.migration_checksum.clone(),
        applied: false,
        applied_at_ms: None,
    });
    health.storage_contract.backend_kind = shared_types::StorageBackendKind::Postgres;
    health.storage_contract.migration_authority = health.migration_status.clone();
    health.storage_contract.degraded_reasons = vec![StorageDegradedReason::MigrationUnapplied];

    let row = history_storage_row(&health, 10);

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HISTORY_SCHEMA_DRIFT)
    );
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "migration_applied=false")
    }));
}

#[test]
fn history_storage_row_preserves_applied_migration_evidence() {
    let mut health = realtime::HistoryStore::new(10).health_snapshot(10);
    health.backend = "postgres";
    health.durable = true;
    health.ephemeral = false;
    mark_history_migration_applied(&mut health);

    let row = history_storage_row(&health, 20);

    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "migration_applied=true")
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .doc_urls
            .iter()
            .any(|url| url.contains("sql-insert"))
    }));
}

#[test]
fn history_storage_row_preserves_backpressure_problem_codes() {
    for code in [
        codes::HISTORY_STORE_RATE_LIMITED,
        codes::HISTORY_QUERY_CANCELED,
    ] {
        let mut health = realtime::HistoryStore::new(10).health_snapshot(10);
        health.backend = "postgres";
        health.durable = true;
        health.ephemeral = false;
        mark_history_migration_applied(&mut health);
        health.query_error_total = 1;
        health.last_error_at_ms = Some(9_000);
        health.last_error = Some("history postgres backpressure".into());
        health.last_error_code = Some(code.into());
        health
            .storage_contract
            .degraded_reasons
            .push(StorageDegradedReason::Backpressure);

        let row = history_storage_row(&health, 10_000);

        assert_eq!(
            row.problem.as_ref().map(|problem| problem.code.as_str()),
            Some(code)
        );
    }
}

#[test]
fn history_storage_row_marks_plain_memory_non_durable_without_problem() {
    let health = realtime::HistoryStore::new(10).health_snapshot(10);

    let row = history_storage_row(&health, 10);

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert!(row.message.contains("非持久化"));
    assert!(row.problem.is_none());
}

#[test]
fn execution_ledger_storage_row_warns_when_unconfigured() {
    let row = execution_ledger_storage_row(
        &ExecutionLedgerStorageSnapshot {
            configured: false,
            path: None,
            event_count: 0,
            replayed_events: 0,
            replay_failures: 0,
            append_successes: 0,
            append_failures: 0,
            last_append_at_ms: None,
            query_successes: 0,
            query_failures: 0,
            last_query_at_ms: None,
        },
        10_000,
    );

    assert_eq!(row.venue, SYSTEM_VENUE);
    assert_eq!(row.operation, OP_EXECUTION_LEDGER_STORAGE);
    assert_eq!(row.source, SOURCE_EXECUTION_LEDGER_STORAGE);
    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.configured, Some(false));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::EXECUTION_LEDGER_STORAGE_UNAVAILABLE)
    );
}

#[test]
fn execution_ledger_storage_row_reports_configured_jsonl() {
    let row = execution_ledger_storage_row(
        &ExecutionLedgerStorageSnapshot {
            configured: true,
            path: Some("/tmp/crossline/execution_ledger.jsonl".to_owned()),
            event_count: 5,
            replayed_events: 3,
            replay_failures: 0,
            append_successes: 2,
            append_failures: 0,
            last_append_at_ms: Some(9_500),
            query_successes: 4,
            query_failures: 0,
            last_query_at_ms: Some(9_800),
        },
        10_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.configured, Some(true));
    assert_eq!(row.requested, Some(9));
    assert_eq!(row.rows, Some(5));
    assert_eq!(row.freshness_ms, Some(200));
    assert!(row.problem.is_none());
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "event_count=5")
    }));
}

#[test]
fn execution_ledger_storage_row_blocks_on_io_errors() {
    let row = execution_ledger_storage_row(
        &ExecutionLedgerStorageSnapshot {
            configured: true,
            path: Some("/tmp/crossline/execution_ledger.jsonl".to_owned()),
            event_count: 1,
            replayed_events: 0,
            replay_failures: 1,
            append_successes: 0,
            append_failures: 2,
            last_append_at_ms: None,
            query_successes: 0,
            query_failures: 0,
            last_query_at_ms: None,
        },
        10_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::EXECUTION_LEDGER_STORAGE_IO_FAILED)
    );
    assert!(
        row.problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("appendFailures"))
            .is_some_and(|failures| failures.as_u64() == Some(2))
    );
}

#[test]
fn execution_ledger_storage_row_warns_on_query_failures() {
    let row = execution_ledger_storage_row(
        &ExecutionLedgerStorageSnapshot {
            configured: true,
            path: Some("/tmp/crossline/execution_ledger.jsonl".to_owned()),
            event_count: 1,
            replayed_events: 1,
            replay_failures: 0,
            append_successes: 1,
            append_failures: 0,
            last_append_at_ms: Some(9_500),
            query_successes: 2,
            query_failures: 1,
            last_query_at_ms: Some(9_900),
        },
        10_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.freshness_ms, Some(100));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::EXECUTION_LEDGER_QUERY_FAILED)
    );
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "query_failures=1")
    }));
}

#[test]
fn order_snapshot_storage_row_warns_when_unconfigured() {
    let row = order_snapshot_storage_row(
        &OrderSnapshotStorageSnapshot {
            configured: false,
            path: None,
            record_count: 0,
            replayed_records: 0,
            replay_failures: 0,
            append_successes: 0,
            append_failures: 0,
            last_append_at_ms: None,
        },
        10_000,
    );

    assert_eq!(row.venue, SYSTEM_VENUE);
    assert_eq!(row.operation, OP_ORDER_SNAPSHOT_STORAGE);
    assert_eq!(row.source, SOURCE_ORDER_SNAPSHOT_STORAGE);
    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.configured, Some(false));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::ORDER_SNAPSHOT_STORAGE_UNAVAILABLE)
    );
}
