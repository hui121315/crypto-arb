fn run_finality_sample_problem(
    raw_order_id: &str,
    internal_order_id: &str,
    message: &str,
    error: Option<&str>,
) -> crate::services::run_finality::RunFinalitySampleProblem {
    crate::services::run_finality::RunFinalitySampleProblem {
        raw_order_id: raw_order_id.to_owned(),
        internal_order_id: internal_order_id.to_owned(),
        venue: "binance".to_owned(),
        source: "ExecutionRun".to_owned(),
        order_state: LiveOrderState::Accepted,
        code: codes::HEDGE_ORDER_FINALITY_FAILED.to_owned(),
        message: message.to_owned(),
        status: Some(502),
        checked_at_ms: 12,
        error: error.map(str::to_owned),
    }
}

fn order_record(
    exchange: &str,
    mode: ExecutionMode,
    state: LiveOrderState,
    created_at_ms: i64,
    updated_at_ms: i64,
) -> OrderRecord {
    OrderRecord {
        intent: shared_types::OrderIntent {
            id: format!("{exchange}-{created_at_ms}"),
            source: shared_types::OrderSource::Manual,
            strategy: None,
            mode,
            exchange: exchange.to_owned(),
            symbol: "BTCUSDT".to_owned(),
            side: shared_types::OrderSide::Buy,
            order_type: shared_types::OrderType::Limit,
            quantity: 1.0,
            price: Some(10.0),
            slippage_tolerance_bps: None,
            reduce_only: false,
            time_in_force: shared_types::TimeInForce::Ioc,
            post_only: false,
            margin_mode: shared_types::MarginMode::Cross,
            leverage: 1.0,
            client_order_id: format!("c-{created_at_ms}"),
            client_order_id_policy: None,
            created_at_ms,
        },
        state,
        risk: None,
        identity: Default::default(),
        last_update_source: Default::default(),
        exchange_order_id: None,
        message: None,
        filled_quantity: None,
        filled_price: None,
        filled_fee: None,
        updated_at_ms,
    }
}

fn nav_health(enabled: bool) -> NavStorageHealth {
    NavStorageHealth {
        path: enabled.then(|| "/tmp/crossline/nav.sqlite".to_owned()),
        enabled,
        load_success_total: 0,
        load_error_total: 0,
        append_success_total: 0,
        append_error_total: 0,
        last_success_at_ms: None,
        last_error_at_ms: None,
        latest_sample_at_ms: None,
        schema_version: None,
        migration_checksum: enabled.then(nav_persist::nav_schema_hash),
        sample_count: 0,
        last_error: None,
        latest_sample_status: None,
        latest_sample_source: None,
        latest_sample_problem: None,
        observed_at_ms: 10,
    }
}

fn sql_migration_health(
    configured: bool,
    applied: bool,
    error: Option<&str>,
) -> SqlLedgerMigrationHealth {
    SqlLedgerMigrationHealth {
        configured,
        migration_id: trading::SQL_LEDGER_MIGRATION_ID,
        migration_path: trading::SQL_LEDGER_MIGRATION_PATH,
        migration_checksum: trading::sql_ledger_schema_hash(),
        schema_version: applied.then_some(trading::SQL_LEDGER_MIGRATION_VERSION),
        applied,
        degraded_reason: if !configured {
            Some(StorageDegradedReason::Disabled)
        } else if applied {
            None
        } else {
            Some(StorageDegradedReason::MigrationFailed)
        },
        last_success_at_ms: applied.then_some(9),
        last_error_at_ms: error.map(|_| 8),
        last_error: error.map(str::to_owned),
        observed_at_ms: 7,
    }
}

fn sql_ledger_snapshot(
    migration: SqlLedgerMigrationHealth,
    writer_configured: bool,
) -> SqlLedgerStorageSnapshot {
    SqlLedgerStorageSnapshot {
        migration,
        writer_configured,
        replay_query_successes: 0,
        replay_query_failures: 0,
        replay_event_rows: 0,
        replayed_events: 0,
        replay_event_failures: 0,
        replay_snapshot_rows: 0,
        replayed_order_snapshots: 0,
        replay_snapshot_failures: 0,
        replay_balance_rows: 0,
        replayed_balance_events: 0,
        replay_balance_failures: 0,
        replay_run_finality_rows: 0,
        replayed_run_finality_events: 0,
        replay_run_finality_failures: 0,
        replay_limited: false,
        replay_limit: 0,
        last_replay_at_ms: None,
        last_replay_error_at_ms: None,
        last_replay_error: None,
        event_append_successes: 0,
        event_append_failures: 0,
        snapshot_append_successes: 0,
        snapshot_append_failures: 0,
        balance_append_successes: 0,
        balance_append_failures: 0,
        run_finality_append_successes: 0,
        run_finality_append_failures: 0,
        dropped_writes: 0,
        last_append_at_ms: None,
        last_error_at_ms: None,
        last_error: None,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct AuditHealthSpec<'a> {
    configured: bool,
    opened: bool,
    write_attempts: u64,
    write_successes: u64,
    write_failures: u64,
    last_write_at_ms: Option<i64>,
    last_error: Option<&'a str>,
}

fn audit_health(spec: AuditHealthSpec<'_>) -> AuditSinkHealthSnapshot {
    AuditSinkHealthSnapshot {
        initialized: true,
        configured: spec.configured,
        opened: spec.opened,
        path: spec
            .configured
            .then(|| "/tmp/crossline/audit.jsonl".to_owned()),
        queue_capacity: if spec.configured && spec.opened {
            1024
        } else {
            0
        },
        pending_writes: spec
            .write_attempts
            .saturating_sub(spec.write_successes.saturating_add(spec.write_failures)),
        writer_alive: spec.configured && spec.opened,
        write_attempts: spec.write_attempts,
        write_successes: spec.write_successes,
        write_failures: spec.write_failures,
        last_write_at_ms: spec.last_write_at_ms,
        last_error_at_ms: spec.last_error.map(|_| 9_000),
        last_error: spec.last_error.map(str::to_owned),
        observed_at_ms: 10,
    }
}

fn mark_history_migration_applied(health: &mut HistoryStoreHealth) {
    let authority = HistoryMigrationStatus {
        migration_id: "20260701_history".to_owned(),
        schema_name: "realtime_history".to_owned(),
        migration_path: "crates/realtime/migrations/20260701_history.sql".to_owned(),
        schema_version: Some(realtime::HISTORY_SCHEMA_VERSION),
        migration_checksum: health.migration_checksum.clone(),
        applied: true,
        applied_at_ms: Some(9),
    };
    health.migration_status = Some(authority.clone());
    health.storage_contract.backend_kind = shared_types::StorageBackendKind::Postgres;
    health.storage_contract.migration_authority = Some(authority);
    health
        .storage_contract
        .degraded_reasons
        .retain(|reason| *reason != StorageDegradedReason::Ephemeral);
}
