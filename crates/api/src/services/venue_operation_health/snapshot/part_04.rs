fn task_registry_summary_row(
    total: usize,
    enabled: usize,
    issues: &[TaskIssue],
    slow_count: usize,
    now_ms: i64,
) -> VenueOperationHealth {
    let healthy = enabled
        .saturating_sub(issues.len())
        .saturating_sub(slow_count);
    let disabled = total.saturating_sub(enabled);
    let status = task_registry_summary_status(enabled, issues, slow_count);
    let message =
        task_registry_summary_message(total, enabled, healthy, disabled, issues.len(), slow_count);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: OP_BACKGROUND_TASKS.to_owned(),
        status,
        source: SOURCE_TASK_REGISTRY.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(enabled > 0),
        requested: Some(enabled as u64),
        rows: Some(healthy as u64),
        freshness_ms: Some(0),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: None,
        problem: None,
        observed_at_ms: now_ms,
    }
}

fn task_registry_task_row(snapshot: &TaskSnapshot, now_ms: i64) -> VenueOperationHealth {
    let status = task_snapshot_status(snapshot);
    let message = task_snapshot_message(snapshot, now_ms);
    let freshness_ms = snapshot.enabled.then_some(snapshot.lag_ms.max(0));
    let problem = snapshot
        .issue
        .as_ref()
        .map(|issue| task_issue_problem(issue, &message));
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: format!("{OP_BACKGROUND_TASK_PREFIX}{}", snapshot.name),
        status,
        source: SOURCE_TASK_REGISTRY.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(snapshot.enabled),
        requested: Some(u64::from(snapshot.enabled)),
        rows: Some(u64::from(
            snapshot.enabled && status == VenueOperationStatus::Ok,
        )),
        freshness_ms,
        retry_after_ms: snapshot.retry_after_ms,
        latency_ms: snapshot.last_duration_ms,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(task_snapshot_evidence(snapshot)),
        problem,
        observed_at_ms: task_snapshot_observed_at(snapshot).unwrap_or(now_ms),
    }
}

pub(crate) fn history_storage_row(
    health: &HistoryStoreHealth,
    now_ms: i64,
) -> VenueOperationHealth {
    let status = history_storage_status(health, now_ms);
    let message = history_storage_message(health, status, now_ms);
    let observed_at_ms = history_storage_observed_at(health).unwrap_or(health.observed_at_ms);
    let problem = history_storage_problem(health, status, &message);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: OP_HISTORY_STORAGE.to_owned(),
        status,
        source: SOURCE_HISTORY_STORAGE.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(health.enabled),
        requested: Some(history_storage_io_total(health)),
        rows: Some(health.success_total()),
        freshness_ms: history_storage_observed_at(health).map(|at| freshness_since(at, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(history_storage_evidence(health)),
        problem,
        observed_at_ms,
    }
}

fn audit_log_storage_row(snapshot: &AuditSinkHealthSnapshot, now_ms: i64) -> VenueOperationHealth {
    let status = audit_log_storage_status(snapshot);
    let message = audit_log_storage_message(snapshot, status);
    let problem = audit_log_storage_problem(snapshot, status, &message);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: OP_AUDIT_LOG_STORAGE.to_owned(),
        status,
        source: SOURCE_AUDIT_LOG_STORAGE.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(snapshot.configured),
        requested: Some(snapshot.write_attempts),
        rows: Some(snapshot.write_successes),
        freshness_ms: audit_log_observed_at(snapshot)
            .map(|observed_at_ms| freshness_since(observed_at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(audit_log_storage_evidence(snapshot)),
        problem,
        observed_at_ms: audit_log_observed_at(snapshot).unwrap_or(snapshot.observed_at_ms),
    }
}

fn nav_storage_row(health: &NavStorageHealth, now_ms: i64) -> VenueOperationHealth {
    let status = nav_storage_status(health, now_ms);
    let message = nav_storage_message(health, status, now_ms);
    let observed_at_ms = nav_storage_observed_at(health).unwrap_or(health.observed_at_ms);
    let problem = nav_storage_problem(health, status, &message);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: OP_NAV_STORAGE.to_owned(),
        status,
        source: SOURCE_NAV_STORAGE.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(health.enabled),
        requested: Some(nav_storage_io_total(health)),
        rows: Some(health.sample_count),
        freshness_ms: nav_storage_observed_at(health).map(|at| freshness_since(at, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(nav_storage_evidence(health)),
        problem,
        observed_at_ms,
    }
}

fn execution_ledger_storage_row(
    snapshot: &ExecutionLedgerStorageSnapshot,
    now_ms: i64,
) -> VenueOperationHealth {
    let status = execution_ledger_storage_status(snapshot);
    let message = execution_ledger_storage_message(snapshot);
    let problem = execution_ledger_storage_problem(snapshot, status, &message);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: OP_EXECUTION_LEDGER_STORAGE.to_owned(),
        status,
        source: SOURCE_EXECUTION_LEDGER_STORAGE.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(snapshot.configured),
        requested: Some(execution_ledger_storage_io_total(snapshot)),
        rows: Some(snapshot.event_count as u64),
        freshness_ms: execution_ledger_storage_observed_at(snapshot)
            .map(|observed_at_ms| freshness_since(observed_at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(execution_ledger_storage_evidence(snapshot)),
        problem,
        observed_at_ms: execution_ledger_storage_observed_at(snapshot).unwrap_or(now_ms),
    }
}

fn order_snapshot_storage_row(
    snapshot: &OrderSnapshotStorageSnapshot,
    now_ms: i64,
) -> VenueOperationHealth {
    let status = order_snapshot_storage_status(snapshot);
    let message = order_snapshot_storage_message(snapshot);
    let problem = order_snapshot_storage_problem(snapshot, status, &message);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: OP_ORDER_SNAPSHOT_STORAGE.to_owned(),
        status,
        source: SOURCE_ORDER_SNAPSHOT_STORAGE.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(snapshot.configured),
        requested: Some(order_snapshot_storage_io_total(snapshot)),
        rows: Some(snapshot.record_count as u64),
        freshness_ms: snapshot
            .last_append_at_ms
            .map(|observed_at_ms| freshness_since(observed_at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(order_snapshot_storage_evidence(snapshot)),
        problem,
        observed_at_ms: snapshot.last_append_at_ms.unwrap_or(now_ms),
    }
}

fn trading_sql_migration_storage_row(
    health: &SqlLedgerMigrationHealth,
    now_ms: i64,
) -> VenueOperationHealth {
    let status = trading_sql_migration_status(health);
    let message = trading_sql_migration_message(health);
    let problem = trading_sql_migration_problem(health, status, &message);
    let observed_at_ms = trading_sql_migration_observed_at(health);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: OP_TRADING_SQL_MIGRATIONS_STORAGE.to_owned(),
        status,
        source: SOURCE_TRADING_SQL_MIGRATION.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(health.configured),
        requested: Some(1),
        rows: Some(u64::from(health.applied)),
        freshness_ms: trading_sql_migration_latest_at(health)
            .map(|latest_at_ms| freshness_since(latest_at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(trading_sql_migration_evidence(health)),
        problem,
        observed_at_ms,
    }
}

fn account_cache_row(
    operation: &str,
    snapshot: AccountCacheSnapshot,
    credential: Option<&VenueCredentialStatus>,
) -> VenueOperationHealth {
    let status = account_cache_status(operation, snapshot.quality, snapshot.freshness_ms);
    let message =
        account_cache_message(operation, snapshot.quality, snapshot.freshness_ms).to_owned();
    VenueOperationHealth {
        venue: snapshot.venue,
        operation: operation.to_owned(),
        status,
        source: SOURCE_ACCOUNT_CACHE.to_owned(),
        message: message.clone(),
        supported: credential.map(|venue| venue.private_read),
        configured: credential.map(credentials_configured),
        requested: None,
        rows: Some(snapshot.rows),
        freshness_ms: Some(snapshot.freshness_ms),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: account_cache_error(status, &message),
        evidence: None,
        problem: None,
        observed_at_ms: snapshot.observed_at_ms,
    }
}
