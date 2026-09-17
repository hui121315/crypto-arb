use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use arc_swap::ArcSwap;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{
    AlertDeliveryStatus, AlertRule, AlertRuleRuntimeStatus, WatchlistItem, WatchlistItemRuntime,
    WatchlistPersistStatus, WatchlistStorageHealth, WatchlistStorageStatus,
};

#[cfg(test)]
use super::AlertRuleRuntime;

pub const WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION: u32 = 1;
pub const WATCHLIST_ALERT_STORAGE_MIGRATION_ID: &str = "20260714_watchlist_alerts";
pub const WATCHLIST_ALERT_STORAGE_MIGRATION_PATH: &str =
    "crates/realtime/migrations/20260714_watchlist_alerts.sql";

const SCHEMA_SQL: &str = include_str!("../../migrations/20260714_watchlist_alerts.sql");
const SOURCE: &str = "watchlist_alert_storage";
const FNV64_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
const FNV64_PRIME: u64 = 0x100000001b3;
const SCHEMA_HASH_VALUE: u64 = fnv1a64(SCHEMA_SQL.as_bytes());

#[derive(Debug, Clone)]
pub struct WatchlistAlertReplay {
    pub store: Arc<WatchlistAlertStore>,
    pub watchlist: Vec<WatchlistItem>,
    pub alert_rules: Vec<AlertRule>,
}

#[derive(Debug)]
pub struct WatchlistAlertStore {
    path: Option<PathBuf>,
    required: bool,
    load_blocked: AtomicBool,
    health: ArcSwap<WatchlistStorageHealth>,
}

impl Default for WatchlistAlertStore {
    fn default() -> Self {
        Self {
            path: None,
            required: false,
            load_blocked: AtomicBool::new(false),
            health: ArcSwap::from_pointee(WatchlistStorageHealth::default()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DurableSnapshot {
    schema_version: u32,
    revision: u64,
    persisted_at_ms: i64,
    watchlist: Vec<WatchlistItem>,
    alert_rules: Vec<AlertRule>,
}

struct LoadedSnapshot {
    snapshot: Option<DurableSnapshot>,
}

struct PersistedSnapshot {
    revision: u64,
    persisted_at_ms: i64,
    watchlist_item_count: usize,
    alert_rule_count: usize,
}

impl WatchlistAlertStore {
    pub async fn initialize(path: Option<PathBuf>, required: bool) -> WatchlistAlertReplay {
        let store = Arc::new(Self {
            path,
            required,
            load_blocked: AtomicBool::new(false),
            health: ArcSwap::from_pointee(WatchlistStorageHealth::default()),
        });
        let Some(path) = store.path.clone() else {
            if required {
                store.load_blocked.store(true, Ordering::Release);
                store.health.store(Arc::new(degraded_health(
                    false,
                    "WATCHLIST_STORAGE_NOT_CONFIGURED",
                    "watchlist alerts require a configured SQLite path",
                )));
            }
            return WatchlistAlertReplay {
                store,
                watchlist: Vec::new(),
                alert_rules: Vec::new(),
            };
        };

        match load_path(path).await {
            Ok(loaded) => store.loaded_replay(loaded),
            Err(error) => {
                tracing::warn!(%error, "watchlist alert SQLite restore failed");
                store.load_blocked.store(true, Ordering::Release);
                store.health.store(Arc::new(degraded_health(
                    true,
                    "WATCHLIST_STORAGE_LOAD_FAILED",
                    "watchlist alert SQLite restore failed; mutations are blocked",
                )));
                WatchlistAlertReplay {
                    store,
                    watchlist: Vec::new(),
                    alert_rules: Vec::new(),
                }
            }
        }
    }

    pub fn health(&self) -> WatchlistStorageHealth {
        self.health.load_full().as_ref().clone()
    }

    pub async fn persist_snapshot(
        &self,
        watchlist: &[WatchlistItem],
        alert_rules: &[AlertRule],
    ) -> Result<WatchlistPersistStatus, String> {
        let Some(path) = self.path.clone() else {
            if self.required {
                return Err("watchlist alert SQLite path is not configured".to_owned());
            }
            return Ok(WatchlistPersistStatus::Volatile);
        };
        if self.load_blocked.load(Ordering::Acquire) {
            return Err("watchlist alert storage is blocked after restore failure".to_owned());
        }

        let attempts = self.health().persist_attempts.saturating_add(1);
        let persisted_at_ms = common::time::now_ms();
        let mut durable_watchlist = watchlist.to_vec();
        let mut durable_rules = alert_rules.to_vec();
        prepare_durable_rows(&mut durable_watchlist, &mut durable_rules, persisted_at_ms);
        match persist_path(path, durable_watchlist, durable_rules, persisted_at_ms).await {
            Ok(persisted) => {
                let previous = self.health();
                self.health.store(Arc::new(WatchlistStorageHealth {
                    backend: "sqlite".to_owned(),
                    configured: true,
                    status: WatchlistStorageStatus::Ready,
                    schema_version: Some(WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION),
                    revision: persisted.revision,
                    watchlist_item_count: persisted.watchlist_item_count,
                    alert_rule_count: persisted.alert_rule_count,
                    persist_attempts: attempts,
                    persist_successes: previous.persist_successes.saturating_add(1),
                    last_persisted_at_ms: Some(persisted.persisted_at_ms),
                    problem: None,
                }));
                Ok(WatchlistPersistStatus::Persisted)
            }
            Err(error) => {
                tracing::warn!(%error, "watchlist alert SQLite persist failed");
                let mut health = self.health();
                health.status = WatchlistStorageStatus::Degraded;
                health.persist_attempts = attempts;
                health.problem = Some(storage_problem(
                    "WATCHLIST_STORAGE_PERSIST_FAILED",
                    "watchlist alert SQLite persist failed",
                ));
                self.health.store(Arc::new(health));
                Err(error)
            }
        }
    }

    fn loaded_replay(self: &Arc<Self>, loaded: LoadedSnapshot) -> WatchlistAlertReplay {
        let Some(mut snapshot) = loaded.snapshot else {
            self.health.store(Arc::new(ready_health(0, 0, 0, None)));
            return WatchlistAlertReplay {
                store: Arc::clone(self),
                watchlist: Vec::new(),
                alert_rules: Vec::new(),
            };
        };
        restore_runtime_rows(&mut snapshot.watchlist, &mut snapshot.alert_rules);
        self.health.store(Arc::new(ready_health(
            snapshot.revision,
            snapshot.watchlist.len(),
            snapshot.alert_rules.len(),
            Some(snapshot.persisted_at_ms),
        )));
        WatchlistAlertReplay {
            store: Arc::clone(self),
            watchlist: snapshot.watchlist,
            alert_rules: snapshot.alert_rules,
        }
    }
}

pub fn storage_schema_hash() -> String {
    format!("fnv1a64:{SCHEMA_HASH_VALUE:016x}")
}

fn ready_health(
    revision: u64,
    watchlist_item_count: usize,
    alert_rule_count: usize,
    last_persisted_at_ms: Option<i64>,
) -> WatchlistStorageHealth {
    WatchlistStorageHealth {
        backend: "sqlite".to_owned(),
        configured: true,
        status: WatchlistStorageStatus::Ready,
        schema_version: Some(WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION),
        revision,
        watchlist_item_count,
        alert_rule_count,
        persist_attempts: 0,
        persist_successes: 0,
        last_persisted_at_ms,
        problem: None,
    }
}

fn degraded_health(configured: bool, code: &str, message: &str) -> WatchlistStorageHealth {
    WatchlistStorageHealth {
        backend: "sqlite".to_owned(),
        configured,
        status: WatchlistStorageStatus::Degraded,
        problem: Some(storage_problem(code, message)),
        ..WatchlistStorageHealth::default()
    }
}

fn storage_problem(code: &str, message: &str) -> shared_types::ApiProblem {
    shared_types::ApiProblem::new(code, message).with_source(SOURCE)
}

fn prepare_durable_rows(
    watchlist: &mut [WatchlistItem],
    alert_rules: &mut [AlertRule],
    now_ms: i64,
) {
    for item in watchlist {
        item.persistence.persist_status = WatchlistPersistStatus::Persisted;
        item.runtime = WatchlistItemRuntime::default();
    }
    for rule in alert_rules {
        rule.persistence.persist_status = WatchlistPersistStatus::Persisted;
        rule.delivery.delivery_kind = rule.channel.delivery_kind().to_owned();
        if rule.delivery.last_fired_at_ms.is_none() {
            rule.delivery.last_fired_at_ms = rule.runtime.last_triggered_at_ms;
        }
        rule.runtime.last_evaluated_at_ms = None;
        rule.runtime.status = restored_rule_status(rule, now_ms);
        rule.runtime.problem = rule.delivery.last_error.clone();
    }
}

fn restore_runtime_rows(watchlist: &mut [WatchlistItem], alert_rules: &mut [AlertRule]) {
    let now_ms = common::time::now_ms();
    for item in watchlist {
        item.persistence.persist_status = WatchlistPersistStatus::Persisted;
        item.runtime = WatchlistItemRuntime::default();
    }
    for rule in alert_rules {
        rule.persistence.persist_status = WatchlistPersistStatus::Persisted;
        rule.delivery.delivery_kind = rule.channel.delivery_kind().to_owned();
        rule.runtime.transport = rule.channel.runtime_transport().to_owned();
        rule.runtime.delivery_supported = rule.channel.is_runtime_deliverable();
        rule.runtime.last_evaluated_at_ms = None;
        rule.runtime.last_triggered_at_ms = rule.delivery.last_fired_at_ms;
        rule.runtime.status = restored_rule_status(rule, now_ms);
        rule.runtime.problem = rule.delivery.last_error.clone();
    }
}

fn restored_rule_status(rule: &AlertRule, now_ms: i64) -> AlertRuleRuntimeStatus {
    if !rule.enabled {
        AlertRuleRuntimeStatus::Disabled
    } else if rule
        .runtime
        .next_eligible_at_ms
        .is_some_and(|deadline| deadline > now_ms)
    {
        AlertRuleRuntimeStatus::Cooldown
    } else if rule.delivery.last_delivery_status == AlertDeliveryStatus::Blocked {
        AlertRuleRuntimeStatus::Blocked
    } else {
        AlertRuleRuntimeStatus::Configured
    }
}

async fn load_path(path: PathBuf) -> Result<LoadedSnapshot, String> {
    tokio::task::spawn_blocking(move || load_sync(&path))
        .await
        .map_err(|error| error.to_string())?
}

async fn persist_path(
    path: PathBuf,
    watchlist: Vec<WatchlistItem>,
    alert_rules: Vec<AlertRule>,
    persisted_at_ms: i64,
) -> Result<PersistedSnapshot, String> {
    tokio::task::spawn_blocking(move || {
        persist_sync(&path, watchlist, alert_rules, persisted_at_ms)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn load_sync(path: &Path) -> Result<LoadedSnapshot, String> {
    let conn = open(path)?;
    let row = conn
        .query_row(
            "SELECT schema_version, revision, payload_json, payload_hash, updated_at_ms
             FROM watchlist_alert_snapshots WHERE singleton = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((schema_version, revision, payload, expected_hash, updated_at_ms)) = row else {
        return Ok(LoadedSnapshot { snapshot: None });
    };
    if schema_version != i64::from(WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION) {
        return Err(format!(
            "unsupported watchlist alert schema version {schema_version}"
        ));
    }
    if payload_hash(&payload) != expected_hash {
        return Err("watchlist alert payload hash mismatch".to_owned());
    }
    let snapshot = serde_json::from_str::<DurableSnapshot>(&payload)
        .map_err(|error| format!("decode watchlist alert snapshot: {error}"))?;
    let revision = u64::try_from(revision).map_err(|_| "negative snapshot revision".to_owned())?;
    if snapshot.schema_version != WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION
        || snapshot.revision != revision
        || snapshot.persisted_at_ms != updated_at_ms
    {
        return Err("watchlist alert snapshot identity drift".to_owned());
    }
    validate_snapshot(&snapshot)?;
    Ok(LoadedSnapshot {
        snapshot: Some(snapshot),
    })
}

fn persist_sync(
    path: &Path,
    watchlist: Vec<WatchlistItem>,
    alert_rules: Vec<AlertRule>,
    persisted_at_ms: i64,
) -> Result<PersistedSnapshot, String> {
    let mut conn = open(path)?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let previous_revision = transaction
        .query_row(
            "SELECT revision FROM watchlist_alert_snapshots WHERE singleton = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0);
    let revision = u64::try_from(previous_revision)
        .map_err(|_| "negative snapshot revision".to_owned())?
        .checked_add(1)
        .ok_or_else(|| "watchlist alert snapshot revision overflow".to_owned())?;
    let snapshot = DurableSnapshot {
        schema_version: WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION,
        revision,
        persisted_at_ms,
        watchlist,
        alert_rules,
    };
    validate_snapshot(&snapshot)?;
    let payload = serde_json::to_string(&snapshot)
        .map_err(|error| format!("encode watchlist alert snapshot: {error}"))?;
    let hash = payload_hash(&payload);
    let revision_sql = i64::try_from(revision)
        .map_err(|_| "watchlist alert snapshot revision exceeds SQLite range".to_owned())?;
    transaction
        .execute(
            "INSERT INTO watchlist_alert_snapshots
                (singleton, schema_version, revision, payload_json, payload_hash, updated_at_ms)
             VALUES (1, ?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(singleton) DO UPDATE SET
                schema_version = excluded.schema_version,
                revision = excluded.revision,
                payload_json = excluded.payload_json,
                payload_hash = excluded.payload_hash,
                updated_at_ms = excluded.updated_at_ms",
            params![
                WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION,
                revision_sql,
                payload,
                hash,
                persisted_at_ms
            ],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(PersistedSnapshot {
        revision,
        persisted_at_ms,
        watchlist_item_count: snapshot.watchlist.len(),
        alert_rule_count: snapshot.alert_rules.len(),
    })
}

fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut conn = Connection::open(path).map_err(|error| error.to_string())?;
    conn.execute_batch(SCHEMA_SQL)
        .map_err(|error| error.to_string())?;
    verify_or_initialize_schema_identity(&mut conn)?;
    Ok(conn)
}

fn verify_or_initialize_schema_identity(conn: &mut Connection) -> Result<(), String> {
    let expected = [
        (
            "schema_version",
            WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION.to_string(),
        ),
        ("schema_hash", storage_schema_hash()),
        (
            "migration_id",
            WATCHLIST_ALERT_STORAGE_MIGRATION_ID.to_owned(),
        ),
        (
            "migration_path",
            WATCHLIST_ALERT_STORAGE_MIGRATION_PATH.to_owned(),
        ),
    ];
    let user_version = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?;
    if user_version != 0 && user_version != i64::from(WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION) {
        return Err(format!(
            "watchlist alert SQLite user_version drift: expected {}, found {user_version}",
            WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION
        ));
    }
    for (key, expected_value) in &expected {
        let actual = conn
            .query_row(
                "SELECT value FROM watchlist_alert_meta WHERE key = ?1",
                [key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        if actual
            .as_deref()
            .is_some_and(|value| value != expected_value)
        {
            return Err(format!("watchlist alert SQLite metadata drift for {key}"));
        }
    }

    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    transaction
        .pragma_update(None, "user_version", WATCHLIST_ALERT_STORAGE_SCHEMA_VERSION)
        .map_err(|error| error.to_string())?;
    for (key, value) in expected {
        transaction
            .execute(
                "INSERT OR IGNORE INTO watchlist_alert_meta (key, value) VALUES (?1, ?2)",
                params![key, value],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

fn validate_snapshot(snapshot: &DurableSnapshot) -> Result<(), String> {
    let mut ids = HashSet::new();
    for item in &snapshot.watchlist {
        item.validate()?;
        if item.id <= 0 || item.persistence.version == 0 || !ids.insert(item.id) {
            return Err("watchlist snapshot contains invalid or duplicate identity".to_owned());
        }
    }
    let mut rule_ids = HashSet::new();
    for rule in &snapshot.alert_rules {
        rule.validate()?;
        if rule.id <= 0
            || rule.persistence.version == 0
            || !rule_ids.insert(rule.id)
            || !ids.contains(&rule.watchlist_id)
        {
            return Err("alert rule snapshot contains invalid identity or reference".to_owned());
        }
    }
    Ok(())
}

fn payload_hash(payload: &str) -> String {
    let digest = Sha256::digest(payload.as_bytes());
    format!("sha256:{digest:x}")
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

#[cfg(test)]
#[path = "storage/tests.rs"]
mod tests;
