use super::types::{
    history_migration_checksum_hex, history_migration_sql, history_migration_status,
    ApiHealthQuery, ApiHealthSampleRow, EventQuery, FundingDiffQuery, FundingDiffRow, FundingQuery,
    FundingRow, HistoryError, IndexCompositionHistoryRow, IndexCompositionQuery, LedgerEventRow,
    OpportunityQuery, OpportunityRow, HISTORY_MIGRATION_ID, HISTORY_SCHEMA_NAME,
    HISTORY_SCHEMA_VERSION,
};
use shared_types::{
    ArbitrageOpportunityDto, FundingRateData, HistoryMigrationStatus, HistoryTimescaleStatus,
    IndexCompositionSnapshot,
};
use std::sync::Arc;
use tokio_postgres::{error::SqlState, Client, Error as PgError, NoTls, Row};

mod append;
mod batches;
mod indexes;
mod inventory;
mod query;
mod rows;
mod schema;
#[cfg(test)]
mod tests;

use batches::*;
use indexes::spawn_query_expression_index_setup;
#[cfg(test)]
use indexes::{INDEX_LOCK_TIMEOUT, INDEX_STATEMENT_TIMEOUT, QUERY_EXPRESSION_INDEXES};
use rows::*;
use schema::*;

/// 数据读写路径的小连接池：轮转取用、断线懒重建。
///
/// 此前全部读写共享单条连接：慢查询（如 /api/history/* 大扫描）head-of-line
/// 阻塞 60s funding loop 的批量写入；且连接断开后没有任何重建逻辑，之后所有
/// history 读写持续报错直到进程重启。
///
/// 注意：迁移/初始化路径持有 Postgres advisory lock（会话级），必须固定在
/// 单条连接上执行，因此 `PostgresHistoryStore::client` 原样保留给初始化专用，
/// 数据路径一律走 [`HistoryClientPool::client`]。
#[derive(Debug)]
pub(super) struct HistoryClientPool {
    database_url: String,
    slots: Vec<tokio::sync::Mutex<Option<Arc<Client>>>>,
    cursor: std::sync::atomic::AtomicUsize,
}

const HISTORY_POOL_SIZE: usize = 3;

impl HistoryClientPool {
    fn new(database_url: String, size: usize) -> Self {
        Self {
            database_url,
            slots: (0..size.max(1))
                .map(|_| tokio::sync::Mutex::new(None))
                .collect(),
            cursor: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    pub(super) async fn client(&self) -> Result<Arc<Client>, HistoryError> {
        let index = self
            .cursor
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % self.slots.len();
        let mut slot = self.slots[index].lock().await;
        if let Some(client) = slot.as_ref() {
            if !client.is_closed() {
                return Ok(Arc::clone(client));
            }
        }
        let (client, connection) = tokio_postgres::connect(&self.database_url, NoTls)
            .await
            .map_err(|e| HistoryError::Unavailable(e.to_string()))?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "postgres history pooled connection closed");
            }
        });
        let client = Arc::new(client);
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }
}

#[derive(Debug, Clone)]
pub(super) struct PostgresHistoryStore {
    /// 初始化/迁移专用（advisory lock 会话绑定）；数据路径走 `pool`。
    client: Arc<Client>,
    pool: Arc<HistoryClientPool>,
    timescale: TimescaleSetupHealth,
    migration_status: HistoryMigrationStatus,
}

impl PostgresHistoryStore {
    pub(super) async fn connect(database_url: &str) -> Result<Self, HistoryError> {
        let (client, connection) = tokio_postgres::connect(database_url, NoTls)
            .await
            .map_err(|e| HistoryError::Unavailable(e.to_string()))?;
        tokio::spawn(async move {
            if let Err(error) = connection.await {
                tracing::warn!(%error, "postgres history connection closed");
            }
        });
        let mut store = Self {
            client: Arc::new(client),
            pool: Arc::new(HistoryClientPool::new(
                database_url.to_owned(),
                HISTORY_POOL_SIZE,
            )),
            timescale: TimescaleSetupHealth::not_applicable(),
            migration_status: history_migration_status(false, None, None),
        };
        let init = store.initialize().await?;
        store.timescale = init.timescale;
        store.migration_status = init.migration;
        spawn_query_expression_index_setup(database_url.to_owned());
        Ok(store)
    }

    async fn initialize(&self) -> Result<PostgresInitHealth, HistoryError> {
        self.client
            .query_one(
                HISTORY_MIGRATION_ADVISORY_LOCK_SQL,
                &[&HISTORY_MIGRATION_ADVISORY_LOCK_KEY],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        let result = self.initialize_locked().await;
        let release = self.release_migration_lock().await;
        match (result, release) {
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(error),
            (Ok(health), Ok(())) => Ok(health),
        }
    }

    async fn initialize_locked(&self) -> Result<PostgresInitHealth, HistoryError> {
        let presence = self.load_schema_presence().await?;
        let existing_schema = presence.history_meta || presence.runtime_tables;
        if existing_schema && !presence.migration_table {
            return Err(HistoryError::SchemaDrift(
                "history schema exists without schema_migrations authority".into(),
            ));
        }
        if !presence.migration_table {
            let bootstrap_sql = history_migration_bootstrap_sql()?;
            self.client
                .batch_execute(&bootstrap_sql)
                .await
                .map_err(|error| postgres_history_error(&error))?;
        }
        let migration = match self.load_schema_migration().await? {
            Some(row) => history_migration_status_from_row(&row)?,
            None if existing_schema => {
                return Err(HistoryError::SchemaDrift(
                    "history schema exists without an applied migration row".into(),
                ));
            }
            None => {
                let schema_sql = history_runtime_schema_sql();
                self.client
                    .batch_execute(&schema_sql)
                    .await
                    .map_err(|error| postgres_history_error(&error))?;
                self.record_schema_migration().await?
            }
        };
        self.validate_schema_version().await?;
        self.validate_runtime_tables().await?;
        let timescale = self.setup_timescale().await;
        Ok(PostgresInitHealth {
            timescale,
            migration,
        })
    }

    async fn release_migration_lock(&self) -> Result<(), HistoryError> {
        let row = self
            .client
            .query_one(
                HISTORY_MIGRATION_ADVISORY_UNLOCK_SQL,
                &[&HISTORY_MIGRATION_ADVISORY_LOCK_KEY],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        let released: bool = row.try_get(0).map_err(|error| {
            HistoryError::Unavailable(format!(
                "history migration advisory unlock result is invalid: {error}"
            ))
        })?;
        if released {
            Ok(())
        } else {
            Err(HistoryError::Unavailable(
                "history migration advisory lock was not held at release".into(),
            ))
        }
    }

    async fn validate_schema_version(&self) -> Result<(), HistoryError> {
        let schema_version = i64::from(HISTORY_SCHEMA_VERSION);
        let row = self
            .client
            .query_one(
                "SELECT value FROM history_meta WHERE key = 'schema_version'",
                &[],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        let stored: i64 = row.try_get("value").map_err(|error| {
            HistoryError::SchemaDrift(format!(
                "history schema version marker is missing or invalid: {error}"
            ))
        })?;
        if stored == schema_version {
            Ok(())
        } else {
            Err(HistoryError::SchemaDrift(format!(
                "history schema version mismatch: expected {schema_version}, found {stored}"
            )))
        }
    }

    pub(super) fn timescale_status(&self) -> HistoryTimescaleStatus {
        self.timescale.status
    }

    pub(super) fn timescale_problem(&self) -> Option<String> {
        self.timescale.problem.clone()
    }

    pub(super) fn migration_status(&self) -> HistoryMigrationStatus {
        self.migration_status.clone()
    }

    async fn record_schema_migration(&self) -> Result<HistoryMigrationStatus, HistoryError> {
        let applied_at_ms = common::time::now_ms();
        let checksum = history_migration_checksum_hex();
        let schema_version = HISTORY_SCHEMA_VERSION as i32;
        self.client
            .execute(
                INSERT_HISTORY_MIGRATION_SQL,
                &[
                    &HISTORY_MIGRATION_ID,
                    &HISTORY_SCHEMA_NAME,
                    &schema_version,
                    &checksum,
                    &applied_at_ms,
                ],
            )
            .await
            .map_err(|error| postgres_history_error(&error))?;
        let row = self.load_schema_migration().await?.ok_or_else(|| {
            HistoryError::SchemaDrift(
                "history migration insert completed without an applied row".into(),
            )
        })?;
        history_migration_status_from_row(&row)
    }

    async fn load_schema_migration(&self) -> Result<Option<Row>, HistoryError> {
        self.client
            .query_opt(LOAD_HISTORY_MIGRATION_SQL, &[&HISTORY_MIGRATION_ID])
            .await
            .map_err(|error| postgres_history_error(&error))
    }

    async fn setup_timescale(&self) -> TimescaleSetupHealth {
        if let Err(problem) = self.enable_timescale_extension().await {
            return TimescaleSetupHealth::plain_postgres(problem);
        }
        self.create_timescale_hypertables().await
    }

    async fn enable_timescale_extension(&self) -> Result<(), String> {
        let result = self
            .client
            .batch_execute("CREATE EXTENSION IF NOT EXISTS timescaledb;")
            .await;
        result.map_err(|error| {
            tracing::warn!(
                %error,
                "timescaledb extension unavailable; history store will use plain postgres tables"
            );
            error.to_string()
        })
    }

    async fn create_timescale_hypertables(&self) -> TimescaleSetupHealth {
        let mut errors = Vec::new();
        for (table, sql) in TIMESCALE_HYPERTABLES {
            if let Err(error) = self.client.batch_execute(sql).await {
                tracing::warn!(
                    %error,
                    table,
                    "timescaledb hypertable setup skipped for one history table"
                );
                errors.push(format!("{table}: {error}"));
            }
        }
        TimescaleSetupHealth::from_hypertable_errors(&errors)
    }
}
