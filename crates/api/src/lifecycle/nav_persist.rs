use common::config::AppConfig;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Arc;
use tracing::warn;

mod sqlite;

use sqlite::{append_path, load_path};

pub(crate) const NAV_SCHEMA_VERSION: u32 = 1;
pub(crate) const NAV_SCHEMA_MIGRATION_ID: &str = "20260605_portfolio_nav";
pub(crate) const NAV_SCHEMA_MIGRATION_PATH: &str =
    "crates/api/migrations/20260605_portfolio_nav.sql";
pub(crate) const NAV_SAMPLE_STATUS_OK: &str = "ok";
pub(crate) const NAV_SAMPLE_STATUS_UNKNOWN: &str = "unknown";
pub(crate) const NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY: &str = "account_equity";
pub(crate) const NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY_MISSING: &str = "account_equity_missing";

const NAV_SCHEMA_SQL: &str = include_str!("../../migrations/20260605_portfolio_nav.sql");
const FNV64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV64_PRIME: u64 = 0x100000001b3;
const NAV_SCHEMA_HASH_VALUE: u64 = fnv1a64(NAV_SCHEMA_SQL.as_bytes());

pub(crate) fn nav_schema_hash() -> String {
    format!("fnv1a64:{NAV_SCHEMA_HASH_VALUE:016x}")
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

#[derive(Debug)]
pub(crate) struct NavStorageHealthStore {
    path: Option<String>,
    load_success_total: AtomicU64,
    load_error_total: AtomicU64,
    append_success_total: AtomicU64,
    append_error_total: AtomicU64,
    last_success_at_ms: AtomicI64,
    last_error_at_ms: AtomicI64,
    latest_sample_at_ms: AtomicI64,
    schema_version: AtomicU64,
    sample_count: AtomicU64,
    last_error: arc_swap::ArcSwapOption<String>,
    latest_sample_status: arc_swap::ArcSwapOption<String>,
    latest_sample_source: arc_swap::ArcSwapOption<String>,
    latest_sample_problem: arc_swap::ArcSwapOption<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NavStorageHealth {
    pub(crate) path: Option<String>,
    pub(crate) enabled: bool,
    pub(crate) load_success_total: u64,
    pub(crate) load_error_total: u64,
    pub(crate) append_success_total: u64,
    pub(crate) append_error_total: u64,
    pub(crate) last_success_at_ms: Option<i64>,
    pub(crate) last_error_at_ms: Option<i64>,
    pub(crate) latest_sample_at_ms: Option<i64>,
    pub(crate) schema_version: Option<u32>,
    pub(crate) migration_checksum: Option<String>,
    pub(crate) sample_count: u64,
    pub(crate) last_error: Option<String>,
    pub(crate) latest_sample_status: Option<String>,
    pub(crate) latest_sample_source: Option<String>,
    pub(crate) latest_sample_problem: Option<String>,
    pub(crate) observed_at_ms: i64,
}

impl NavStorageHealthStore {
    pub(crate) fn new(config: &AppConfig) -> Self {
        Self {
            path: nav_path(config).map(|path| path.display().to_string()),
            load_success_total: AtomicU64::new(0),
            load_error_total: AtomicU64::new(0),
            append_success_total: AtomicU64::new(0),
            append_error_total: AtomicU64::new(0),
            last_success_at_ms: AtomicI64::new(0),
            last_error_at_ms: AtomicI64::new(0),
            latest_sample_at_ms: AtomicI64::new(0),
            schema_version: AtomicU64::new(0),
            sample_count: AtomicU64::new(0),
            last_error: arc_swap::ArcSwapOption::empty(),
            latest_sample_status: arc_swap::ArcSwapOption::empty(),
            latest_sample_source: arc_swap::ArcSwapOption::empty(),
            latest_sample_problem: arc_swap::ArcSwapOption::empty(),
        }
    }

    pub(crate) fn snapshot(&self, observed_at_ms: i64) -> NavStorageHealth {
        NavStorageHealth {
            path: self.path.clone(),
            enabled: self.path.is_some(),
            load_success_total: self.load_success_total.load(Ordering::Relaxed),
            load_error_total: self.load_error_total.load(Ordering::Relaxed),
            append_success_total: self.append_success_total.load(Ordering::Relaxed),
            append_error_total: self.append_error_total.load(Ordering::Relaxed),
            last_success_at_ms: non_zero_ms(self.last_success_at_ms.load(Ordering::Relaxed)),
            last_error_at_ms: non_zero_ms(self.last_error_at_ms.load(Ordering::Relaxed)),
            latest_sample_at_ms: non_zero_ms(self.latest_sample_at_ms.load(Ordering::Relaxed)),
            schema_version: non_zero_u32(self.schema_version.load(Ordering::Relaxed)),
            migration_checksum: self.path.as_ref().map(|_| nav_schema_hash()),
            sample_count: self.sample_count.load(Ordering::Relaxed),
            last_error: self.last_error.load_full().as_deref().cloned(),
            latest_sample_status: self.latest_sample_status.load_full().as_deref().cloned(),
            latest_sample_source: self.latest_sample_source.load_full().as_deref().cloned(),
            latest_sample_problem: self.latest_sample_problem.load_full().as_deref().cloned(),
            observed_at_ms,
        }
    }

    fn record_load_success(&self, sample_count: u64) {
        self.record_success(&self.load_success_total, sample_count);
    }

    fn record_load_error(&self, error: &str) {
        self.record_error(&self.load_error_total, error);
    }

    fn record_append_success(&self, sample_count: u64, occurred_at_ms: i64) {
        self.record_success(&self.append_success_total, sample_count);
        self.record_sample(
            occurred_at_ms,
            NAV_SAMPLE_STATUS_OK,
            NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY,
            None,
        );
    }

    fn record_append_error(&self, error: &str) {
        self.record_error(&self.append_error_total, error);
    }

    pub(crate) fn record_sample_skipped(&self, occurred_at_ms: i64, source: &str, problem: &str) {
        self.record_sample(
            occurred_at_ms,
            NAV_SAMPLE_STATUS_UNKNOWN,
            source,
            Some(problem),
        );
    }

    fn record_success(&self, counter: &AtomicU64, sample_count: u64) {
        counter.fetch_add(1, Ordering::Relaxed);
        self.last_success_at_ms
            .store(common::time::now_ms(), Ordering::Relaxed);
        self.schema_version
            .store(u64::from(NAV_SCHEMA_VERSION), Ordering::Relaxed);
        self.sample_count.store(sample_count, Ordering::Relaxed);
    }

    fn record_error(&self, counter: &AtomicU64, error: &str) {
        counter.fetch_add(1, Ordering::Relaxed);
        self.last_error_at_ms
            .store(common::time::now_ms(), Ordering::Relaxed);
        self.last_error.store(Some(Arc::new(error.to_owned())));
    }

    fn record_sample(
        &self,
        occurred_at_ms: i64,
        status: &str,
        source: &str,
        problem: Option<&str>,
    ) {
        self.latest_sample_at_ms
            .store(occurred_at_ms, Ordering::Relaxed);
        self.latest_sample_status
            .store(Some(Arc::new(status.to_owned())));
        self.latest_sample_source
            .store(Some(Arc::new(source.to_owned())));
        self.latest_sample_problem
            .store(problem.map(|value| Arc::new(value.to_owned())));
    }
}

impl NavStorageHealth {
    pub(crate) fn success_total(&self) -> u64 {
        self.load_success_total
            .saturating_add(self.append_success_total)
    }

    pub(crate) fn error_total(&self) -> u64 {
        self.load_error_total
            .saturating_add(self.append_error_total)
    }
}

pub(crate) async fn load(
    config: &AppConfig,
    oldest_ms: i64,
    health: &NavStorageHealthStore,
) -> Vec<(i64, f64)> {
    let Some(path) = nav_path(config) else {
        return Vec::new();
    };
    match load_path(path, oldest_ms).await {
        Ok(result) => {
            health.record_load_success(result.sample_count);
            result.rows
        }
        Err(error) => {
            health.record_load_error(&error);
            warn!(%error, "portfolio nav history load failed");
            Vec::new()
        }
    }
}

pub(crate) async fn append_sample(
    config: &AppConfig,
    occurred_at_ms: i64,
    nav_usd: f64,
    health: &NavStorageHealthStore,
) {
    let Some(path) = nav_path(config) else {
        return;
    };
    match append_path(path, occurred_at_ms, nav_usd).await {
        Ok(sample_count) => health.record_append_success(sample_count, occurred_at_ms),
        Err(error) => {
            health.record_append_error(&error);
            warn!(%error, "portfolio nav history append failed");
        }
    }
}

fn nav_path(config: &AppConfig) -> Option<PathBuf> {
    config
        .storage
        .portfolio_nav_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| config.storage.resolve_runtime_path(value))
}

fn non_zero_ms(value: i64) -> Option<i64> {
    (value > 0).then_some(value)
}

fn non_zero_u32(value: u64) -> Option<u32> {
    (value > 0).then_some(value.min(u64::from(u32::MAX)) as u32)
}

#[cfg(test)]
mod tests;
