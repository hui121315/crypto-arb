use super::{
    nav_schema_hash, NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY, NAV_SAMPLE_STATUS_OK,
    NAV_SCHEMA_MIGRATION_ID, NAV_SCHEMA_MIGRATION_PATH, NAV_SCHEMA_SQL, NAV_SCHEMA_VERSION,
};
use rusqlite::{params, Connection};
use std::path::{Path, PathBuf};
use tokio::task;

pub(super) struct NavLoadResult {
    pub(super) rows: Vec<(i64, f64)>,
    pub(super) sample_count: u64,
}

pub(super) async fn load_path(path: PathBuf, oldest_ms: i64) -> Result<NavLoadResult, String> {
    task::spawn_blocking(move || load_sync(&path, oldest_ms))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

pub(super) async fn append_path(
    path: PathBuf,
    occurred_at_ms: i64,
    nav_usd: f64,
) -> Result<u64, String> {
    task::spawn_blocking(move || append_sync(&path, occurred_at_ms, nav_usd))
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.to_string())
}

fn load_sync(path: &Path, oldest_ms: i64) -> rusqlite::Result<NavLoadResult> {
    let conn = open(path)?;
    prune(&conn, oldest_ms)?;
    let mut stmt = conn.prepare(
        "SELECT occurred_at_ms, nav_usd
         FROM portfolio_nav_samples
         WHERE occurred_at_ms >= ?1 AND status = ?2
         ORDER BY occurred_at_ms ASC",
    )?;
    let rows = stmt.query_map(params![oldest_ms, NAV_SAMPLE_STATUS_OK], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, f64>(1)?))
    })?;
    Ok(NavLoadResult {
        rows: rows.collect::<rusqlite::Result<Vec<_>>>()?,
        sample_count: sample_count(&conn)?,
    })
}

fn append_sync(path: &Path, occurred_at_ms: i64, nav_usd: f64) -> rusqlite::Result<u64> {
    let conn = open(path)?;
    conn.execute(
        "INSERT OR REPLACE INTO portfolio_nav_samples
            (occurred_at_ms, nav_usd, status, source, problem)
         VALUES (?1, ?2, ?3, ?4, NULL)",
        params![
            occurred_at_ms,
            nav_usd,
            NAV_SAMPLE_STATUS_OK,
            NAV_SAMPLE_SOURCE_ACCOUNT_EQUITY
        ],
    )?;
    sample_count(&conn)
}

pub(super) fn open(path: &Path) -> rusqlite::Result<Connection> {
    if let Some(parent) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(NAV_SCHEMA_SQL)?;
    migrate_schema(&conn)?;
    Ok(conn)
}

fn migrate_schema(conn: &Connection) -> rusqlite::Result<()> {
    ensure_sample_column(
        conn,
        "status",
        "ALTER TABLE portfolio_nav_samples ADD COLUMN status TEXT NOT NULL DEFAULT 'ok'",
    )?;
    ensure_sample_column(
        conn,
        "source",
        "ALTER TABLE portfolio_nav_samples ADD COLUMN source TEXT NOT NULL DEFAULT 'position_margin'",
    )?;
    ensure_sample_column(
        conn,
        "problem",
        "ALTER TABLE portfolio_nav_samples ADD COLUMN problem TEXT",
    )?;
    conn.pragma_update(None, "user_version", NAV_SCHEMA_VERSION)?;
    conn.execute(
        "INSERT OR REPLACE INTO portfolio_nav_meta (key, value) VALUES ('schema_version', ?1)",
        params![NAV_SCHEMA_VERSION.to_string()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO portfolio_nav_meta (key, value) VALUES ('schema_hash', ?1)",
        params![nav_schema_hash()],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO portfolio_nav_meta (key, value) VALUES ('migration_id', ?1)",
        params![NAV_SCHEMA_MIGRATION_ID],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO portfolio_nav_meta (key, value) VALUES ('migration_path', ?1)",
        params![NAV_SCHEMA_MIGRATION_PATH],
    )?;
    Ok(())
}

fn ensure_sample_column(conn: &Connection, name: &str, sql: &str) -> rusqlite::Result<()> {
    if sample_columns(conn)?.iter().any(|column| column == name) {
        return Ok(());
    }
    conn.execute_batch(sql)
}

pub(super) fn sample_columns(conn: &Connection) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare("PRAGMA table_info(portfolio_nav_samples)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    rows.collect()
}

fn sample_count(conn: &Connection) -> rusqlite::Result<u64> {
    let count = conn.query_row("SELECT COUNT(*) FROM portfolio_nav_samples", [], |row| {
        row.get::<_, i64>(0)
    })?;
    Ok(count.max(0) as u64)
}

fn prune(conn: &Connection, oldest_ms: i64) -> rusqlite::Result<()> {
    conn.execute(
        "DELETE FROM portfolio_nav_samples WHERE occurred_at_ms < ?1",
        params![oldest_ms],
    )?;
    Ok(())
}
