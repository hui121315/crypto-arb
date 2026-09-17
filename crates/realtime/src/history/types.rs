pub type ApiHealthSampleRow = shared_types::ApiHealthSampleRow;
pub type FundingDiffRow = shared_types::FundingDiffRow;
pub type FundingRow = shared_types::FundingRow;
pub type IndexCompositionHistoryRow = shared_types::IndexCompositionHistoryRow;
pub type LedgerEventRow = shared_types::LedgerEventRow;
pub type OpportunityRow = shared_types::OpportunityHistoryRow;

use shared_types::{HistoryMigrationStatus, StorageDegradedReason};
use std::collections::VecDeque;
use thiserror::Error;

pub const HISTORY_SCHEMA_VERSION: u32 = 2;
pub(super) const HISTORY_MIGRATION_ID: &str = "20260701_history";
pub(super) const HISTORY_MIGRATION_PATH: &str = "crates/realtime/migrations/20260701_history.sql";
pub(super) const HISTORY_SCHEMA_NAME: &str = "realtime_history";
const HISTORY_MIGRATION_SQL: &str = include_str!("../../migrations/20260701_history.sql");
const FNV64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV64_PRIME: u64 = 0x100000001b3;

const HISTORY_MIGRATION_CHECKSUM: u64 = fnv1a64(HISTORY_MIGRATION_SQL.as_bytes());

pub fn history_migration_checksum_hex() -> String {
    format!("{HISTORY_MIGRATION_CHECKSUM:016x}")
}

pub(super) fn history_migration_sql() -> &'static str {
    HISTORY_MIGRATION_SQL
}

pub(super) fn history_migration_status(
    applied: bool,
    schema_version: Option<u32>,
    applied_at_ms: Option<i64>,
) -> HistoryMigrationStatus {
    HistoryMigrationStatus {
        migration_id: HISTORY_MIGRATION_ID.to_owned(),
        schema_name: HISTORY_SCHEMA_NAME.to_owned(),
        migration_path: HISTORY_MIGRATION_PATH.to_owned(),
        schema_version,
        migration_checksum: Some(history_migration_checksum_hex()),
        applied,
        applied_at_ms,
    }
}

#[derive(Debug, Clone, Default)]
pub struct FundingQuery {
    pub symbol: Option<String>,
    pub exchange: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct FundingDiffQuery {
    pub symbol: Option<String>,
    pub long_exchange: Option<String>,
    pub short_exchange: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct FundingDiffStatsQuery {
    pub symbol: Option<String>,
    pub long_exchange: Option<String>,
    pub short_exchange: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct OpportunityQuery {
    pub symbol: Option<String>,
    pub min_yield: Option<f64>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct IndexCompositionQuery {
    pub venue: Option<String>,
    pub symbol: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
}

#[derive(Debug, Clone, Default)]
pub struct ApiHealthQuery {
    pub exchange: Option<String>,
    pub endpoint: Option<String>,
    pub outcome: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
}

/// Correlation-aware filter over the unified event/audit ledger. Any subset of
/// the correlation chain (`request_id` / `run_id` / `ticket_id` /
/// `client_order_id` / `exchange_order_id`) can be supplied to replay and join
/// related events.
#[derive(Debug, Clone, Default)]
pub struct EventQuery {
    pub category: Option<String>,
    pub action: Option<String>,
    pub request_id: Option<String>,
    pub run_id: Option<String>,
    pub ticket_id: Option<String>,
    pub client_order_id: Option<String>,
    pub exchange_order_id: Option<String>,
    pub from_ms: Option<i64>,
    pub to_ms: Option<i64>,
    pub limit: usize,
}

const fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = FNV64_OFFSET_BASIS;
    let mut index = 0;
    while index < bytes.len() {
        hash ^= bytes[index] as u64;
        hash = hash.wrapping_mul(FNV64_PRIME);
        index += 1;
    }
    hash
}

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("history store unavailable: {0}")]
    Unavailable(String),
    #[error("history store rate limited: {0}")]
    RateLimited(String),
    #[error("history query canceled: {0}")]
    QueryCanceled(String),
    #[error("history schema drift: {0}")]
    SchemaDrift(String),
    #[error("history payload encode failed: {0}")]
    Encode(String),
    #[error("history payload decode failed: {0}")]
    Decode(String),
}

impl HistoryError {
    pub fn problem_code(&self) -> &'static str {
        use shared_types::problem::codes;
        match self {
            Self::Unavailable(_) => codes::HISTORY_STORE_UNAVAILABLE,
            Self::RateLimited(_) => codes::HISTORY_STORE_RATE_LIMITED,
            Self::QueryCanceled(_) => codes::HISTORY_QUERY_CANCELED,
            Self::SchemaDrift(_) => codes::HISTORY_SCHEMA_DRIFT,
            Self::Encode(_) => codes::HISTORY_ENCODE_FAILED,
            Self::Decode(_) => codes::HISTORY_DECODE_FAILED,
        }
    }

    pub(crate) fn storage_degraded_reason(
        &self,
        operation_reason: StorageDegradedReason,
    ) -> StorageDegradedReason {
        match self {
            Self::SchemaDrift(_) => StorageDegradedReason::SchemaDrift,
            Self::RateLimited(_) | Self::QueryCanceled(_) => StorageDegradedReason::Backpressure,
            Self::Encode(_) | Self::Decode(_) => StorageDegradedReason::PayloadInvalid,
            Self::Unavailable(_) => operation_reason,
        }
    }
}

impl From<HistoryError> for common::AppError {
    fn from(err: HistoryError) -> Self {
        let code = err.problem_code();
        let message = err.to_string();
        match err {
            HistoryError::Unavailable(_)
            | HistoryError::RateLimited(_)
            | HistoryError::QueryCanceled(_)
            | HistoryError::SchemaDrift(_) => {
                common::AppError::domain(common::StatusCode::SERVICE_UNAVAILABLE, code, message)
            }
            HistoryError::Encode(_) => {
                common::AppError::domain(common::StatusCode::INTERNAL_SERVER_ERROR, code, message)
            }
            HistoryError::Decode(_) => {
                common::AppError::domain(common::StatusCode::INTERNAL_SERVER_ERROR, code, message)
            }
        }
    }
}

pub(super) fn trim_to_max<T>(rows: &mut VecDeque<T>, max_rows: usize) {
    if rows.len() > max_rows {
        let drain = rows.len() - max_rows;
        rows.drain(..drain);
    }
}

pub(super) fn match_opt(filter: &Option<String>, value: &str) -> bool {
    filter
        .as_deref()
        .map(|needle| value.eq_ignore_ascii_case(needle))
        .unwrap_or(true)
}

#[cfg(test)]
mod history_error_tests {
    use super::*;

    #[test]
    fn unavailable_maps_to_service_unavailable_code() {
        let err: common::AppError = HistoryError::Unavailable("db down".into()).into();
        assert_eq!(err.code(), "HISTORY_STORE_UNAVAILABLE");
        assert_eq!(err.status().as_u16(), 503);
    }

    #[test]
    fn schema_drift_maps_to_typed_service_unavailable_code() {
        let err: common::AppError = HistoryError::SchemaDrift("missing table".into()).into();
        assert_eq!(err.code(), "HISTORY_SCHEMA_DRIFT");
        assert_eq!(err.status().as_u16(), 503);
    }

    #[test]
    fn postgres_backpressure_errors_map_to_typed_codes() {
        let limited: common::AppError =
            HistoryError::RateLimited("too many connections".into()).into();
        let canceled: common::AppError =
            HistoryError::QueryCanceled("statement timeout".into()).into();

        assert_eq!(limited.code(), "HISTORY_STORE_RATE_LIMITED");
        assert_eq!(limited.status().as_u16(), 503);
        assert_eq!(canceled.code(), "HISTORY_QUERY_CANCELED");
        assert_eq!(canceled.status().as_u16(), 503);
    }

    #[test]
    fn encode_and_decode_map_to_internal_codes() {
        let encode: common::AppError = HistoryError::Encode("bad".into()).into();
        assert_eq!(encode.code(), "HISTORY_ENCODE_FAILED");
        assert_eq!(encode.status().as_u16(), 500);
        let decode: common::AppError = HistoryError::Decode("bad".into()).into();
        assert_eq!(decode.code(), "HISTORY_DECODE_FAILED");
        assert_eq!(decode.status().as_u16(), 500);
    }
}

#[cfg(test)]
mod migration_checksum_tests {
    use super::*;

    #[test]
    fn migration_checksum_is_stable_hex_identity() {
        let checksum = history_migration_checksum_hex();

        assert_eq!(checksum.len(), 16);
        assert!(checksum.chars().all(|item| item.is_ascii_hexdigit()));
        assert_ne!(HISTORY_MIGRATION_CHECKSUM, fnv1a64(b""));
    }
}

#[cfg(test)]
mod retention_tests {
    use super::*;

    #[test]
    fn trim_to_max_drops_oldest_rows_without_reordering_survivors() {
        let mut rows = VecDeque::from([1, 2, 3, 4, 5]);

        trim_to_max(&mut rows, 3);

        assert_eq!(rows.into_iter().collect::<Vec<_>>(), vec![3, 4, 5]);
    }
}
