fn trading_sql_ledger_storage_problem(
    snapshot: &SqlLedgerStorageSnapshot,
    status: VenueOperationStatus,
    message: &str,
) -> Option<ApiProblem> {
    if status == VenueOperationStatus::Ok {
        return None;
    }
    let mut problem = ApiProblem::new(
        trading_sql_ledger_storage_problem_code(snapshot),
        message.to_owned(),
    )
    .with_source(SOURCE_TRADING_SQL_LEDGER_WRITER);
    problem.details = Some(serde_json::json!({
        "storageContract": snapshot.migration.storage_contract(),
        "migrationConfigured": snapshot.migration.configured,
        "migrationApplied": snapshot.migration.applied,
        "writerConfigured": snapshot.writer_configured,
        "schemaVersion": snapshot.migration.schema_version,
        "migrationChecksum": snapshot.migration.migration_checksum,
        "replayQuerySuccesses": snapshot.replay_query_successes,
        "replayQueryFailures": snapshot.replay_query_failures,
        "replayEventRows": snapshot.replay_event_rows,
        "replayedEvents": snapshot.replayed_events,
        "replayEventFailures": snapshot.replay_event_failures,
        "replaySnapshotRows": snapshot.replay_snapshot_rows,
        "replayedOrderSnapshots": snapshot.replayed_order_snapshots,
        "replaySnapshotFailures": snapshot.replay_snapshot_failures,
        "replayBalanceRows": snapshot.replay_balance_rows,
        "replayedBalanceEvents": snapshot.replayed_balance_events,
        "replayBalanceFailures": snapshot.replay_balance_failures,
        "replayRunFinalityRows": snapshot.replay_run_finality_rows,
        "replayedRunFinalityEvents": snapshot.replayed_run_finality_events,
        "replayRunFinalityFailures": snapshot.replay_run_finality_failures,
        "replayLimited": snapshot.replay_limited,
        "replayLimit": snapshot.replay_limit,
        "lastReplayAtMs": snapshot.last_replay_at_ms,
        "lastReplayErrorAtMs": snapshot.last_replay_error_at_ms,
        "lastReplayError": snapshot.last_replay_error.as_deref(),
        "eventAppendSuccesses": snapshot.event_append_successes,
        "eventAppendFailures": snapshot.event_append_failures,
        "snapshotAppendSuccesses": snapshot.snapshot_append_successes,
        "snapshotAppendFailures": snapshot.snapshot_append_failures,
        "balanceAppendSuccesses": snapshot.balance_append_successes,
        "balanceAppendFailures": snapshot.balance_append_failures,
        "runFinalityAppendSuccesses": snapshot.run_finality_append_successes,
        "runFinalityAppendFailures": snapshot.run_finality_append_failures,
        "droppedWrites": snapshot.dropped_writes,
        "lastAppendAtMs": snapshot.last_append_at_ms,
        "lastErrorAtMs": snapshot.last_error_at_ms,
        "lastError": snapshot.last_error.as_deref(),
    }));
    Some(problem)
}

fn trading_sql_ledger_storage_problem_code(snapshot: &SqlLedgerStorageSnapshot) -> &'static str {
    if snapshot.migration.degraded_reason == Some(StorageDegradedReason::SchemaDrift) {
        return codes::TRADING_SQL_SCHEMA_DRIFT;
    }
    if !snapshot.migration.configured || !snapshot.migration.applied || !snapshot.writer_configured
    {
        return codes::TRADING_SQL_LEDGER_UNAVAILABLE;
    }
    codes::TRADING_SQL_LEDGER_WRITE_FAILED
}

fn trading_sql_ledger_storage_evidence(
    snapshot: &SqlLedgerStorageSnapshot,
) -> VenueOperationEvidence {
    VenueOperationEvidence {
        method: "postgres_sql".to_owned(),
        path: "order_events/order_snapshots/balance_events/run_finality_events".to_owned(),
        checked_at: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        doc_version: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        schema_hash: snapshot.migration.migration_checksum.clone(),
        fixture_id: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        parser_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_builder_test: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        auth_kind: UNRECORDED_EVIDENCE_MARKER.to_owned(),
        request_id: None,
        request_context: trading_sql_ledger_context(snapshot),
        doc_urls: vec![
            POSTGRES_CREATE_TABLE_DOC_URL.to_owned(),
            POSTGRES_INSERT_DOC_URL.to_owned(),
            POSTGRES_SELECT_DOC_URL.to_owned(),
            POSTGRES_JSON_DOC_URL.to_owned(),
        ],
        use_cases: vec![
            "trading_sql_ledger_writer".to_owned(),
            "trading_sql_ledger_replay".to_owned(),
            "order_event_append_sink".to_owned(),
            "order_snapshot_append_sink".to_owned(),
            "balance_event_replay".to_owned(),
            "run_finality_event_append_sink".to_owned(),
            "run_finality_event_replay".to_owned(),
        ],
        data_kinds: vec![
            "execution_ledger_event".to_owned(),
            "order_snapshot".to_owned(),
            "venue_balance".to_owned(),
            "execution_run_finality".to_owned(),
            "close_run_finality".to_owned(),
        ],
        rate_scopes: Vec::new(),
        weight: 0,
    }
}

fn trading_sql_ledger_context(snapshot: &SqlLedgerStorageSnapshot) -> Vec<String> {
    let mut request_context = trading_sql_ledger_counter_context(snapshot);
    push_trading_sql_ledger_optional_context(&mut request_context, snapshot);
    request_context
}

fn trading_sql_ledger_counter_context(snapshot: &SqlLedgerStorageSnapshot) -> Vec<String> {
    let mut context = vec![
        format!(
            "backend_kind={}",
            snapshot.migration.storage_contract().backend_kind
        ),
        format!("migration_configured={}", snapshot.migration.configured),
        format!("migration_applied={}", snapshot.migration.applied),
        format!("writer_configured={}", snapshot.writer_configured),
        format!("migration_id={}", snapshot.migration.migration_id),
        format!("migration_path={}", snapshot.migration.migration_path),
        format!(
            "schema_version={}",
            optional_u32_label(snapshot.migration.schema_version)
        ),
        format!(
            "migration_checksum={}",
            snapshot.migration.migration_checksum
        ),
        format!("replay_query_successes={}", snapshot.replay_query_successes),
        format!("replay_query_failures={}", snapshot.replay_query_failures),
        format!("replay_event_rows={}", snapshot.replay_event_rows),
        format!("replayed_events={}", snapshot.replayed_events),
        format!("replay_event_failures={}", snapshot.replay_event_failures),
        format!("replay_snapshot_rows={}", snapshot.replay_snapshot_rows),
        format!(
            "replayed_order_snapshots={}",
            snapshot.replayed_order_snapshots
        ),
        format!(
            "replay_snapshot_failures={}",
            snapshot.replay_snapshot_failures
        ),
        format!("replay_balance_rows={}", snapshot.replay_balance_rows),
        format!(
            "replayed_balance_events={}",
            snapshot.replayed_balance_events
        ),
        format!(
            "replay_balance_failures={}",
            snapshot.replay_balance_failures
        ),
        format!(
            "replay_run_finality_rows={}",
            snapshot.replay_run_finality_rows
        ),
        format!(
            "replayed_run_finality_events={}",
            snapshot.replayed_run_finality_events
        ),
        format!(
            "replay_run_finality_failures={}",
            snapshot.replay_run_finality_failures
        ),
        format!("replay_limited={}", snapshot.replay_limited),
        format!("replay_limit={}", snapshot.replay_limit),
        format!("event_append_successes={}", snapshot.event_append_successes),
        format!("event_append_failures={}", snapshot.event_append_failures),
        format!(
            "snapshot_append_successes={}",
            snapshot.snapshot_append_successes
        ),
        format!(
            "snapshot_append_failures={}",
            snapshot.snapshot_append_failures
        ),
        format!(
            "balance_append_successes={}",
            snapshot.balance_append_successes
        ),
        format!(
            "balance_append_failures={}",
            snapshot.balance_append_failures
        ),
        format!(
            "run_finality_append_successes={}",
            snapshot.run_finality_append_successes
        ),
        format!(
            "run_finality_append_failures={}",
            snapshot.run_finality_append_failures
        ),
        format!("dropped_writes={}", snapshot.dropped_writes),
    ];
    if let Some(reason) = snapshot.migration.degraded_reason {
        context.push(format!("degraded_reason={reason}"));
    }
    context
}

fn push_trading_sql_ledger_optional_context(
    request_context: &mut Vec<String>,
    snapshot: &SqlLedgerStorageSnapshot,
) {
    push_optional_i64_context(
        request_context,
        "last_replay_at_ms",
        snapshot.last_replay_at_ms,
    );
    push_optional_i64_context(
        request_context,
        "last_replay_error_at_ms",
        snapshot.last_replay_error_at_ms,
    );
    push_optional_context(
        request_context,
        "last_replay_error",
        snapshot.last_replay_error.as_deref(),
    );
    push_optional_i64_context(
        request_context,
        "last_append_at_ms",
        snapshot.last_append_at_ms,
    );
    push_optional_i64_context(
        request_context,
        "last_error_at_ms",
        snapshot.last_error_at_ms,
    );
    push_optional_context(
        request_context,
        "last_error",
        snapshot.last_error.as_deref(),
    );
}

fn trading_sql_ledger_storage_rows(snapshot: &SqlLedgerStorageSnapshot) -> usize {
    snapshot
        .event_append_successes
        .saturating_add(snapshot.snapshot_append_successes)
        .saturating_add(snapshot.balance_append_successes)
        .saturating_add(snapshot.run_finality_append_successes)
        .saturating_add(snapshot.replayed_events)
        .saturating_add(snapshot.replayed_order_snapshots)
        .saturating_add(snapshot.replayed_balance_events)
        .saturating_add(snapshot.replayed_run_finality_events)
}

fn trading_sql_ledger_storage_io_total(snapshot: &SqlLedgerStorageSnapshot) -> u64 {
    snapshot
        .event_append_successes
        .saturating_add(snapshot.event_append_failures)
        .saturating_add(snapshot.snapshot_append_successes)
        .saturating_add(snapshot.snapshot_append_failures)
        .saturating_add(snapshot.balance_append_successes)
        .saturating_add(snapshot.balance_append_failures)
        .saturating_add(snapshot.run_finality_append_successes)
        .saturating_add(snapshot.run_finality_append_failures)
        .saturating_add(snapshot.dropped_writes)
        .saturating_add(snapshot.replay_query_successes)
        .saturating_add(snapshot.replay_query_failures)
        .saturating_add(snapshot.replayed_events)
        .saturating_add(snapshot.replay_event_failures)
        .saturating_add(snapshot.replayed_order_snapshots)
        .saturating_add(snapshot.replay_snapshot_failures)
        .saturating_add(snapshot.replayed_balance_events)
        .saturating_add(snapshot.replay_balance_failures)
        .saturating_add(snapshot.replayed_run_finality_events)
        .saturating_add(snapshot.replay_run_finality_failures) as u64
}
