use super::*;

const FNV64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV64_PRIME: u64 = 0x100000001b3;

#[test]
fn registry_order_paths_versions_and_checksums_are_frozen() {
    assert_eq!(ordered().len(), 4);
    assert_eq!(
        ordered()
            .iter()
            .map(|migration| (
                migration.id,
                migration.path,
                migration.schema_version,
                migration.checksum,
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "20260601_orders",
                "crates/trading/migrations/20260601_orders.sql",
                5,
                "fnv1a64:0153f177915e4352",
            ),
            (
                "20260710_run_cost_integrity",
                "crates/trading/migrations/20260710_run_cost_integrity.sql",
                6,
                "fnv1a64:9371f25a7e6a392f",
            ),
            (
                "20260710_projection_job_protocol",
                "crates/trading/migrations/20260710_projection_job_protocol.sql",
                7,
                "fnv1a64:4d98f4d1d18160f8",
            ),
            (
                "20260710_run_cost_sources",
                "crates/trading/migrations/20260710_run_cost_sources.sql",
                8,
                "fnv1a64:1e74f494fa260aeb",
            ),
        ]
    );
    for migration in ordered() {
        assert_eq!(migration.checksum, checksum(migration.sql));
    }
    assert_eq!(latest(), &ordered()[3]);
}

#[test]
fn existing_history_must_match_every_immutable_field() {
    let migration = &ordered()[0];
    let applied = applied_row(migration);
    assert_eq!(
        disposition(migration, Some(&applied)),
        Ok(MigrationDisposition::Applied)
    );
    assert_eq!(
        disposition(migration, None),
        Ok(MigrationDisposition::Missing)
    );

    for (field, drifted) in [
        (
            "migration_id",
            AppliedMigration {
                id: "other".to_owned(),
                ..applied.clone()
            },
        ),
        (
            "schema_name",
            AppliedMigration {
                schema_name: "other".to_owned(),
                ..applied.clone()
            },
        ),
        (
            "schema_version",
            AppliedMigration {
                schema_version: 99,
                ..applied.clone()
            },
        ),
        (
            "checksum",
            AppliedMigration {
                checksum: "fnv1a64:0000000000000000".to_owned(),
                ..applied
            },
        ),
    ] {
        let error = disposition(migration, Some(&drifted)).unwrap_err();
        assert_eq!(error.reason, StorageDegradedReason::SchemaDrift);
        assert!(error.message.contains("schema migration drift"));
        assert!(error.message.contains(field));
    }
}

#[test]
fn history_sql_is_bootstrapped_and_never_rewritten() {
    assert!(BOOTSTRAP_SQL.contains("CREATE TABLE IF NOT EXISTS schema_migrations"));
    assert!(INSERT_MIGRATION_SQL.contains("ON CONFLICT (migration_id) DO NOTHING"));
    assert!(!INSERT_MIGRATION_SQL.contains("DO UPDATE"));
    assert!(!INSERT_MIGRATION_SQL.contains("UPDATE schema_migrations"));
}

#[test]
fn migration_lock_is_transaction_scoped_and_precedes_state_or_ddl() {
    assert_eq!(
        MIGRATION_ADVISORY_LOCK_SQL,
        "SELECT pg_advisory_xact_lock($1)"
    );
    assert_ne!(MIGRATION_ADVISORY_LOCK_KEY, 0);
    let apply_source = include_str!("../migrations.rs");
    let lock = apply_source
        .find(".query_one(MIGRATION_ADVISORY_LOCK_SQL")
        .unwrap();
    let bootstrap = apply_source.find(".batch_execute(BOOTSTRAP_SQL)").unwrap();
    let state_check = apply_source
        .find("apply_one(&transaction, migration")
        .unwrap();
    assert!(lock < bootstrap);
    assert!(bootstrap < state_check);
}

#[test]
fn base_fees_schema_has_one_run_id_column_before_freeze() {
    let fees = table_definition(BASE_SQL, "fees");
    assert_eq!(fees.matches("run_id").count(), 1);
}

#[test]
fn integrity_migration_contains_required_tables_keys_and_indexes() {
    for fragment in [
        "CREATE TABLE IF NOT EXISTS ledger_projection_jobs",
        "event_id        TEXT NOT NULL REFERENCES order_events(event_id)",
        "PRIMARY KEY (event_id, projector)",
        "CREATE INDEX IF NOT EXISTS idx_ledger_projection_jobs_pending_order",
        "ON ledger_projection_jobs(available_at_ms, event_id, projector)",
        "WHERE status = 'pending'",
        "attempt_count   INTEGER NOT NULL DEFAULT 0",
        "payload_hash    TEXT NOT NULL",
        "CREATE TABLE IF NOT EXISTS run_cost_facts",
        "PRIMARY KEY (run_kind, run_id, scope, component, event_id)",
        "CREATE INDEX IF NOT EXISTS idx_run_cost_facts_run_component_time",
        "CREATE INDEX IF NOT EXISTS idx_run_cost_facts_event",
        "amount_usd            DOUBLE PRECISION",
        "quality               TEXT NOT NULL",
        "occurred_at_ms        BIGINT NOT NULL",
        "captured_at_ms        BIGINT NOT NULL",
    ] {
        assert!(
            RUN_COST_INTEGRITY_SQL.contains(fragment),
            "missing {fragment}"
        );
    }
    let jobs = table_definition(RUN_COST_INTEGRITY_SQL, "ledger_projection_jobs");
    assert!(!jobs.contains("run_kind"));
    assert!(!jobs.contains("run_id"));
}

#[test]
fn projection_job_protocol_migration_adds_claim_guards_and_indexes() {
    for fragment in [
        "ADD COLUMN IF NOT EXISTS claim_token TEXT",
        "status IN ('pending', 'processing', 'completed')",
        "CHECK (attempt_count >= 0)",
        "ledger_projection_jobs_claim_state_check",
        "idx_ledger_projection_jobs_claim_pending",
        "ON ledger_projection_jobs(projector, available_at_ms, event_id)",
        "idx_ledger_projection_jobs_claim_expired",
        "ON ledger_projection_jobs(projector, claimed_at_ms, event_id)",
    ] {
        assert!(
            PROJECTION_JOB_PROTOCOL_SQL.contains(fragment),
            "missing {fragment}"
        );
    }
}

#[test]
fn run_cost_sources_migration_normalizes_sources_and_rebuild_receipts() {
    for fragment in [
        "DROP CONSTRAINT IF EXISTS run_cost_facts_event_id_fkey",
        "ADD COLUMN IF NOT EXISTS source_run_finality_event_id TEXT",
        "FOREIGN KEY (source_order_event_id)",
        "REFERENCES order_events(event_id)",
        "FOREIGN KEY (source_run_finality_event_id)",
        "REFERENCES run_finality_events(event_id)",
        "run_cost_facts_exactly_one_source_check",
        "CREATE TABLE IF NOT EXISTS run_finality_source_links",
        "PRIMARY KEY (run_finality_event_id)",
        "status IN ('intrinsic', 'unique', 'missing', 'unlinked', 'ambiguous')",
        "candidate_count       INTEGER NOT NULL",
        "CREATE TABLE IF NOT EXISTS run_cost_rebuild_receipts",
        "order_cursor         BIGINT NOT NULL DEFAULT 0",
        "finality_cursor      BIGINT NOT NULL DEFAULT 0",
        "order_high_water     BIGINT NOT NULL DEFAULT 0",
        "finality_high_water  BIGINT NOT NULL DEFAULT 0",
        "facts_written        BIGINT NOT NULL DEFAULT 0",
        "legacy_links_written BIGINT NOT NULL DEFAULT 0",
    ] {
        assert!(
            RUN_COST_SOURCES_SQL.contains(fragment),
            "missing {fragment}"
        );
    }
    assert!(!RUN_COST_SOURCES_SQL.contains("UPDATE order_events"));
    assert!(!RUN_COST_SOURCES_SQL.contains("UPDATE run_finality_events"));
    assert!(!RUN_COST_SOURCES_SQL.contains("DELETE FROM order_events"));
    assert!(!RUN_COST_SOURCES_SQL.contains("DELETE FROM run_finality_events"));
}

fn applied_row(migration: &Migration) -> AppliedMigration {
    AppliedMigration {
        id: migration.id.to_owned(),
        schema_name: migration.schema_name.to_owned(),
        schema_version: migration.schema_version,
        checksum: migration.checksum.to_owned(),
    }
}

fn table_definition<'a>(sql: &'a str, table: &str) -> &'a str {
    let start = sql
        .find(&format!("CREATE TABLE IF NOT EXISTS {table}"))
        .unwrap();
    let rest = &sql[start..];
    let end = rest.find(");").unwrap();
    &rest[..end]
}

fn checksum(sql: &str) -> String {
    let hash = sql
        .as_bytes()
        .iter()
        .fold(FNV64_OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(FNV64_PRIME)
        });
    format!("fnv1a64:{hash:016x}")
}
