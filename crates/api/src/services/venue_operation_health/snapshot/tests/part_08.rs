#[test]
fn order_snapshot_storage_row_reports_configured_jsonl() {
    let row = order_snapshot_storage_row(
        &OrderSnapshotStorageSnapshot {
            configured: true,
            path: Some("/tmp/crossline/order_snapshots.jsonl".to_owned()),
            record_count: 3,
            replayed_records: 2,
            replay_failures: 0,
            append_successes: 4,
            append_failures: 0,
            last_append_at_ms: Some(9_500),
        },
        10_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.configured, Some(true));
    assert_eq!(row.requested, Some(6));
    assert_eq!(row.rows, Some(3));
    assert_eq!(row.freshness_ms, Some(500));
    assert!(row.problem.is_none());
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "record_count=3")
    }));
}

#[test]
fn order_snapshot_storage_row_blocks_on_io_errors() {
    let row = order_snapshot_storage_row(
        &OrderSnapshotStorageSnapshot {
            configured: true,
            path: Some("/tmp/crossline/order_snapshots.jsonl".to_owned()),
            record_count: 1,
            replayed_records: 0,
            replay_failures: 1,
            append_successes: 0,
            append_failures: 2,
            last_append_at_ms: None,
        },
        10_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::ORDER_SNAPSHOT_STORAGE_IO_FAILED)
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
fn trading_sql_migration_storage_row_warns_when_unconfigured() {
    let row = trading_sql_migration_storage_row(&sql_migration_health(false, false, None), 10);

    assert_eq!(row.venue, SYSTEM_VENUE);
    assert_eq!(row.operation, OP_TRADING_SQL_MIGRATIONS_STORAGE);
    assert_eq!(row.source, SOURCE_TRADING_SQL_MIGRATION);
    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.configured, Some(false));
    assert_eq!(row.rows, Some(0));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::TRADING_SQL_MIGRATION_UNAVAILABLE)
    );
}

#[test]
fn trading_sql_migration_storage_row_reports_applied_schema() {
    let row = trading_sql_migration_storage_row(&sql_migration_health(true, true, None), 10);
    let schema_version = format!(
        "schema_version={}",
        trading::SQL_LEDGER_MIGRATION_VERSION
    );

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(1));
    assert!(row.problem.is_none());
    assert!(
        row.evidence
            .as_ref()
            .is_some_and(|evidence| evidence.schema_hash.starts_with("fnv1a64:"))
    );
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == &schema_version)
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "schema_name=trading_order_ledger")
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .doc_urls
            .iter()
            .any(|url| url.contains("sql-createtable"))
    }));
}

#[test]
fn trading_sql_migration_storage_row_uses_typed_schema_drift_reason() {
    let mut health = sql_migration_health(true, false, Some("immutable migration mismatch"));
    health.degraded_reason = Some(StorageDegradedReason::SchemaDrift);

    let row = trading_sql_migration_storage_row(&health, 10);

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::TRADING_SQL_SCHEMA_DRIFT)
    );
    assert!(row
        .problem
        .as_ref()
        .and_then(|problem| problem.details.as_ref())
        .and_then(|details| details.pointer("/storageContract/degradedReasons"))
        .is_some_and(|reasons| reasons.as_array().is_some_and(|reasons| {
            reasons.iter().any(|reason| reason == "schema_drift")
        })));
}

#[test]
fn trading_sql_migration_storage_row_blocks_failed_runner() {
    let row = trading_sql_migration_storage_row(
        &sql_migration_health(true, false, Some("schema migration failed")),
        10,
    );

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.rows, Some(0));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::TRADING_SQL_MIGRATION_FAILED)
    );
    assert!(
        row.error
            .as_deref()
            .is_some_and(|error| error.contains("失败"))
    );
}

#[test]
fn trading_sql_ledger_storage_row_warns_when_unconfigured() {
    let row = trading_sql_ledger_storage_row(
        &sql_ledger_snapshot(sql_migration_health(false, false, None), false),
        10_000,
    );

    assert_eq!(row.venue, SYSTEM_VENUE);
    assert_eq!(row.operation, OP_TRADING_SQL_LEDGER_STORAGE);
    assert_eq!(row.source, SOURCE_TRADING_SQL_LEDGER_WRITER);
    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.configured, Some(false));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::TRADING_SQL_LEDGER_UNAVAILABLE)
    );
}

#[test]
fn trading_sql_ledger_storage_row_reports_writer_append_health() {
    let mut snapshot = sql_ledger_snapshot(sql_migration_health(true, true, None), true);
    snapshot.event_append_successes = 2;
    snapshot.snapshot_append_successes = 3;
    snapshot.last_append_at_ms = Some(9_500);

    let row = trading_sql_ledger_storage_row(&snapshot, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(5));
    assert_eq!(row.requested, Some(5));
    assert_eq!(row.freshness_ms, Some(500));
    assert!(row.problem.is_none());
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "writer_configured=true")
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .doc_urls
            .iter()
            .any(|url| url.contains("sql-insert"))
    }));
}

#[test]
fn trading_sql_ledger_storage_row_reports_replay_query_health() {
    let mut snapshot = sql_ledger_snapshot(sql_migration_health(true, true, None), true);
    snapshot.replay_query_successes = 2;
    snapshot.replay_event_rows = 4;
    snapshot.replayed_events = 4;
    snapshot.replay_snapshot_rows = 1;
    snapshot.replayed_order_snapshots = 1;
    snapshot.replay_limit = 50_000;
    snapshot.last_replay_at_ms = Some(9_600);

    let row = trading_sql_ledger_storage_row(&snapshot, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.rows, Some(5));
    assert_eq!(row.requested, Some(7));
    assert_eq!(row.freshness_ms, Some(400));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "replayed_events=4")
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .doc_urls
            .iter()
            .any(|url| url.contains("sql-select"))
    }));
}

#[test]
fn trading_sql_ledger_storage_row_blocks_replay_decode_failures() {
    let mut snapshot = sql_ledger_snapshot(sql_migration_health(true, true, None), true);
    snapshot.replay_query_successes = 2;
    snapshot.replay_event_failures = 1;
    snapshot.last_replay_error_at_ms = Some(9_800);
    snapshot.last_replay_error = Some("order_events payload decode failed".to_owned());

    let row = trading_sql_ledger_storage_row(&snapshot, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::TRADING_SQL_LEDGER_WRITE_FAILED)
    );
    assert!(
        row.problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("replayEventFailures"))
            .is_some_and(|failures| failures.as_u64() == Some(1))
    );
}

#[test]
fn trading_sql_ledger_storage_row_blocks_write_failures_and_dropped_writes() {
    let mut snapshot = sql_ledger_snapshot(sql_migration_health(true, true, None), true);
    snapshot.event_append_failures = 1;
    snapshot.dropped_writes = 2;
    snapshot.last_error_at_ms = Some(9_800);
    snapshot.last_error = Some("sql writer queue failed".to_owned());

    let row = trading_sql_ledger_storage_row(&snapshot, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(row.requested, Some(3));
    assert_eq!(row.freshness_ms, Some(200));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::TRADING_SQL_LEDGER_WRITE_FAILED)
    );
    assert!(
        row.problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("droppedWrites"))
            .is_some_and(|dropped| dropped.as_u64() == Some(2))
    );
}
