use shared_types::{
    HistoryBackendStatus, HistoryMigrationStatus, HistoryTimescaleStatus, StorageBackendKind,
    StorageDegradedReason, StorageRuntimeContract,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryStoreHealth {
    pub backend: &'static str,
    pub storage_contract: StorageRuntimeContract,
    pub enabled: bool,
    pub durable: bool,
    pub fallback: bool,
    pub ephemeral: bool,
    pub schema_version: Option<u32>,
    pub migration_checksum: Option<String>,
    pub migration_status: Option<HistoryMigrationStatus>,
    pub startup_problem: Option<String>,
    pub timescale_status: Option<HistoryTimescaleStatus>,
    pub timescale_problem: Option<String>,
    pub append_success_total: u64,
    pub append_error_total: u64,
    pub query_success_total: u64,
    pub query_error_total: u64,
    pub last_success_at_ms: Option<i64>,
    pub last_append_at_ms: Option<i64>,
    pub last_query_at_ms: Option<i64>,
    pub last_error_at_ms: Option<i64>,
    pub last_error: Option<String>,
    pub last_error_code: Option<String>,
    pub observed_at_ms: i64,
}

impl HistoryStoreHealth {
    pub fn success_total(&self) -> u64 {
        self.append_success_total
            .saturating_add(self.query_success_total)
    }

    pub fn error_total(&self) -> u64 {
        self.append_error_total
            .saturating_add(self.query_error_total)
    }

    pub fn backend_status(&self) -> HistoryBackendStatus {
        HistoryBackendStatus {
            backend: self.backend.to_owned(),
            storage_contract: self.storage_contract.clone(),
            enabled: self.enabled,
            durable: self.durable,
            fallback: self.fallback,
            ephemeral: self.ephemeral,
            schema_version: self.schema_version,
            migration_checksum: self.migration_checksum.clone(),
            migration_status: self.migration_status.clone(),
            startup_problem: self.startup_problem.clone(),
            timescale_status: self.timescale_status,
            timescale_problem: self.timescale_problem.clone(),
            append_success_total: self.append_success_total,
            append_error_total: self.append_error_total,
            query_success_total: self.query_success_total,
            query_error_total: self.query_error_total,
            last_success_at_ms: self.last_success_at_ms,
            last_append_at_ms: self.last_append_at_ms,
            last_query_at_ms: self.last_query_at_ms,
            last_error_at_ms: self.last_error_at_ms,
            last_error: self.last_error.clone(),
            last_error_code: self.last_error_code.clone(),
            observed_at_ms: self.observed_at_ms,
        }
    }

    pub fn has_degraded_reason(&self, reason: StorageDegradedReason) -> bool {
        self.storage_contract.degraded_reasons.contains(&reason)
    }
}

pub(super) struct HistoryStorageContractInput {
    pub backend_kind: StorageBackendKind,
    pub enabled: bool,
    pub durable: bool,
    pub fallback: bool,
    pub migration_authority: Option<HistoryMigrationStatus>,
    pub timescale_status: Option<HistoryTimescaleStatus>,
    pub last_error_reason: Option<StorageDegradedReason>,
    pub last_error_at_ms: Option<i64>,
    pub last_success_at_ms: Option<i64>,
}

pub(super) fn history_storage_contract(
    input: HistoryStorageContractInput,
) -> StorageRuntimeContract {
    let mut degraded_reasons = Vec::with_capacity(5);
    if !input.enabled {
        degraded_reasons.push(StorageDegradedReason::Disabled);
    } else if !input.durable {
        degraded_reasons.push(StorageDegradedReason::Ephemeral);
    }
    if input.fallback {
        degraded_reasons.push(StorageDegradedReason::Fallback);
    }
    if input.enabled
        && input.durable
        && !input
            .migration_authority
            .as_ref()
            .is_some_and(|authority| authority.applied)
    {
        degraded_reasons.push(StorageDegradedReason::MigrationUnapplied);
    }
    if matches!(
        input.timescale_status,
        Some(HistoryTimescaleStatus::PlainPostgres | HistoryTimescaleStatus::Partial)
    ) {
        degraded_reasons.push(StorageDegradedReason::ExtensionDegraded);
    }
    let error_recovered = match (input.last_error_at_ms, input.last_success_at_ms) {
        (Some(error), Some(success)) => success >= error,
        _ => false,
    };
    if !error_recovered {
        if let Some(reason) = input.last_error_reason {
            if !degraded_reasons.contains(&reason) {
                degraded_reasons.push(reason);
            }
        }
    }
    StorageRuntimeContract {
        backend_kind: input.backend_kind,
        degraded_reasons,
        migration_authority: input.migration_authority,
    }
}
