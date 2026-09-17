fn trading_sql_migration_message(health: &SqlLedgerMigrationHealth) -> String {
    if !health.configured {
        return format!(
            "Trading SQL migration 未配置 Postgres，{} 未运行",
            health.migration_id
        );
    }
    if let Some(error) = health.last_error.as_deref() {
        return format!(
            "Trading SQL migration {} 失败：{}",
            health.migration_id, error
        );
    }
    format!(
        "Trading SQL migration {} 已应用，schema_version={}，checksum={}",
        health.migration_id,
        optional_u32_label(health.schema_version),
        health.migration_checksum
    )
}

fn trading_sql_migration_problem(
    health: &SqlLedgerMigrationHealth,
    status: VenueOperationStatus,
    message: &str,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok {
        return None;
    }
    let mut problem = ApiProblem::new(trading_sql_migration_problem_code(health), message)
        .with_source(SOURCE_TRADING_SQL_MIGRATION);
    problem.details = Some(serde_json::json!({
        "storageContract": health.storage_contract(),
        "configured": health.configured,
        "applied": health.applied,
        "migrationId": health.migration_id,
        "migrationPath": health.migration_path,
        "schemaVersion": health.schema_version,
        "migrationChecksum": health.migration_checksum,
        "lastSuccessAtMs": health.last_success_at_ms,
        "lastErrorAtMs": health.last_error_at_ms,
        "lastError": health.last_error.as_deref(),
    }));
    Some(problem)
}

fn trading_sql_migration_problem_code(health: &SqlLedgerMigrationHealth) -> &'static str {
    if !health.configured {
        return codes::TRADING_SQL_MIGRATION_UNAVAILABLE;
    }
    if health.degraded_reason == Some(StorageDegradedReason::SchemaDrift) {
        return codes::TRADING_SQL_SCHEMA_DRIFT;
    }
    codes::TRADING_SQL_MIGRATION_FAILED
}

fn trading_sql_migration_evidence(health: &SqlLedgerMigrationHealth) -> VenueOperationEvidence {
    let authority = health.migration_authority();
    let mut request_context = vec![
        format!("backend_kind={}", health.storage_contract().backend_kind),
        format!("configured={}", health.configured),
        format!("applied={}", health.applied),
        format!("migration_id={}", authority.migration_id),
        format!("migration_path={}", authority.migration_path),
        format!("schema_name={}", authority.schema_name),
        format!(
            "schema_version={}",
            optional_u32_label(authority.schema_version)
        ),
        format!(
            "migration_checksum={}",
            authority.migration_checksum.as_deref().unwrap_or("unknown")
        ),
    ];
    if let Some(reason) = health.degraded_reason {
        request_context.push(format!("degraded_reason={reason}"));
    }
    push_optional_i64_context(
        &mut request_context,
        "last_success_at_ms",
        health.last_success_at_ms,
    );
    push_optional_i64_context(
        &mut request_context,
        "last_error_at_ms",
        health.last_error_at_ms,
    );
    push_optional_context(
        &mut request_context,
        "last_error",
        health.last_error.as_deref(),
    );
    VenueOperationEvidence {
        method: "postgres_sql".to_owned(),
        path: "schema_migrations/order_events/order_snapshots".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: health.migration_checksum.clone(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: None,
        request_context,
        doc_urls: vec![
            POSTGRES_CREATE_TABLE_DOC_URL.to_owned(),
            POSTGRES_INSERT_DOC_URL.to_owned(),
        ],
        use_cases: vec![
            "trading_sql_migration_runner".to_owned(),
            "order_ledger_schema_source".to_owned(),
        ],
        data_kinds: vec![
            "schema_migration".to_owned(),
            "order_event".to_owned(),
            "order_snapshot".to_owned(),
        ],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn trading_sql_migration_latest_at(health: &SqlLedgerMigrationHealth) -> Option<i64> {
    match (health.last_success_at_ms, health.last_error_at_ms) {
        (Some(success), Some(error)) => Some(success.max(error)),
        (Some(success), None) => Some(success),
        (None, Some(error)) => Some(error),
        (None, None) => None,
    }
}

fn trading_sql_migration_observed_at(health: &SqlLedgerMigrationHealth) -> i64 {
    trading_sql_migration_latest_at(health).unwrap_or(health.observed_at_ms)
}

fn trading_sql_ledger_storage_row(
    snapshot: &SqlLedgerStorageSnapshot,
    now_ms: i64,
) -> VenueOperationHealth {
    let status = trading_sql_ledger_storage_status(snapshot);
    let message = trading_sql_ledger_storage_message(snapshot);
    let problem = trading_sql_ledger_storage_problem(snapshot, status, &message);
    let observed_at_ms = trading_sql_ledger_observed_at(snapshot);
    VenueOperationHealth {
        venue: SYSTEM_VENUE.to_owned(),
        operation: OP_TRADING_SQL_LEDGER_STORAGE.to_owned(),
        status,
        source: SOURCE_TRADING_SQL_LEDGER_WRITER.to_owned(),
        message: message.clone(),
        supported: Some(true),
        configured: Some(snapshot.migration.configured),
        requested: Some(trading_sql_ledger_storage_io_total(snapshot)),
        rows: Some(trading_sql_ledger_storage_rows(snapshot) as u64),
        freshness_ms: trading_sql_ledger_latest_at(snapshot)
            .map(|latest_at_ms| freshness_since(latest_at_ms, now_ms)),
        retry_after_ms: None,
        latency_ms: None,
        latency_p95_ms: None,
        error: attention_error(status, &message),
        evidence: Some(trading_sql_ledger_storage_evidence(snapshot)),
        problem,
        observed_at_ms,
    }
}

fn trading_sql_ledger_storage_status(snapshot: &SqlLedgerStorageSnapshot) -> VenueOperationStatus {
    if !snapshot.migration.configured {
        return VenueOperationStatus::Warn;
    }
    if !snapshot.migration.applied || !snapshot.writer_configured {
        return VenueOperationStatus::Blocked;
    }
    if snapshot.event_append_failures > 0
        || snapshot.snapshot_append_failures > 0
        || snapshot.balance_append_failures > 0
        || snapshot.run_finality_append_failures > 0
        || snapshot.dropped_writes > 0
        || snapshot.replay_query_failures > 0
        || snapshot.replay_event_failures > 0
        || snapshot.replay_snapshot_failures > 0
        || snapshot.replay_balance_failures > 0
        || snapshot.replay_run_finality_failures > 0
    {
        return VenueOperationStatus::Blocked;
    }
    if snapshot.replay_limited {
        return VenueOperationStatus::Warn;
    }
    VenueOperationStatus::Ok
}

fn trading_sql_ledger_storage_message(snapshot: &SqlLedgerStorageSnapshot) -> String {
    let io = format!(
        "event_ok={}，event_failed={}，snapshot_ok={}，snapshot_failed={}，balance_ok={}，balance_failed={}，run_finality_ok={}，run_finality_failed={}，dropped={}",
        snapshot.event_append_successes,
        snapshot.event_append_failures,
        snapshot.snapshot_append_successes,
        snapshot.snapshot_append_failures,
        snapshot.balance_append_successes,
        snapshot.balance_append_failures,
        snapshot.run_finality_append_successes,
        snapshot.run_finality_append_failures,
        snapshot.dropped_writes
    );
    let replay = format!(
        "replay_query_ok={}，replay_query_failed={}，event_rows={}，events_replayed={}，event_decode_failed={}，snapshot_rows={}，snapshots_replayed={}，snapshot_decode_failed={}，balance_rows={}，balances_replayed={}，balance_decode_failed={}，run_finality_rows={}，run_finality_replayed={}，run_finality_decode_failed={}",
        snapshot.replay_query_successes,
        snapshot.replay_query_failures,
        snapshot.replay_event_rows,
        snapshot.replayed_events,
        snapshot.replay_event_failures,
        snapshot.replay_snapshot_rows,
        snapshot.replayed_order_snapshots,
        snapshot.replay_snapshot_failures,
        snapshot.replay_balance_rows,
        snapshot.replayed_balance_events,
        snapshot.replay_balance_failures,
        snapshot.replay_run_finality_rows,
        snapshot.replayed_run_finality_events,
        snapshot.replay_run_finality_failures
    );
    if !snapshot.migration.configured {
        return format!("Trading SQL ledger 未配置 Postgres，{io}，{replay}");
    }
    if !snapshot.migration.applied {
        return format!(
            "Trading SQL ledger 阻断：migration 未应用，{}",
            snapshot
                .migration
                .last_error
                .as_deref()
                .unwrap_or("unknown migration error")
        );
    }
    if !snapshot.writer_configured {
        return format!(
            "Trading SQL ledger writer 未启动，migration 已应用但无 append sink，{replay}"
        );
    }
    if let Some(error) = snapshot.last_replay_error.as_deref() {
        return format!("Trading SQL ledger replay/query 异常：{error}，{io}，{replay}");
    }
    if let Some(error) = snapshot.last_error.as_deref() {
        return format!("Trading SQL ledger writer 最近写入异常：{error}，{io}，{replay}");
    }
    if snapshot.replay_limited {
        return format!(
            "Trading SQL ledger replay 达到上限 {} 行，需分页/游标补全，{}，{}",
            snapshot.replay_limit, io, replay
        );
    }
    format!(
        "Trading SQL ledger writer/replay 可用，schema_version={}，checksum={}，{}，{}",
        optional_u32_label(snapshot.migration.schema_version),
        snapshot.migration.migration_checksum,
        io,
        replay
    )
}
