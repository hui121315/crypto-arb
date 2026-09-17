#[test]
fn task_registry_stale_issue_is_blocking_runtime_evidence() {
    let registry = crate::task_registry::TaskRegistry::default();
    registry.register("market_prewarm", 5_000);

    let rows = task_registry_rows(&registry, common::time::now_ms() + 10 * 60_000);

    let task = rows
        .iter()
        .find(|row| row.operation == "background_task:market_prewarm")
        .expect("stale task row");
    assert_eq!(task.status, VenueOperationStatus::Blocked);
    assert!(
        task.problem
            .as_ref()
            .is_some_and(|problem| problem.code == "TASK_STALE")
    );
    assert_eq!(task.latency_ms, None);
    let evidence = task.evidence.as_ref().expect("task evidence");
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "slow_threshold_ms=7500")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "slow_tick_count=0")
    );
}

#[test]
fn audit_log_storage_row_warns_when_unconfigured() {
    let row = audit_log_storage_row(
        &audit_health(AuditHealthSpec {
            configured: false,
            opened: false,
            ..Default::default()
        }),
        10_000,
    );

    assert_eq!(row.venue, SYSTEM_VENUE);
    assert_eq!(row.operation, OP_AUDIT_LOG_STORAGE);
    assert_eq!(row.source, SOURCE_AUDIT_LOG_STORAGE);
    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.configured, Some(false));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::AUDIT_STORAGE_UNAVAILABLE)
    );
}

#[test]
fn audit_log_storage_row_reports_configured_open_sink() {
    let row = audit_log_storage_row(
        &audit_health(AuditHealthSpec {
            configured: true,
            opened: true,
            write_attempts: 2,
            write_successes: 2,
            last_write_at_ms: Some(9_500),
            ..Default::default()
        }),
        10_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.configured, Some(true));
    assert_eq!(row.requested, Some(2));
    assert_eq!(row.rows, Some(2));
    assert_eq!(row.freshness_ms, Some(500));
    assert!(row.problem.is_none());
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "opened=true")
    }));
    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item == "writer_alive=true")
    }));
}

#[test]
fn audit_log_storage_row_reports_async_writer_queue_health() {
    let row = audit_log_storage_row(
        &audit_health(AuditHealthSpec {
            configured: true,
            opened: true,
            write_attempts: 3,
            write_successes: 2,
            last_write_at_ms: Some(9_500),
            ..Default::default()
        }),
        10_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Ok);
    assert_eq!(row.requested, Some(3));
    assert_eq!(row.rows, Some(2));
    assert!(row.message.contains("pending=1"));
    assert!(row.message.contains("capacity=1024"));
    assert!(row.message.contains("writer_alive=true"));
    let evidence = row.evidence.as_ref().expect("audit evidence");
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "queue_capacity=1024")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "pending_writes=1")
    );
    assert!(
        evidence
            .request_context
            .iter()
            .any(|item| item == "writer_alive=true")
    );
}

#[test]
fn audit_log_storage_row_blocks_open_or_write_failure() {
    let row = audit_log_storage_row(
        &audit_health(AuditHealthSpec {
            configured: true,
            opened: false,
            write_attempts: 1,
            write_failures: 1,
            last_error: Some("open_failed: permission denied"),
            ..Default::default()
        }),
        10_000,
    );

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::AUDIT_STORAGE_UNAVAILABLE)
    );
    assert!(
        row.problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("lastError"))
            .is_some_and(|error| error.as_str() == Some("open_failed: permission denied"))
    );
}

#[test]
fn history_storage_row_maps_disabled_backend_to_blocked_problem() {
    let health = realtime::HistoryStore::disabled().health_snapshot(10);

    let row = history_storage_row(&health, 10);

    assert_eq!(row.venue, SYSTEM_VENUE);
    assert_eq!(row.operation, OP_HISTORY_STORAGE);
    assert_eq!(row.source, SOURCE_HISTORY_STORAGE);
    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HISTORY_STORE_UNAVAILABLE)
    );
}

#[test]
fn history_storage_row_warns_on_memory_fallback() {
    let health =
        realtime::HistoryStore::memory_fallback("postgres connect failed").health_snapshot(10);

    let row = history_storage_row(&health, 10);

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert_eq!(row.configured, Some(true));
    assert!(row.message.contains("fallback"));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HISTORY_STORE_UNAVAILABLE)
    );
}

#[test]
fn history_storage_row_warns_on_timescale_degraded_postgres() {
    let mut health = realtime::HistoryStore::new(10).health_snapshot(10);
    health.backend = "postgres";
    health.durable = true;
    health.ephemeral = false;
    mark_history_migration_applied(&mut health);
    health.timescale_status = Some(HistoryTimescaleStatus::PlainPostgres);
    health.timescale_problem = Some("timescaledb extension missing".to_owned());
    health
        .storage_contract
        .degraded_reasons
        .push(StorageDegradedReason::ExtensionDegraded);

    let row = history_storage_row(&health, 10);

    assert_eq!(row.status, VenueOperationStatus::Warn);
    assert!(row.message.contains("Timescale"));
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HISTORY_STORE_DEGRADED)
    );
    assert!(
        row.problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("timescaleStatus"))
            .is_some_and(|status| status.as_str() == Some("plain_postgres"))
    );
}

#[test]
fn history_storage_row_blocks_schema_drift_with_typed_problem() {
    let mut health = realtime::HistoryStore::new(10).health_snapshot(10);
    health.backend = "postgres";
    health.durable = true;
    health.ephemeral = false;
    mark_history_migration_applied(&mut health);
    health.append_error_total = 1;
    health.last_error_at_ms = Some(9_000);
    health.last_error = Some("history schema drift: relation funding_rates missing".into());
    health.last_error_code = Some(codes::HISTORY_SCHEMA_DRIFT.into());
    health
        .storage_contract
        .degraded_reasons
        .push(StorageDegradedReason::SchemaDrift);

    let row = history_storage_row(&health, 10_000);

    assert_eq!(row.status, VenueOperationStatus::Blocked);
    assert_eq!(
        row.problem.as_ref().map(|problem| problem.code.as_str()),
        Some(codes::HISTORY_SCHEMA_DRIFT)
    );
    assert!(
        row.problem
            .as_ref()
            .and_then(|problem| problem.details.as_ref())
            .and_then(|details| details.get("lastErrorCode"))
            .is_some_and(|code| code.as_str() == Some(codes::HISTORY_SCHEMA_DRIFT))
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
fn history_storage_row_preserves_migration_checksum_evidence() {
    let health = realtime::HistoryStore::new(10).health_snapshot(10);

    let row = history_storage_row(&health, 10);

    assert!(row.evidence.as_ref().is_some_and(|evidence| {
        evidence
            .request_context
            .iter()
            .any(|item| item.starts_with("migration_checksum="))
    }));
}
