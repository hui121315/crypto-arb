mod dispatch;
mod funding_diff;
mod funding_stats;
mod health;
mod memory;
mod postgres;
mod telemetry;
mod types;

#[cfg(test)]
mod tests;

use memory::MemoryHistoryStore;
use postgres::PostgresHistoryStore;
use shared_types::{HistoryMigrationStatus, StorageBackendKind, StorageDegradedReason};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use telemetry::HistoryStoreTelemetry;

pub use funding_stats::FundingDiffStatsProjector;
pub use health::HistoryStoreHealth;
pub use types::{
    history_migration_checksum_hex, ApiHealthQuery, ApiHealthSampleRow, EventQuery,
    FundingDiffQuery, FundingDiffRow, FundingDiffStatsQuery, FundingQuery, FundingRow,
    HistoryError, IndexCompositionHistoryRow, IndexCompositionQuery, LedgerEventRow,
    OpportunityQuery, OpportunityRow, HISTORY_SCHEMA_VERSION,
};

#[derive(Debug, Clone)]
pub struct HistoryStore {
    backend: HistoryBackend,
    telemetry: Arc<HistoryStoreTelemetry>,
}

#[derive(Debug, Clone)]
enum HistoryBackend {
    Memory(MemoryHistoryStore),
    Postgres(PostgresHistoryStore),
    Disabled,
}

impl Default for HistoryStore {
    fn default() -> Self {
        Self::new(100_000)
    }
}

impl HistoryStore {
    pub fn new(max_rows: usize) -> Self {
        Self::with_backend(
            HistoryBackend::Memory(MemoryHistoryStore::new(max_rows)),
            None,
            Some(HISTORY_SCHEMA_VERSION),
        )
    }

    pub fn disabled() -> Self {
        Self::with_backend(HistoryBackend::Disabled, None, None)
    }

    pub fn memory_fallback(startup_problem: impl Into<String>) -> Self {
        Self::with_backend(
            HistoryBackend::Memory(MemoryHistoryStore::new(100_000)),
            Some(startup_problem.into()),
            Some(HISTORY_SCHEMA_VERSION),
        )
    }

    pub async fn postgres(database_url: &str) -> Result<Self, HistoryError> {
        Ok(Self::with_backend(
            HistoryBackend::Postgres(PostgresHistoryStore::connect(database_url).await?),
            None,
            Some(HISTORY_SCHEMA_VERSION),
        ))
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend_kind().as_str()
    }

    pub fn backend_kind(&self) -> StorageBackendKind {
        match &self.backend {
            HistoryBackend::Memory(_) => StorageBackendKind::Memory,
            HistoryBackend::Postgres(_) => StorageBackendKind::Postgres,
            HistoryBackend::Disabled => StorageBackendKind::Disabled,
        }
    }

    pub async fn approximate_row_count(&self) -> Option<usize> {
        match &self.backend {
            HistoryBackend::Memory(memory) => Some(memory.row_count().await),
            HistoryBackend::Postgres(_) | HistoryBackend::Disabled => None,
        }
    }

    pub fn health_snapshot(&self, observed_at_ms: i64) -> HistoryStoreHealth {
        let last_success_at_ms =
            non_zero_ms(self.telemetry.last_success_at_ms.load(Ordering::Relaxed));
        let last_append_at_ms =
            non_zero_ms(self.telemetry.last_append_at_ms.load(Ordering::Relaxed));
        let last_query_at_ms = non_zero_ms(self.telemetry.last_query_at_ms.load(Ordering::Relaxed));
        let last_error_at_ms = non_zero_ms(self.telemetry.last_error_at_ms.load(Ordering::Relaxed));
        let durable = matches!(self.backend, HistoryBackend::Postgres(_));
        let enabled = !matches!(self.backend, HistoryBackend::Disabled);
        let (timescale_status, timescale_problem) = self.timescale_health();
        let migration_status = self.migration_status();
        let startup_problem = self
            .telemetry
            .startup_problem
            .load_full()
            .as_deref()
            .cloned();
        let last_error_reason = self
            .telemetry
            .last_error_reason
            .load_full()
            .as_deref()
            .copied();
        let storage_contract =
            health::history_storage_contract(health::HistoryStorageContractInput {
                backend_kind: self.backend_kind(),
                enabled,
                durable,
                fallback: startup_problem.is_some(),
                migration_authority: migration_status.clone(),
                timescale_status,
                last_error_reason,
                last_error_at_ms,
                last_success_at_ms,
            });
        HistoryStoreHealth {
            backend: self.backend_name(),
            storage_contract,
            enabled,
            durable,
            fallback: startup_problem.is_some(),
            ephemeral: enabled && !durable,
            schema_version: self.telemetry.schema_version(),
            migration_checksum: enabled.then(types::history_migration_checksum_hex),
            migration_status,
            startup_problem,
            timescale_status,
            timescale_problem,
            append_success_total: self.telemetry.append_success_total.load(Ordering::Relaxed),
            append_error_total: self.telemetry.append_error_total.load(Ordering::Relaxed),
            query_success_total: self.telemetry.query_success_total.load(Ordering::Relaxed),
            query_error_total: self.telemetry.query_error_total.load(Ordering::Relaxed),
            last_success_at_ms,
            last_append_at_ms,
            last_query_at_ms,
            last_error_at_ms,
            last_error: self.telemetry.last_error.load_full().as_deref().cloned(),
            last_error_code: self
                .telemetry
                .last_error_code
                .load_full()
                .as_deref()
                .cloned(),
            observed_at_ms,
        }
    }

    fn migration_status(&self) -> Option<HistoryMigrationStatus> {
        match &self.backend {
            HistoryBackend::Postgres(pg) => Some(pg.migration_status()),
            HistoryBackend::Memory(_) | HistoryBackend::Disabled => None,
        }
    }

    fn timescale_health(&self) -> (Option<shared_types::HistoryTimescaleStatus>, Option<String>) {
        match &self.backend {
            HistoryBackend::Postgres(pg) => (Some(pg.timescale_status()), pg.timescale_problem()),
            HistoryBackend::Memory(_) | HistoryBackend::Disabled => (None, None),
        }
    }

    fn with_backend(
        backend: HistoryBackend,
        startup_problem: Option<String>,
        schema_version: Option<u32>,
    ) -> Self {
        Self {
            backend,
            telemetry: Arc::new(HistoryStoreTelemetry::new(startup_problem, schema_version)),
        }
    }

    fn record_append<T>(&self, result: Result<T, HistoryError>) -> Result<T, HistoryError> {
        self.record_result(
            result,
            &self.telemetry.append_success_total,
            &self.telemetry.append_error_total,
            Some(&self.telemetry.last_append_at_ms),
            StorageDegradedReason::WriteFailed,
        )
    }

    fn record_query<T>(&self, result: Result<T, HistoryError>) -> Result<T, HistoryError> {
        self.record_result(
            result,
            &self.telemetry.query_success_total,
            &self.telemetry.query_error_total,
            Some(&self.telemetry.last_query_at_ms),
            StorageDegradedReason::ReadFailed,
        )
    }

    fn record_result<T>(
        &self,
        result: Result<T, HistoryError>,
        success: &AtomicU64,
        error: &AtomicU64,
        operation_success_at_ms: Option<&AtomicI64>,
        operation_error_reason: StorageDegradedReason,
    ) -> Result<T, HistoryError> {
        match &result {
            Ok(_) => self
                .telemetry
                .record_success(success, operation_success_at_ms),
            Err(err) => self
                .telemetry
                .record_error(error, err, operation_error_reason),
        }
        result
    }
}

fn non_zero_ms(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

fn diff_query(query: FundingDiffStatsQuery) -> FundingDiffQuery {
    FundingDiffQuery {
        symbol: query.symbol,
        long_exchange: query.long_exchange,
        short_exchange: query.short_exchange,
        from_ms: query.from_ms,
        to_ms: query.to_ms,
        limit: query.limit.clamp(1, 50_000),
    }
}
