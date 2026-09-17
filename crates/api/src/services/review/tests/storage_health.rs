use super::super::storage_health::{execution_ledger_storage_health, sql_ledger_storage_health};
use super::super::*;

#[test]
fn unconfigured_memory_storage_warns_without_turning_available_data_into_a_failure() {
    let envelope = ReviewEnvelope::new(
        Vec::<u8>::new(),
        1_000,
        30,
        ReviewDataSource::ExecutionLedger,
        Some(ReviewLedgerStatus::NoLedgerEvents),
        Vec::new(),
    );
    let health = review_storage_health(ReviewDataSource::ExecutionLedger, 0, 1_000);

    let decorated = with_review_storage_health(envelope, health);

    assert_eq!(decorated.status, ListStatus::Fresh);
    assert!(decorated.problems.is_empty());
    assert!(decorated.storage_health.as_ref().is_some_and(|health| {
        health.status == VenueOperationStatus::Warn && health.configured == Some(false)
    }));
}

#[test]
fn execution_ledger_storage_health_reports_configured_jsonl() {
    let health = execution_ledger_storage_health(
        &ExecutionLedgerStorageSnapshot {
            configured: true,
            path: Some("/tmp/execution_ledger.jsonl".into()),
            event_count: 3,
            replayed_events: 2,
            replay_failures: 0,
            append_successes: 1,
            append_failures: 0,
            last_append_at_ms: Some(900),
            query_successes: 4,
            query_failures: 0,
            last_query_at_ms: Some(950),
        },
        2,
        1_000,
    );

    assert_eq!(health.status, VenueOperationStatus::Ok);
    assert_eq!(health.source, EXECUTION_LEDGER_STORAGE_SOURCE);
    assert_eq!(health.configured, Some(true));
    assert_eq!(health.requested, Some(2));
    assert_eq!(health.rows, Some(3));
    assert_eq!(health.freshness_ms, Some(50));
    assert!(health.message.contains("replayed=2"));
    assert!(health.message.contains("query_ok=4"));
    assert!(health.problem.is_none());
}

#[test]
fn sql_ledger_storage_health_reports_configured_sql() {
    let health = sql_ledger_storage_health(
        &SqlLedgerStorageSnapshot {
            migration: trading::SqlLedgerMigrationHealth {
                configured: true,
                migration_id: "20260601_orders",
                migration_path: "crates/trading/migrations/20260601_orders.sql",
                migration_checksum: "fnv1a64:0000000000000000".to_owned(),
                schema_version: Some(3),
                applied: true,
                degraded_reason: None,
                last_success_at_ms: Some(900),
                last_error_at_ms: None,
                last_error: None,
                observed_at_ms: 900,
            },
            writer_configured: true,
            replay_query_successes: 3,
            replay_query_failures: 0,
            replay_event_rows: 4,
            replayed_events: 4,
            replay_event_failures: 0,
            replay_snapshot_rows: 2,
            replayed_order_snapshots: 2,
            replay_snapshot_failures: 0,
            replay_balance_rows: 1,
            replayed_balance_events: 1,
            replay_balance_failures: 0,
            replay_run_finality_rows: 1,
            replayed_run_finality_events: 1,
            replay_run_finality_failures: 0,
            replay_limited: false,
            replay_limit: 50_000,
            last_replay_at_ms: Some(950),
            last_replay_error_at_ms: None,
            last_replay_error: None,
            event_append_successes: 5,
            event_append_failures: 0,
            snapshot_append_successes: 2,
            snapshot_append_failures: 0,
            balance_append_successes: 1,
            balance_append_failures: 0,
            run_finality_append_successes: 1,
            run_finality_append_failures: 0,
            dropped_writes: 0,
            last_append_at_ms: Some(980),
            last_error_at_ms: None,
            last_error: None,
        },
        2,
        1_000,
    );

    assert_eq!(health.status, VenueOperationStatus::Ok);
    assert_eq!(health.source, SQL_LEDGER_STORAGE_SOURCE);
    assert_eq!(health.configured, Some(true));
    assert_eq!(health.rows, Some(4));
    assert_eq!(health.freshness_ms, Some(20));
    assert!(health.message.contains("replayed_events=4"));
    assert!(health.message.contains("replayed_balances=1"));
    assert!(health.message.contains("replayed_run_finality=1"));
    assert!(health.problem.is_none());
}

#[test]
fn execution_ledger_storage_health_blocks_on_storage_errors() {
    let health = execution_ledger_storage_health(
        &ExecutionLedgerStorageSnapshot {
            configured: true,
            path: Some("/tmp/execution_ledger.jsonl".into()),
            event_count: 0,
            replayed_events: 0,
            replay_failures: 1,
            append_successes: 0,
            append_failures: 2,
            last_append_at_ms: None,
            query_successes: 0,
            query_failures: 1,
            last_query_at_ms: Some(950),
        },
        0,
        1_000,
    );

    assert_eq!(health.status, VenueOperationStatus::Blocked);
    assert!(health
        .error
        .as_ref()
        .is_some_and(|error| error.contains("replay_failures=1")));
    assert!(health
        .error
        .as_ref()
        .is_some_and(|error| error.contains("append_failures=2")));
    assert!(health.problem.as_ref().is_some_and(|problem| {
        problem.code == codes::EXECUTION_LEDGER_STORAGE_IO_FAILED
            && problem.source.as_deref() == Some(EXECUTION_LEDGER_STORAGE_SOURCE)
    }));
}
