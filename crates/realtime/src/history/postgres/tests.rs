use super::*;

mod live;

#[test]
fn timescale_health_marks_enabled_when_hypertables_all_succeed() {
    let errors: Vec<String> = Vec::new();
    let health = TimescaleSetupHealth::from_hypertable_errors(&errors);

    assert_eq!(health.status, HistoryTimescaleStatus::Enabled);
    assert_eq!(health.problem, None);
}

#[test]
fn timescale_health_marks_partial_when_hypertable_setup_fails() {
    let health = TimescaleSetupHealth::from_hypertable_errors(&["funding_rates: denied".into()]);

    assert_eq!(health.status, HistoryTimescaleStatus::Partial);
    assert!(health
        .problem
        .as_deref()
        .is_some_and(|problem| problem.contains("funding_rates")));
}

#[test]
fn timescale_health_marks_plain_postgres_when_extension_fails() {
    let health = TimescaleSetupHealth::plain_postgres("extension missing");

    assert_eq!(health.status, HistoryTimescaleStatus::PlainPostgres);
    assert_eq!(health.problem.as_deref(), Some("extension missing"));
}

#[test]
fn schema_drift_sql_states_are_classified() {
    assert!(is_history_schema_drift_state(&SqlState::UNDEFINED_TABLE));
    assert!(is_history_schema_drift_state(&SqlState::UNDEFINED_COLUMN));
    assert!(is_history_schema_drift_state(
        &SqlState::INVALID_COLUMN_REFERENCE
    ));
    assert!(!is_history_schema_drift_state(&SqlState::DUPLICATE_TABLE));
}

#[test]
fn postgres_backpressure_sql_states_are_classified() {
    assert!(is_history_backpressure_state(
        &SqlState::TOO_MANY_CONNECTIONS
    ));
    assert!(!is_history_backpressure_state(&SqlState::QUERY_CANCELED));
}

#[test]
fn runtime_schema_is_derived_from_authoritative_migration() {
    let runtime_schema = history_runtime_schema_sql();
    let migration = history_migration_sql();

    assert!(runtime_schema.contains("INSERT INTO history_meta"));
    assert!(runtime_schema.contains("durable schema; append-only"));
    assert!(!runtime_schema.contains("CREATE EXTENSION IF NOT EXISTS timescaledb"));
    assert!(!runtime_schema.contains("create_hypertable("));

    for table in schema_object_names(&runtime_schema, "CREATE TABLE IF NOT EXISTS") {
        assert!(
            migration.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
            "migration missing runtime table {table}"
        );
    }
    for index in schema_object_names(&runtime_schema, "CREATE INDEX IF NOT EXISTS") {
        assert!(
            migration.contains(&format!("CREATE INDEX IF NOT EXISTS {index}")),
            "migration missing runtime index {index}"
        );
    }
}

#[test]
fn migration_bootstrap_is_derived_from_authoritative_migration() {
    let bootstrap = history_migration_bootstrap_sql().expect("migration bootstrap");

    assert!(bootstrap.starts_with("CREATE TABLE IF NOT EXISTS schema_migrations"));
    assert!(!bootstrap.contains("history_meta"));
    assert!(!bootstrap.contains("create_hypertable("));
}

#[test]
fn applied_migration_history_is_insert_only() {
    assert!(INSERT_HISTORY_MIGRATION_SQL.contains("ON CONFLICT (migration_id) DO NOTHING"));
    assert!(!INSERT_HISTORY_MIGRATION_SQL.contains("DO UPDATE"));
    assert!(!INSERT_HISTORY_MIGRATION_SQL.contains("UPDATE schema_migrations"));
}

#[test]
fn applied_migration_identity_rejects_every_immutable_field_drift() {
    let applied = applied_history_migration();
    assert!(history_migration_status_from_applied(&applied).is_ok());

    for (field, drifted) in [
        (
            "migration_id",
            AppliedHistoryMigration {
                migration_id: "other".into(),
                ..applied.clone()
            },
        ),
        (
            "schema_name",
            AppliedHistoryMigration {
                schema_name: "other".into(),
                ..applied.clone()
            },
        ),
        (
            "schema_version",
            AppliedHistoryMigration {
                schema_version: 99,
                ..applied.clone()
            },
        ),
        (
            "checksum",
            AppliedHistoryMigration {
                checksum: "0000000000000000".into(),
                ..applied
            },
        ),
    ] {
        let error = history_migration_status_from_applied(&drifted)
            .expect_err("immutable migration drift must fail closed");
        match error {
            HistoryError::SchemaDrift(message) => assert!(message.contains(field)),
            other => panic!("expected schema drift for {field}, got {other}"),
        }
    }
}

#[test]
fn runtime_schema_keeps_plain_postgres_baseline_tables() {
    let schema = history_runtime_schema_sql();

    for table in HISTORY_RUNTIME_TABLES {
        assert!(
            schema.contains(&format!("CREATE TABLE IF NOT EXISTS {table}")),
            "runtime schema missing table {table}"
        );
    }
}

#[test]
fn query_expression_indexes_are_non_blocking_and_bounded() {
    assert!(QUERY_EXPRESSION_INDEXES
        .iter()
        .all(|(_, sql)| sql.starts_with("CREATE INDEX CONCURRENTLY IF NOT EXISTS")));
    assert_eq!(INDEX_LOCK_TIMEOUT, "2s");
    assert_eq!(INDEX_STATEMENT_TIMEOUT, "30s");
}

#[test]
fn timescale_hypertables_match_authoritative_migration() {
    let migration = history_migration_sql();
    let mut migration_tables: Vec<String> = migration
        .split("create_hypertable('")
        .skip(1)
        .filter_map(|rest| rest.split('\'').next())
        .map(str::to_owned)
        .collect();
    migration_tables.sort();

    let mut registered: Vec<String> = TIMESCALE_HYPERTABLES
        .iter()
        .map(|(table, _)| (*table).to_owned())
        .collect();
    registered.sort();

    assert_eq!(
        registered, migration_tables,
        "TIMESCALE_HYPERTABLES drifted from the authoritative migration"
    );
    for (table, sql) in TIMESCALE_HYPERTABLES {
        assert!(
            migration.contains(sql),
            "migration missing hypertable statement for {table}"
        );
    }
}

#[test]
fn history_migration_status_uses_authoritative_identity() {
    let status = history_migration_status(false, None, None);

    assert_eq!(status.migration_id, HISTORY_MIGRATION_ID);
    assert_eq!(status.schema_name, HISTORY_SCHEMA_NAME);
    assert_eq!(status.schema_version, None);
    assert!(!status.applied);
    assert_eq!(
        status.migration_checksum.as_deref(),
        Some(history_migration_checksum_hex().as_str())
    );
}

fn applied_history_migration() -> AppliedHistoryMigration {
    AppliedHistoryMigration {
        migration_id: HISTORY_MIGRATION_ID.into(),
        schema_name: HISTORY_SCHEMA_NAME.into(),
        schema_version: HISTORY_SCHEMA_VERSION as i32,
        checksum: history_migration_checksum_hex(),
        applied_at_ms: 1_700_000_000_000,
    }
}

fn schema_object_names(sql: &str, prefix: &str) -> Vec<String> {
    sql.lines()
        .filter_map(|line| line.trim().strip_prefix(prefix))
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|name| name.trim_end_matches('(').to_owned())
        .collect()
}
