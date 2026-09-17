use super::*;

pub(super) const INSERT_HISTORY_MIGRATION_SQL: &str = "INSERT INTO schema_migrations \
     (migration_id, schema_name, schema_version, checksum, applied_at_ms) \
     VALUES ($1, $2, $3, $4, $5) \
     ON CONFLICT (migration_id) DO NOTHING";
pub(super) const LOAD_HISTORY_MIGRATION_SQL: &str =
    "SELECT migration_id, schema_name, schema_version, checksum, applied_at_ms \
     FROM schema_migrations WHERE migration_id = $1";
pub(super) const HISTORY_MIGRATION_ADVISORY_LOCK_SQL: &str = "SELECT pg_advisory_lock($1)";
pub(super) const HISTORY_MIGRATION_ADVISORY_UNLOCK_SQL: &str = "SELECT pg_advisory_unlock($1)";
pub(super) const HISTORY_MIGRATION_ADVISORY_LOCK_KEY: i64 = 0x4352_4f53_5348_4953;
pub(super) const HISTORY_RUNTIME_TABLES: [&str; 16] = [
    "history_meta",
    "schema_migrations",
    "funding_rates",
    "funding_diffs",
    "opportunities",
    "index_compositions",
    "watchlist",
    "alert_rules",
    "api_health",
    "events",
    "executions",
    "orders",
    "fills",
    "fees",
    "funding_payments",
    "balances",
];

pub(super) fn postgres_history_error(error: &PgError) -> HistoryError {
    match error.code() {
        Some(code) if is_history_schema_drift_state(code) => {
            HistoryError::SchemaDrift(error.to_string())
        }
        Some(code) if is_history_backpressure_state(code) => {
            HistoryError::RateLimited(error.to_string())
        }
        Some(code) if code == &SqlState::QUERY_CANCELED => {
            HistoryError::QueryCanceled(error.to_string())
        }
        _ => HistoryError::Unavailable(error.to_string()),
    }
}

pub(super) fn history_migration_status_from_row(
    row: &Row,
) -> Result<HistoryMigrationStatus, HistoryError> {
    let applied = AppliedHistoryMigration {
        migration_id: history_migration_column(row, "migration_id")?,
        schema_name: history_migration_column(row, "schema_name")?,
        schema_version: history_migration_column(row, "schema_version")?,
        checksum: history_migration_column(row, "checksum")?,
        applied_at_ms: history_migration_column(row, "applied_at_ms")?,
    };
    history_migration_status_from_applied(&applied)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AppliedHistoryMigration {
    pub(super) migration_id: String,
    pub(super) schema_name: String,
    pub(super) schema_version: i32,
    pub(super) checksum: String,
    pub(super) applied_at_ms: i64,
}

pub(super) fn history_migration_status_from_applied(
    applied: &AppliedHistoryMigration,
) -> Result<HistoryMigrationStatus, HistoryError> {
    let expected_version = HISTORY_SCHEMA_VERSION as i32;
    let expected_checksum = history_migration_checksum_hex();

    if applied.migration_id != HISTORY_MIGRATION_ID {
        return Err(history_migration_field_drift(
            "migration_id",
            HISTORY_MIGRATION_ID,
            &applied.migration_id,
        ));
    }
    if applied.schema_name != HISTORY_SCHEMA_NAME {
        return Err(history_migration_field_drift(
            "schema_name",
            HISTORY_SCHEMA_NAME,
            &applied.schema_name,
        ));
    }
    if applied.schema_version != expected_version {
        return Err(history_migration_field_drift(
            "schema_version",
            expected_version,
            applied.schema_version,
        ));
    }
    if applied.checksum != expected_checksum {
        return Err(history_migration_field_drift(
            "checksum",
            expected_checksum,
            &applied.checksum,
        ));
    }
    Ok(history_migration_status(
        true,
        Some(HISTORY_SCHEMA_VERSION),
        Some(applied.applied_at_ms),
    ))
}

fn history_migration_column<'a, T>(row: &'a Row, column: &str) -> Result<T, HistoryError>
where
    T: tokio_postgres::types::FromSql<'a>,
{
    row.try_get(column).map_err(|error| {
        HistoryError::SchemaDrift(format!(
            "history migration column {column} is missing or invalid: {error}"
        ))
    })
}

fn history_migration_field_drift(
    field: &str,
    expected: impl std::fmt::Display,
    actual: impl std::fmt::Display,
) -> HistoryError {
    HistoryError::SchemaDrift(format!(
        "history migration row mismatch for {field}: expected {expected}, found {actual}"
    ))
}

pub(super) fn is_history_schema_drift_state(state: &SqlState) -> bool {
    state == &SqlState::UNDEFINED_TABLE
        || state == &SqlState::UNDEFINED_COLUMN
        || state == &SqlState::INVALID_COLUMN_REFERENCE
}

// PostgreSQL Appendix A SQLSTATE evidence:
// https://www.postgresql.org/docs/16/errcodes-appendix.html
pub(super) fn is_history_backpressure_state(state: &SqlState) -> bool {
    state == &SqlState::TOO_MANY_CONNECTIONS
}

pub(super) fn history_runtime_schema_sql() -> String {
    history_migration_sql()
        .lines()
        .filter(|line| is_runtime_schema_line(line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn history_migration_bootstrap_sql() -> Result<String, HistoryError> {
    let migration = history_migration_sql();
    let marker = "CREATE TABLE IF NOT EXISTS schema_migrations";
    let start = migration.find(marker).ok_or_else(|| {
        HistoryError::SchemaDrift(
            "authoritative history migration is missing schema_migrations bootstrap".into(),
        )
    })?;
    let statement = &migration[start..];
    let end = statement.find(");").ok_or_else(|| {
        HistoryError::SchemaDrift(
            "authoritative history migration has an incomplete schema_migrations bootstrap".into(),
        )
    })?;
    Ok(statement[..end + 2].to_owned())
}

pub(super) fn is_runtime_schema_line(line: &str) -> bool {
    let normalized = line.trim().to_ascii_lowercase();
    !normalized.starts_with("create extension if not exists timescaledb")
        && !normalized.starts_with("select create_hypertable(")
}

pub(super) const TIMESCALE_HYPERTABLES: [(&str, &str); 12] = [
    (
        "funding_rates",
        "SELECT create_hypertable('funding_rates', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "funding_diffs",
        "SELECT create_hypertable('funding_diffs', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "opportunities",
        "SELECT create_hypertable('opportunities', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "index_compositions",
        "SELECT create_hypertable('index_compositions', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "api_health",
        "SELECT create_hypertable('api_health', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "events",
        "SELECT create_hypertable('events', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "executions",
        "SELECT create_hypertable('executions', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "orders",
        "SELECT create_hypertable('orders', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "fills",
        "SELECT create_hypertable('fills', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "fees",
        "SELECT create_hypertable('fees', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "funding_payments",
        "SELECT create_hypertable('funding_payments', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
    (
        "balances",
        "SELECT create_hypertable('balances', 'occurred_at_ms', chunk_time_interval => 86400000, if_not_exists => TRUE);",
    ),
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PostgresInitHealth {
    pub(super) timescale: TimescaleSetupHealth,
    pub(super) migration: HistoryMigrationStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TimescaleSetupHealth {
    pub(super) status: HistoryTimescaleStatus,
    pub(super) problem: Option<String>,
}

impl TimescaleSetupHealth {
    pub(super) fn not_applicable() -> Self {
        Self {
            status: HistoryTimescaleStatus::NotApplicable,
            problem: None,
        }
    }

    pub(super) fn plain_postgres(problem: impl Into<String>) -> Self {
        Self {
            status: HistoryTimescaleStatus::PlainPostgres,
            problem: Some(problem.into()),
        }
    }

    pub(super) fn from_hypertable_errors(errors: &[String]) -> Self {
        if errors.is_empty() {
            return Self {
                status: HistoryTimescaleStatus::Enabled,
                problem: None,
            };
        }
        Self {
            status: HistoryTimescaleStatus::Partial,
            problem: Some(errors.join("; ")),
        }
    }
}
