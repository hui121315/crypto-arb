use crate::diagnostics::sanitize_delivery_record;
use rusqlite::{params, Connection, OptionalExtension};
use shared_types::{WebhookDeliveryRecord, WebhookDeliveryStatus, WebhookEvent};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const SCHEMA_VERSION: i64 = 1;
const TERMINAL_RETENTION: i64 = 10_000;
const STATE_PENDING: &str = "pending";
const STATE_BASELINE: &str = "baseline";
const META_BOOTSTRAP_COMPLETE: &str = "bootstrap_complete";
const META_DIAGNOSTIC_REDACTION: &str = "diagnostic_redaction_v1";

const SCHEMA_SQL: &str = "
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
CREATE TABLE IF NOT EXISTS webhook_outbox_meta (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS webhook_outbox_events (
    event_id TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    kind TEXT NOT NULL,
    event_json TEXT,
    record_json TEXT,
    occurred_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_webhook_outbox_state_updated
    ON webhook_outbox_events(state, updated_at_ms);
";

#[derive(Debug, Default)]
pub(super) struct OutboxReplay {
    pub(super) pending: Vec<WebhookEvent>,
    pub(super) recent: Vec<WebhookDeliveryRecord>,
    pub(super) event_ids: Vec<String>,
    pub(super) delivered_total: u64,
    pub(super) failed_total: u64,
    pub(super) requires_bootstrap: bool,
}

#[derive(Clone, Debug, Default)]
pub(super) struct WebhookOutbox {
    path: Option<Arc<PathBuf>>,
}

impl WebhookOutbox {
    pub(super) async fn initialize(path: Option<PathBuf>) -> Result<(Self, OutboxReplay), String> {
        let outbox = Self {
            path: path.map(Arc::new),
        };
        let Some(path) = outbox.path.as_deref().cloned() else {
            return Ok((outbox, OutboxReplay::default()));
        };
        let replay = tokio::task::spawn_blocking(move || load_sync(&path))
            .await
            .map_err(|error| format!("webhook outbox load task failed: {error}"))??;
        Ok((outbox, replay))
    }

    pub(super) fn is_durable(&self) -> bool {
        self.path.is_some()
    }

    pub(super) async fn reserve(&self, event: WebhookEvent) -> Result<bool, String> {
        let Some(path) = self.path.as_deref().cloned() else {
            return Ok(true);
        };
        tokio::task::spawn_blocking(move || reserve_sync(&path, &event))
            .await
            .map_err(|error| format!("webhook outbox reserve task failed: {error}"))?
    }

    pub(super) async fn release_pending(&self, event_id: String) -> Result<(), String> {
        let Some(path) = self.path.as_deref().cloned() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || release_pending_sync(&path, &event_id))
            .await
            .map_err(|error| format!("webhook outbox release task failed: {error}"))?
    }

    pub(super) async fn complete(&self, record: WebhookDeliveryRecord) -> Result<(), String> {
        let Some(path) = self.path.as_deref().cloned() else {
            return Ok(());
        };
        let record = sanitize_delivery_record(record);
        tokio::task::spawn_blocking(move || complete_sync(&path, &record))
            .await
            .map_err(|error| format!("webhook outbox completion task failed: {error}"))?
    }

    pub(super) async fn bootstrap(&self, event_ids: Vec<String>) -> Result<(), String> {
        let Some(path) = self.path.as_deref().cloned() else {
            return Ok(());
        };
        tokio::task::spawn_blocking(move || bootstrap_sync(&path, &event_ids))
            .await
            .map_err(|error| format!("webhook outbox bootstrap task failed: {error}"))?
    }
}

fn load_sync(path: &Path) -> Result<OutboxReplay, String> {
    let mut conn = open(path)?;
    redact_legacy_diagnostics(&mut conn)?;
    let pending = load_json_rows::<WebhookEvent>(
        &conn,
        "SELECT event_json FROM webhook_outbox_events
         WHERE state = 'pending' AND event_json IS NOT NULL
         ORDER BY occurred_at_ms ASC",
        "pending webhook event",
    )?;
    let recent = load_json_rows::<WebhookDeliveryRecord>(
        &conn,
        "SELECT record_json FROM webhook_outbox_events
         WHERE state != 'pending' AND record_json IS NOT NULL
         ORDER BY updated_at_ms DESC LIMIT 50",
        "webhook delivery record",
    )?;
    let event_ids = load_string_rows(
        &conn,
        "SELECT event_id FROM webhook_outbox_events ORDER BY updated_at_ms ASC",
    )?;
    let requires_bootstrap = conn
        .query_row(
            "SELECT value FROM webhook_outbox_meta WHERE key = ?1",
            [META_BOOTSTRAP_COMPLETE],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_none();
    Ok(OutboxReplay {
        pending,
        recent,
        event_ids,
        delivered_total: count_state(&conn, "delivered")?,
        failed_total: count_state(&conn, "failed")?,
        requires_bootstrap,
    })
}

fn redact_legacy_diagnostics(conn: &mut Connection) -> Result<(), String> {
    let already_redacted = conn
        .query_row(
            "SELECT value FROM webhook_outbox_meta WHERE key = ?1",
            [META_DIAGNOSTIC_REDACTION],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();
    if already_redacted {
        return Ok(());
    }

    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let rows = {
        let mut statement = transaction
            .prepare(
                "SELECT event_id, record_json FROM webhook_outbox_events
                 WHERE record_json IS NOT NULL",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    for (event_id, encoded) in rows {
        let record = serde_json::from_str::<WebhookDeliveryRecord>(&encoded)
            .map_err(|error| format!("decode webhook delivery record: {error}"))?;
        let sanitized = serde_json::to_string(&sanitize_delivery_record(record))
            .map_err(|error| format!("encode webhook delivery record: {error}"))?;
        if sanitized != encoded {
            transaction
                .execute(
                    "UPDATE webhook_outbox_events SET record_json = ?1 WHERE event_id = ?2",
                    params![sanitized, event_id],
                )
                .map_err(|error| error.to_string())?;
        }
    }
    transaction
        .execute(
            "INSERT OR REPLACE INTO webhook_outbox_meta (key, value) VALUES (?1, '1')",
            [META_DIAGNOSTIC_REDACTION],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

fn reserve_sync(path: &Path, event: &WebhookEvent) -> Result<bool, String> {
    let conn = open(path)?;
    let event_json = serde_json::to_string(event)
        .map_err(|error| format!("encode pending webhook event: {error}"))?;
    let kind = serde_json::to_string(&event.kind)
        .map_err(|error| format!("encode webhook event kind: {error}"))?;
    let inserted = conn
        .execute(
            "INSERT OR IGNORE INTO webhook_outbox_events
                (event_id, state, kind, event_json, record_json, occurred_at_ms, updated_at_ms)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?5)",
            params![
                event.id,
                STATE_PENDING,
                kind,
                event_json,
                event.occurred_at_ms
            ],
        )
        .map_err(|error| error.to_string())?;
    Ok(inserted == 1)
}

fn release_pending_sync(path: &Path, event_id: &str) -> Result<(), String> {
    let conn = open(path)?;
    conn.execute(
        "DELETE FROM webhook_outbox_events WHERE event_id = ?1 AND state = ?2",
        params![event_id, STATE_PENDING],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

fn complete_sync(path: &Path, record: &WebhookDeliveryRecord) -> Result<(), String> {
    let conn = open(path)?;
    let record_json = serde_json::to_string(record)
        .map_err(|error| format!("encode webhook delivery record: {error}"))?;
    let kind = serde_json::to_string(&record.kind)
        .map_err(|error| format!("encode webhook delivery kind: {error}"))?;
    let state = delivery_state(record.status);
    conn.execute(
        "INSERT INTO webhook_outbox_events
            (event_id, state, kind, event_json, record_json, occurred_at_ms, updated_at_ms)
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?5)
         ON CONFLICT(event_id) DO UPDATE SET
            state = excluded.state,
            kind = excluded.kind,
            event_json = NULL,
            record_json = excluded.record_json,
            updated_at_ms = excluded.updated_at_ms",
        params![
            record.event_id,
            state,
            kind,
            record_json,
            record.updated_at_ms
        ],
    )
    .map_err(|error| error.to_string())?;
    prune_terminal(&conn)
}

fn bootstrap_sync(path: &Path, event_ids: &[String]) -> Result<(), String> {
    let mut conn = open(path)?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let now_ms = now_ms();
    for event_id in event_ids {
        transaction
            .execute(
                "INSERT OR IGNORE INTO webhook_outbox_events
                    (event_id, state, kind, event_json, record_json, occurred_at_ms, updated_at_ms)
                 VALUES (?1, ?2, ?3, NULL, NULL, ?4, ?4)",
                params![event_id, STATE_BASELINE, STATE_BASELINE, now_ms],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction
        .execute(
            "INSERT OR REPLACE INTO webhook_outbox_meta (key, value) VALUES (?1, '1')",
            [META_BOOTSTRAP_COMPLETE],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    prune_terminal(&conn)
}

fn open(path: &Path) -> Result<Connection, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let conn = Connection::open(path).map_err(|error| error.to_string())?;
    conn.busy_timeout(std::time::Duration::from_secs(2))
        .map_err(|error| error.to_string())?;
    conn.execute_batch(SCHEMA_SQL)
        .map_err(|error| error.to_string())?;
    let version = conn
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| error.to_string())?;
    if version != 0 && version != SCHEMA_VERSION {
        return Err(format!(
            "unsupported webhook outbox schema version {version}"
        ));
    }
    conn.pragma_update(None, "user_version", SCHEMA_VERSION)
        .map_err(|error| error.to_string())?;
    Ok(conn)
}

fn load_json_rows<T>(conn: &Connection, query: &str, label: &str) -> Result<Vec<T>, String>
where
    T: serde::de::DeserializeOwned,
{
    let mut statement = conn.prepare(query).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    rows.into_iter()
        .map(|row| serde_json::from_str(&row).map_err(|error| format!("decode {label}: {error}")))
        .collect()
}

fn load_string_rows(conn: &Connection, query: &str) -> Result<Vec<String>, String> {
    let mut statement = conn.prepare(query).map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn count_state(conn: &Connection, state: &str) -> Result<u64, String> {
    let count = conn
        .query_row(
            "SELECT COUNT(*) FROM webhook_outbox_events WHERE state = ?1",
            [state],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(count.max(0) as u64)
}

fn prune_terminal(conn: &Connection) -> Result<(), String> {
    conn.execute(
        "DELETE FROM webhook_outbox_events
         WHERE state != ?1 AND event_id NOT IN (
            SELECT event_id FROM webhook_outbox_events
            WHERE state != ?1 ORDER BY updated_at_ms DESC LIMIT ?2
         )",
        params![STATE_PENDING, TERMINAL_RETENTION],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

const fn delivery_state(status: WebhookDeliveryStatus) -> &'static str {
    match status {
        WebhookDeliveryStatus::Delivered => "delivered",
        WebhookDeliveryStatus::Failed => "failed",
        WebhookDeliveryStatus::Queued => STATE_PENDING,
        WebhookDeliveryStatus::Dropped => "dropped",
        WebhookDeliveryStatus::Disabled => "disabled",
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared_types::{
        WebhookApplicationAck, WebhookEventKind, WebhookProvider, WEBHOOK_EVENT_VERSION,
    };

    #[tokio::test]
    async fn pending_and_terminal_rows_survive_reopen() -> Result<(), String> {
        let path = temp_path("replay");
        let (outbox, first) = WebhookOutbox::initialize(Some(path.clone())).await?;
        assert!(first.requires_bootstrap);
        assert!(outbox.reserve(event("pending")).await?);
        assert!(outbox.reserve(event("delivered")).await?);
        outbox.complete(record("delivered")).await?;
        drop(outbox);

        let (_, replay) = WebhookOutbox::initialize(Some(path.clone())).await?;
        assert_eq!(replay.pending.len(), 1);
        assert_eq!(replay.pending[0].id, "pending");
        assert_eq!(replay.recent.len(), 1);
        assert_eq!(replay.delivered_total, 1);
        assert!(replay.event_ids.contains(&"delivered".to_owned()));
        cleanup(&path);
        Ok(())
    }

    #[tokio::test]
    async fn bootstrap_ids_are_durable_without_event_payloads() -> Result<(), String> {
        let path = temp_path("bootstrap");
        let (outbox, replay) = WebhookOutbox::initialize(Some(path.clone())).await?;
        assert!(replay.requires_bootstrap);
        outbox
            .bootstrap(vec!["execution-history-hedged".to_owned()])
            .await?;
        drop(outbox);

        let (outbox, replay) = WebhookOutbox::initialize(Some(path.clone())).await?;
        assert!(!replay.requires_bootstrap);
        assert!(replay.pending.is_empty());
        assert!(replay
            .event_ids
            .contains(&"execution-history-hedged".to_owned()));
        assert!(!outbox.reserve(event("execution-history-hedged")).await?);
        cleanup(&path);
        Ok(())
    }

    fn event(id: &str) -> WebhookEvent {
        WebhookEvent {
            id: id.to_owned(),
            version: WEBHOOK_EVENT_VERSION.to_owned(),
            kind: WebhookEventKind::Test,
            occurred_at_ms: 1,
            payload: serde_json::json!({"ok": true}),
        }
    }

    fn record(id: &str) -> WebhookDeliveryRecord {
        WebhookDeliveryRecord {
            event_id: id.to_owned(),
            kind: WebhookEventKind::Test,
            provider: WebhookProvider::Generic,
            status: WebhookDeliveryStatus::Delivered,
            attempts: 1,
            response_status: Some(204),
            application_ack: WebhookApplicationAck::TransportOnly,
            response_message: None,
            error: None,
            updated_at_ms: 2,
        }
    }

    fn temp_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "crossline-webhook-outbox-{label}-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ))
    }

    fn cleanup(path: &Path) {
        for candidate in [
            path.to_path_buf(),
            PathBuf::from(format!("{}-wal", path.display())),
            PathBuf::from(format!("{}-shm", path.display())),
        ] {
            let _ = std::fs::remove_file(candidate);
        }
    }
}
