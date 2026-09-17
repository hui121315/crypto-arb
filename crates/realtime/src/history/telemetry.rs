use super::HistoryError;
use shared_types::StorageDegradedReason;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug)]
pub(super) struct HistoryStoreTelemetry {
    pub(super) startup_problem: arc_swap::ArcSwapOption<String>,
    pub(super) append_success_total: AtomicU64,
    pub(super) append_error_total: AtomicU64,
    pub(super) query_success_total: AtomicU64,
    pub(super) query_error_total: AtomicU64,
    schema_version: AtomicU64,
    pub(super) last_success_at_ms: AtomicI64,
    pub(super) last_append_at_ms: AtomicI64,
    pub(super) last_query_at_ms: AtomicI64,
    pub(super) last_error_at_ms: AtomicI64,
    pub(super) last_error: arc_swap::ArcSwapOption<String>,
    pub(super) last_error_code: arc_swap::ArcSwapOption<String>,
    pub(super) last_error_reason: arc_swap::ArcSwapOption<StorageDegradedReason>,
}

impl HistoryStoreTelemetry {
    pub(super) fn new(startup_problem: Option<String>, schema_version: Option<u32>) -> Self {
        let startup = arc_swap::ArcSwapOption::empty();
        if let Some(problem) = startup_problem {
            startup.store(Some(Arc::new(problem)));
        }
        Self {
            startup_problem: startup,
            append_success_total: AtomicU64::new(0),
            append_error_total: AtomicU64::new(0),
            query_success_total: AtomicU64::new(0),
            query_error_total: AtomicU64::new(0),
            schema_version: AtomicU64::new(schema_version.map(u64::from).unwrap_or_default()),
            last_success_at_ms: AtomicI64::new(0),
            last_append_at_ms: AtomicI64::new(0),
            last_query_at_ms: AtomicI64::new(0),
            last_error_at_ms: AtomicI64::new(0),
            last_error: arc_swap::ArcSwapOption::empty(),
            last_error_code: arc_swap::ArcSwapOption::empty(),
            last_error_reason: arc_swap::ArcSwapOption::empty(),
        }
    }

    pub(super) fn record_success(
        &self,
        counter: &AtomicU64,
        operation_success_at_ms: Option<&AtomicI64>,
    ) {
        counter.fetch_add(1, Ordering::Relaxed);
        let now_ms = common::time::now_ms();
        self.last_success_at_ms.store(now_ms, Ordering::Relaxed);
        if let Some(operation_success_at_ms) = operation_success_at_ms {
            operation_success_at_ms.store(now_ms, Ordering::Relaxed);
        }
    }

    pub(super) fn record_error(
        &self,
        counter: &AtomicU64,
        error: &HistoryError,
        operation_reason: StorageDegradedReason,
    ) {
        counter.fetch_add(1, Ordering::Relaxed);
        self.last_error_at_ms
            .store(common::time::now_ms(), Ordering::Relaxed);
        self.last_error.store(Some(Arc::new(error.to_string())));
        self.last_error_code
            .store(Some(Arc::new(error.problem_code().to_owned())));
        self.last_error_reason.store(Some(Arc::new(
            error.storage_degraded_reason(operation_reason),
        )));
    }

    pub(super) fn schema_version(&self) -> Option<u32> {
        let version = self.schema_version.load(Ordering::Relaxed);
        (version > 0).then_some(version as u32)
    }
}
