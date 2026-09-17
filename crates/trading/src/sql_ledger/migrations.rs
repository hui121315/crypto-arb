use shared_types::StorageDegradedReason;
use tokio_postgres::{Client, Row};

const SCHEMA_NAME: &str = "trading_order_ledger";
const MIGRATION_ADVISORY_LOCK_KEY: i64 = 0x4352_4f53_534c_494e;
const MIGRATION_ADVISORY_LOCK_SQL: &str = "SELECT pg_advisory_xact_lock($1)";
const BOOTSTRAP_SQL: &str = "\
CREATE TABLE IF NOT EXISTS schema_migrations (\
    migration_id   TEXT PRIMARY KEY,\
    schema_name    TEXT NOT NULL,\
    schema_version INTEGER NOT NULL,\
    checksum       TEXT NOT NULL,\
    applied_at_ms  BIGINT NOT NULL,\
    applied_at     TIMESTAMPTZ NOT NULL DEFAULT NOW()\
);";
const LOAD_MIGRATION_SQL: &str = "\
SELECT migration_id, schema_name, schema_version, checksum \
FROM schema_migrations WHERE migration_id = $1";
const INSERT_MIGRATION_SQL: &str = "\
INSERT INTO schema_migrations \
    (migration_id, schema_name, schema_version, checksum, applied_at_ms) \
VALUES ($1, $2, $3, $4, $5) \
ON CONFLICT (migration_id) DO NOTHING";

const BASE_SQL: &str = include_str!("../../migrations/20260601_orders.sql");
const RUN_COST_INTEGRITY_SQL: &str =
    include_str!("../../migrations/20260710_run_cost_integrity.sql");
const PROJECTION_JOB_PROTOCOL_SQL: &str =
    include_str!("../../migrations/20260710_projection_job_protocol.sql");
const RUN_COST_SOURCES_SQL: &str = include_str!("../../migrations/20260710_run_cost_sources.sql");

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Migration {
    pub(super) id: &'static str,
    pub(super) path: &'static str,
    pub(super) schema_name: &'static str,
    pub(super) schema_version: i32,
    pub(super) checksum: &'static str,
    pub(super) sql: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct AppliedMigration {
    pub(super) id: String,
    pub(super) schema_name: String,
    pub(super) schema_version: i32,
    pub(super) checksum: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MigrationDisposition {
    Missing,
    Applied,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MigrationFailure {
    pub(super) reason: StorageDegradedReason,
    pub(super) message: String,
}

impl MigrationFailure {
    pub(super) fn unavailable(message: impl Into<String>) -> Self {
        Self {
            reason: StorageDegradedReason::Unavailable,
            message: message.into(),
        }
    }

    pub(super) fn migration(message: impl Into<String>) -> Self {
        Self {
            reason: StorageDegradedReason::MigrationFailed,
            message: message.into(),
        }
    }

    fn schema_drift(message: impl Into<String>) -> Self {
        Self {
            reason: StorageDegradedReason::SchemaDrift,
            message: message.into(),
        }
    }
}

const MIGRATIONS: [Migration; 4] = [
    Migration {
        id: "20260601_orders",
        path: "crates/trading/migrations/20260601_orders.sql",
        schema_name: SCHEMA_NAME,
        schema_version: 5,
        checksum: "fnv1a64:0153f177915e4352",
        sql: BASE_SQL,
    },
    Migration {
        id: "20260710_run_cost_integrity",
        path: "crates/trading/migrations/20260710_run_cost_integrity.sql",
        schema_name: SCHEMA_NAME,
        schema_version: 6,
        checksum: "fnv1a64:9371f25a7e6a392f",
        sql: RUN_COST_INTEGRITY_SQL,
    },
    Migration {
        id: "20260710_projection_job_protocol",
        path: "crates/trading/migrations/20260710_projection_job_protocol.sql",
        schema_name: SCHEMA_NAME,
        schema_version: 7,
        checksum: "fnv1a64:4d98f4d1d18160f8",
        sql: PROJECTION_JOB_PROTOCOL_SQL,
    },
    Migration {
        id: "20260710_run_cost_sources",
        path: "crates/trading/migrations/20260710_run_cost_sources.sql",
        schema_name: SCHEMA_NAME,
        schema_version: 8,
        checksum: "fnv1a64:1e74f494fa260aeb",
        sql: RUN_COST_SOURCES_SQL,
    },
];

pub(super) const fn ordered() -> &'static [Migration] {
    &MIGRATIONS
}

pub(super) const fn latest() -> &'static Migration {
    &MIGRATIONS[MIGRATIONS.len() - 1]
}

pub(super) async fn apply_all(
    client: &mut Client,
    applied_at_ms: i64,
) -> Result<(), MigrationFailure> {
    let transaction = client.transaction().await.map_err(|error| {
        MigrationFailure::migration(format!(
            "schema migration transaction start failed: {error}"
        ))
    })?;
    transaction
        .query_one(MIGRATION_ADVISORY_LOCK_SQL, &[&MIGRATION_ADVISORY_LOCK_KEY])
        .await
        .map_err(|error| {
            MigrationFailure::migration(format!("schema migration advisory lock failed: {error}"))
        })?;
    transaction
        .batch_execute(BOOTSTRAP_SQL)
        .await
        .map_err(|error| {
            MigrationFailure::migration(format!("schema_migrations bootstrap failed: {error}"))
        })?;

    for migration in ordered() {
        apply_one(&transaction, migration, applied_at_ms).await?;
    }
    transaction.commit().await.map_err(|error| {
        MigrationFailure::migration(format!(
            "schema migration transaction commit failed: {error}"
        ))
    })
}

async fn apply_one<C>(
    client: &C,
    migration: &'static Migration,
    applied_at_ms: i64,
) -> Result<(), MigrationFailure>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let applied = load_applied(client, migration.id).await?;
    if disposition(migration, applied.as_ref())? == MigrationDisposition::Applied {
        return Ok(());
    }

    client
        .batch_execute(migration.sql)
        .await
        .map_err(|error| migration_error(migration, "apply", &error))?;
    client
        .execute(
            INSERT_MIGRATION_SQL,
            &[
                &migration.id,
                &migration.schema_name,
                &migration.schema_version,
                &migration.checksum,
                &applied_at_ms,
            ],
        )
        .await
        .map_err(|error| migration_error(migration, "history insert", &error))?;

    let applied = load_applied(client, migration.id).await?;
    if disposition(migration, applied.as_ref())? != MigrationDisposition::Applied {
        return Err(MigrationFailure::migration(format!(
            "schema migration {} history insert did not persist",
            migration.id
        )));
    }
    Ok(())
}

async fn load_applied<C>(
    client: &C,
    migration_id: &str,
) -> Result<Option<AppliedMigration>, MigrationFailure>
where
    C: tokio_postgres::GenericClient + Sync,
{
    let row = client
        .query_opt(LOAD_MIGRATION_SQL, &[&migration_id])
        .await
        .map_err(|error| {
            MigrationFailure::migration(format!(
                "schema migration history query failed for {migration_id}: {error}"
            ))
        })?;
    row.as_ref().map(applied_from_row).transpose()
}

fn applied_from_row(row: &Row) -> Result<AppliedMigration, MigrationFailure> {
    Ok(AppliedMigration {
        id: migration_column(row, "migration_id")?,
        schema_name: migration_column(row, "schema_name")?,
        schema_version: row
            .try_get("schema_version")
            .map_err(|error| migration_column_error("schema_version", &error))?,
        checksum: migration_column(row, "checksum")?,
    })
}

fn migration_column(row: &Row, column: &str) -> Result<String, MigrationFailure> {
    row.try_get(column)
        .map_err(|error| migration_column_error(column, &error))
}

fn migration_column_error(column: &str, error: &tokio_postgres::Error) -> MigrationFailure {
    MigrationFailure::schema_drift(format!(
        "schema migration history column {column} is invalid: {error}"
    ))
}

pub(super) fn disposition(
    expected: &Migration,
    applied: Option<&AppliedMigration>,
) -> Result<MigrationDisposition, MigrationFailure> {
    let Some(applied) = applied else {
        return Ok(MigrationDisposition::Missing);
    };
    verify_field(expected, "migration_id", expected.id, applied.id.as_str())?;
    verify_field(
        expected,
        "schema_name",
        expected.schema_name,
        applied.schema_name.as_str(),
    )?;
    if expected.schema_version != applied.schema_version {
        return Err(MigrationFailure::schema_drift(format!(
            "schema migration drift for {}: schema_version expected {}, found {}",
            expected.id, expected.schema_version, applied.schema_version
        )));
    }
    verify_field(
        expected,
        "checksum",
        expected.checksum,
        applied.checksum.as_str(),
    )?;
    Ok(MigrationDisposition::Applied)
}

fn verify_field(
    expected: &Migration,
    field: &str,
    expected_value: &str,
    applied_value: &str,
) -> Result<(), MigrationFailure> {
    if expected_value == applied_value {
        return Ok(());
    }
    Err(MigrationFailure::schema_drift(format!(
        "schema migration drift for {}: {field} expected {expected_value:?}, found {applied_value:?}",
        expected.id
    )))
}

fn migration_error(
    migration: &Migration,
    operation: &str,
    error: &tokio_postgres::Error,
) -> MigrationFailure {
    MigrationFailure::migration(format!(
        "schema migration {} ({}) {operation} failed: {error}",
        migration.id, migration.path
    ))
}

#[cfg(test)]
#[path = "migrations/tests.rs"]
mod tests;
